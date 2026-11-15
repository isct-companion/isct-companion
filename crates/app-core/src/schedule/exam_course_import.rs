//! The local side of importing a timetable into ScienceTokyoExam.
use super::{content_language, localized, occurrence::timetable_courses, text, ScheduleError};
use super::{search::subject_code_candidates, SchoolReads, Term};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamCourseImportPreview {
    pub term: String,
    pub courses: Vec<ExamCourseImportCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamCourseImportCandidate {
    pub course_id: String,
    pub title: String,
    pub instructor: String,
    pub subject_codes: Vec<String>,
    /// Local syllabus offering, for matching against the remote subject's
    /// class list. It is not itself a verified remote class ID.
    pub offering_id: Option<String>,
}

pub(super) fn preview(
    workspace: &Value,
    school: &SchoolReads,
    term: Option<&str>,
) -> Result<ExamCourseImportPreview, ScheduleError> {
    let term = term
        .or_else(|| text(workspace, "currentTerm"))
        .ok_or_else(|| ScheduleError::InvalidInput("Current term is unavailable.".into()))?;
    let term = Term::parse(term)?.to_string();
    let language = content_language(workspace);
    let courses = timetable_courses(workspace, &term)
        .map(|course| {
            let course_id = text(course, "id").ok_or_else(|| {
                ScheduleError::InvalidInput("A timetable course has no ID.".into())
            })?;
            Ok(ExamCourseImportCandidate {
                course_id: course_id.into(),
                title: course_label(course, "title", "/overrides/title", language),
                instructor: course_label(course, "instructor", "/overrides/instructor", language),
                subject_codes: subject_code_candidates(workspace, school, course),
                offering_id: text(course, "offeringId").map(str::to_owned),
            })
        })
        .collect::<Result<_, ScheduleError>>()?;
    Ok(ExamCourseImportPreview { term, courses })
}

fn course_label(course: &Value, field: &str, override_path: &str, language: &str) -> String {
    course
        .pointer(override_path)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            localized(
                course.get("names").and_then(|names| names.get(field)),
                language,
                text(course, field).unwrap_or_default(),
            )
        })
}
