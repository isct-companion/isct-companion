use super::{array, text, ExamView};
use crate::course_materials::CourseKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

/// IDs project the authoritative snapshot without copying cached school records.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseDetail {
    pub material_ids: Vec<String>,
    pub sections: Vec<i32>,
    pub assignment_ids: Vec<String>,
    pub exam_ids: Vec<String>,
    pub exams: Vec<ExamView>,
    pub offering_id: Option<String>,
    /// The course's T2SCHOLA page, from the course itself or its LMS record.
    pub lms_url: Option<String>,
    pub syllabus_url: Option<String>,
    /// Where the course's downloaded files live, for opening its materials.
    pub files: Option<CourseKey>,
}

const LMS_HOST: &str = "lms.s.isct.ac.jp";
const SYLLABUS: &str = "https://syllabus.s.isct.ac.jp/";

fn lms_url(workspace: &Value, course: &Value) -> Option<String> {
    let provider = course.pointer("/source/provider").and_then(Value::as_str);
    if let Some(url) =
        text(course, "url").filter(|url| provider == Some("lms") && url.contains(LMS_HOST))
    {
        return Some(url.into());
    }
    let offering = text(course, "offeringId");
    let external = course.pointer("/source/externalId").and_then(Value::as_str);
    array(workspace, "lmsCourses")
        .iter()
        .find(|row| {
            (offering.is_some() && text(row, "offeringId") == offering)
                || (provider == Some("lms")
                    && row
                        .get("lmsId")
                        .and_then(Value::as_i64)
                        .is_some_and(|id| Some(id.to_string().as_str()) == external))
        })
        .and_then(|row| text(row, "url"))
        .map(str::to_owned)
}

fn syllabus_url(workspace: &Value, course: &Value, materials: &[&Value]) -> Option<String> {
    let offering = text(course, "offeringId");
    array(workspace, "offerings")
        .iter()
        .find(|row| offering.is_some() && text(row, "id") == offering)
        .and_then(|row| text(row, "syllabusUrl"))
        .filter(|url| !url.is_empty())
        .or_else(|| text(course, "url").filter(|url| url.starts_with(SYLLABUS)))
        .map(str::to_owned)
        .or_else(|| offering.map(|id| format!("{SYLLABUS}courses/-/-/-/{id}")))
        .or_else(|| {
            materials
                .iter()
                .filter_map(|row| text(row, "resourceUrl"))
                .find(|url| url.starts_with(SYLLABUS))
                .map(str::to_owned)
        })
}

pub fn course_detail(workspace: &Value, course_id: &str) -> CourseDetail {
    let Some(course) = array(workspace, "courses")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(course_id))
    else {
        return CourseDetail::default();
    };
    let mut materials: Vec<_> = array(workspace, "materials")
        .iter()
        .filter(|row| {
            visible(row)
                && text(row, "courseId") == Some(course_id)
                && row.get("visible").and_then(Value::as_bool) != Some(false)
                && !matches!(text(row, "module"), Some("assign" | "quiz"))
        })
        .collect();
    let syllabus_url = syllabus_url(workspace, course, &materials);
    let lms_url = lms_url(workspace, course);
    let section = |row: &&Value| {
        row.get("sectionNumber")
            .and_then(Value::as_i64)
            .or_else(|| row.get("session").and_then(Value::as_i64))
            .unwrap_or_default() as i32
    };
    materials.sort_by_key(section);
    let mut sections: Vec<_> = materials.iter().map(section).collect();
    sections.dedup();
    let mut assignments: Vec<_> = array(workspace, "assignments")
        .iter()
        .filter(|row| {
            visible(row)
                && text(row, "courseId") == Some(course_id)
                && row.get("hidden").and_then(Value::as_bool) != Some(true)
        })
        .collect();
    assignments.sort_by_key(|row| text(row, "dueAt").unwrap_or("~"));
    let exams: Vec<_> = super::course_terms(workspace, course)
        .iter()
        .flat_map(|term| super::exam_views(workspace, term))
        .filter(|exam| exam.course_id.as_deref() == Some(course_id))
        .collect();
    let ids = |rows: Vec<&Value>| {
        rows.iter()
            .filter_map(|row| text(row, "id").map(str::to_owned))
            .collect()
    };
    CourseDetail {
        material_ids: ids(materials),
        sections,
        assignment_ids: ids(assignments),
        exam_ids: exams
            .iter()
            .filter(|exam| exam.kind == "exam" && !exam.hidden && exam.status == "scheduled")
            .map(|exam| exam.id.clone())
            .collect(),
        exams,
        offering_id: array(workspace, "offerings")
            .iter()
            .find(|row| visible(row) && text(row, "id") == text(course, "offeringId"))
            .and_then(|row| text(row, "id"))
            .map(str::to_owned),
        lms_url,
        syllabus_url,
        files: text(course, "termId").map(|term| CourseKey {
            term: crate::storage::folder_name(term),
            course: crate::storage::folder_name(
                text(course, "code")
                    .filter(|code| !code.is_empty())
                    .or_else(|| text(course, "id"))
                    .unwrap_or_default(),
            ),
        }),
    }
}

// Course details retain cached rows until tombstoned; exam freshness is checked separately.
fn visible(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}
