use super::{array, text};
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
    pub offering_id: Option<String>,
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
    let code = text(course, "code")
        .unwrap_or_default()
        .trim()
        .to_uppercase();
    let mut exams: Vec<_> = array(workspace, "exams")
        .iter()
        .filter(|row| {
            visible(row)
                && row.get("missingSince").is_none_or(Value::is_null)
                && text(row, "status") == Some("scheduled")
                && text(row, "courseCode")
                    .unwrap_or_default()
                    .trim()
                    .to_uppercase()
                    == code
        })
        .collect();
    exams.sort_by_key(|row| text(row, "date").unwrap_or_default());
    let ids = |rows: Vec<&Value>| {
        rows.iter()
            .filter_map(|row| text(row, "id").map(str::to_owned))
            .collect()
    };
    CourseDetail {
        material_ids: ids(materials),
        sections,
        assignment_ids: ids(assignments),
        exam_ids: ids(exams),
        offering_id: array(workspace, "offerings")
            .iter()
            .find(|row| visible(row) && text(row, "id") == text(course, "offeringId"))
            .and_then(|row| text(row, "id"))
            .map(str::to_owned),
    }
}

// Course details retain cached rows until tombstoned; exam freshness is checked separately.
fn visible(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}
