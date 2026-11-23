//! The user's visibility choices for individual LMS timetable placements.
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn membership_id(lms_id: u64, term: &str) -> String {
    format!("lms-membership:{lms_id}:{term}")
}

pub(super) fn timetable_terms(lms: &Value, year: i64) -> Vec<String> {
    if !(2000..2100).contains(&year) {
        return Vec::new();
    }
    let prefix = format!("{year}-");
    super::lms_terms(lms)
        .into_iter()
        .filter(|term| term.starts_with(&prefix))
        .collect()
}

/// A hiding preference survives a term disappearing from LMS. It can only
/// be changed by the timetable control while that term exists in LMS.
pub(crate) fn current_lms_placements(workspace: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    for lms in crate::schedule::array(workspace, "lmsCourses")
        .iter()
        .filter(|row| crate::schedule::visible(row) && row["hidden"] != true)
    {
        let (Some(id), Some(year)) = (lms["lmsId"].as_u64(), lms["year"].as_i64()) else {
            continue;
        };
        ids.extend(
            timetable_terms(lms, year)
                .iter()
                .map(|term| membership_id(id, term)),
        );
    }
    ids
}

pub(crate) fn is_lms_timetable_membership(row: &Value) -> bool {
    row["evidence"]["provider"] == "lms"
        && row["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("lms-membership:"))
}

/// Older versions restored LMS visibility by writing a local declaration
/// for the same course and quarter after the explicit hide. The existing
/// declaration keeps that choice until the user hides the slot again.
pub(crate) fn restores_lms_timetable(declaration: &Value, hide: &Value) -> bool {
    is_lms_timetable_membership(hide)
        && hide["deletedAt"].is_string()
        && declaration["deletedAt"].is_null()
        && declaration["evidence"]["provider"] == "manual"
        && declaration["status"] == "declared"
        && declaration["courseId"] == hide["courseId"]
        && declaration["termId"] == hide["termId"]
        && declaration["updatedAt"]
            .as_str()
            .zip(hide["updatedAt"].as_str())
            .is_some_and(|(declared, hidden)| declared >= hidden)
}

pub(super) fn hides_lms_timetable(row: &Value, choices: &[Value]) -> bool {
    is_lms_timetable_membership(row)
        && row["deletedAt"].is_string()
        && !choices
            .iter()
            .any(|choice| restores_lms_timetable(choice, row))
}

/// Local restoration declarations belong to their LMS placement. When LMS
/// removes the term, the declaration alone cannot put a course on it.
pub(super) fn visible_choices(choices: &[Value]) -> impl Iterator<Item = &Value> {
    let hides: Vec<_> = choices
        .iter()
        .filter(|row| is_lms_timetable_membership(row))
        .collect();
    choices.iter().filter(move |row| {
        if is_lms_timetable_membership(row) {
            hides_lms_timetable(row, choices)
        } else {
            !hides.iter().any(|hide| restores_lms_timetable(row, hide))
        }
    })
}
