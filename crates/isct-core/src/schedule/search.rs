//! One lookup policy for lecture, assignment and published-exam filters.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::{array, exam_views, text, visible, SchoolReads};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LectureSearch {
    pub course_ids: Vec<String>,
    pub assignment_ids: Vec<String>,
    pub exam_keys: Vec<String>,
}

fn normalized(value: &str) -> String {
    value.nfkc().flat_map(char::to_lowercase).collect()
}

pub(super) fn course_code_key(code: &str) -> String {
    code.nfkc().collect::<String>().trim().to_uppercase()
}

fn course_code_aliases(workspace: &Value, school: &SchoolReads, course: &Value) -> Vec<String> {
    let mut codes: Vec<_> = text(course, "code")
        .into_iter()
        .map(course_code_key)
        .collect();
    if let Some(offering) = text(course, "offeringId").and_then(|id| school.offering(id)) {
        codes.push(course_code_key(&offering.code));
    }
    if let Some(lms) = array(workspace, "lmsCourses")
        .iter()
        .find(|lms| lms["lmsId"].is_number() && lms["lmsId"] == course["lmsCourseId"])
    {
        if let Some(shortname) = text(lms, "shortname") {
            codes.push(crate::data::lms_course_code(shortname));
        }
    }
    // LMS sometimes prints two official codes in one shortname, with a
    // shared class suffix: ENT.B101(LAS.F101)-02. Both remain searchable.
    static COMPOUND: std::sync::OnceLock<Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    if let Ok(pattern) = COMPOUND.get_or_init(|| {
        regex::Regex::new(r"^([A-Z]{3}\.[A-Z]\d{3})\(([A-Z]{3}\.[A-Z]\d{3})\)(-\d+)?$")
    }) {
        for code in codes.clone() {
            if let Some(parts) = pattern.captures(&code) {
                let suffix = parts.get(3).map(|part| part.as_str()).unwrap_or_default();
                for index in [1, 2] {
                    if let Some(part) = parts.get(index) {
                        codes.push(format!("{}{suffix}", part.as_str()));
                    }
                }
            }
        }
    }
    codes.retain(|code| !code.is_empty());
    codes.sort();
    codes.dedup();
    codes
}

fn course_fields(workspace: &Value, school: &SchoolReads, course: &Value) -> Vec<Value> {
    vec![
        serde_json::json!(course_code_aliases(workspace, school, course)),
        course["title"].clone(),
        course["names"].clone(),
        course["instructor"].clone(),
    ]
}

fn compact(value: &str) -> String {
    normalized(value)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn strings(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::String(value) => into.push(value.clone()),
        Value::Array(values) => values.iter().for_each(|value| strings(value, into)),
        Value::Object(values) => values.values().for_each(|value| strings(value, into)),
        _ => {}
    }
}

fn matches(fields: &[Value], query: &str) -> bool {
    let mut values = Vec::new();
    fields.iter().for_each(|value| strings(value, &mut values));
    let normalized: Vec<_> = values.iter().map(|value| normalized(value)).collect();
    let compacted: Vec<_> = values.iter().map(|value| compact(value)).collect();
    query
        .split_whitespace()
        .map(compact)
        .filter(|part| !part.is_empty())
        .all(|part| {
            compacted.iter().any(|value| value.contains(&part))
            // A single misspelling is useful for names, but never changes a
            // course identity or an automatic exam association.
            || (part.len() >= 5 && part.chars().all(|c| c.is_ascii_alphabetic())
                && normalized.iter().flat_map(|value| value.split(|c: char| !c.is_alphanumeric()))
                    .any(|word| strsim::damerau_levenshtein(word, &part) <= 1))
        })
}

pub(super) fn lecture_search(
    workspace: &Value,
    school: &SchoolReads,
    term: Option<&str>,
    query: &str,
) -> LectureSearch {
    let courses: Vec<_> = match term {
        Some(term) => super::term_courses(workspace, term),
        None => array(workspace, "courses")
            .iter()
            .filter(|course| visible(course))
            .collect(),
    };
    let mut result = LectureSearch::default();
    for course in &courses {
        if matches(&course_fields(workspace, school, course), query) {
            if let Some(id) = text(course, "id") {
                result.course_ids.push(id.into());
            }
        }
    }
    for assignment in array(workspace, "assignments")
        .iter()
        .filter(|row| visible(row))
    {
        let Some(course) = courses
            .iter()
            .find(|course| text(course, "id") == text(assignment, "courseId"))
        else {
            continue;
        };
        let mut fields = course_fields(workspace, school, course);
        fields.push(assignment["title"].clone());
        if matches(&fields, query) {
            if let Some(id) = text(assignment, "id") {
                result.assignment_ids.push(id.into());
            }
        }
    }
    let terms = term
        .map(|term| [term.to_string()].into_iter().collect())
        .unwrap_or_else(|| super::exams::exam_terms(workspace));
    for exam in terms.iter().flat_map(|term| exam_views(workspace, term)) {
        let raw = array(workspace, "exams")
            .iter()
            .find(|row| text(row, "id") == Some(exam.id.as_str()));
        if matches(
            &[
                Value::String(exam.code),
                Value::String(exam.title),
                Value::String(exam.room),
                raw.map(|row| row["label"].clone()).unwrap_or(Value::Null),
            ],
            query,
        ) {
            result.exam_keys.push(exam.key);
        }
    }
    result
}
