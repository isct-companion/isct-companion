//! The courses 過去問 › 探す lists: this quarter's enrolled courses, read
//! from the device. Each opens its subject on ScienceTokyoExam only when
//! tapped, so listing them asks the site nothing.
use super::{array, search::subject_codes, text, visible, ScheduleError, SchoolReads, Term};
use crate::locale::{self, ContentLanguage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamCourseList {
    pub courses: Vec<ExamCourse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamCourse {
    pub course_id: String,
    pub title: String,
    pub instructor: String,
    /// The codes its subject is looked up by; empty when it has none, and
    /// then it cannot be opened.
    pub subject_codes: Vec<String>,
}

/// Courses with a live membership in the current quarter that was not
/// withdrawn. Plan items are only planned, not taken, so they are left out.
pub(super) fn current(
    workspace: &Value,
    school: &SchoolReads,
) -> Result<ExamCourseList, ScheduleError> {
    let term = text(workspace, "currentTerm")
        .ok_or_else(|| ScheduleError::InvalidInput("Current term is unavailable.".into()))?;
    let term = Term::parse(term)?.to_string();
    let enrolled: HashSet<&str> = array(workspace, "memberships")
        .iter()
        .filter(|row| {
            visible(row)
                && text(row, "termId") == Some(term.as_str())
                && text(row, "status") != Some("withdrawn")
        })
        .filter_map(|row| text(row, "courseId"))
        .collect();
    let language = ContentLanguage::of(workspace);
    let courses = array(workspace, "courses")
        .iter()
        .filter(|course| visible(course))
        .filter_map(|course| Some((text(course, "id")?, course)))
        .filter(|(id, _)| enrolled.contains(id))
        .map(|(id, course)| ExamCourse {
            course_id: id.into(),
            title: course_label(course, "title", "/overrides/title", language),
            instructor: course_label(course, "instructor", "/overrides/instructor", language),
            subject_codes: subject_codes(workspace, school, course),
        })
        .collect();
    Ok(ExamCourseList { courses })
}

fn course_label(
    course: &Value,
    field: &str,
    override_path: &str,
    language: ContentLanguage,
) -> String {
    course
        .pointer(override_path)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            locale::pick(
                course.get("names").and_then(|names| names.get(field)),
                language,
            )
            .or_else(|| text(course, field))
            .unwrap_or_default()
            .to_owned()
        })
}
