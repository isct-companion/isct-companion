//! The read side of the school records: what the official academic identity
//! fills into a fresh profile, and what the workspace's courses show of the
//! catalog. Courses themselves are merged when their sources are written
//! (`course_merge`).
use super::*;
use std::collections::BTreeMap;

use crate::schedule::{array, text_or_empty as text};

/// Fills the department and a fresh study profile from the 教務 academic
/// identity; choices the person made win.
pub(super) fn apply_academic_identity(
    connection: &Connection,
    workspace: &mut Value,
) -> Result<(), DataError> {
    if let Some(identity) = read_setting(connection, "kyomuAcademicIdentity")? {
        if text(&workspace["settings"], "departmentId").is_empty() {
            let departments: Vec<Value> =
                serde_json::from_str(include_str!("../course_query/departments.json"))?;
            if let Some(department) = departments.iter().find(|d| {
                d.pointer("/name/ja").and_then(Value::as_str) == identity["department"].as_str()
            }) {
                workspace["settings"]["departmentId"] = department["id"].clone();
            }
        }
        if let Ok(date) =
            chrono::NaiveDate::parse_from_str(text(&identity, "admittedOn"), "%Y-%m-%d")
        {
            let programme = text(&identity, "programme");
            let stage = if programme.contains("学士") {
                "undergraduate"
            } else if programme.contains("修士") {
                "master"
            } else if programme.contains("博士") {
                "doctoral"
            } else if programme.contains("専門職") {
                "professional"
            } else {
                "unknown"
            };
            // Existing user choices win; a fresh profile uses the official entry date.
            if workspace["academicProfile"]["admissionYear"].is_null() {
                workspace["academicProfile"]["admissionYear"] = date.year().into();
            }
            if array(&workspace["academicProfile"], "studyEnrollments").is_empty()
                && stage != "unknown"
            {
                let quarter = match date.month() {
                    4..=6 => 1,
                    7..=9 => 2,
                    10..=12 => 3,
                    _ => 4,
                };
                let year = date.year() - i32::from(date.month() < 4);
                let category = text(&identity, "entryCategory");
                let kind = if category.contains("編入") {
                    "transfer"
                } else if ["前期日程", "後期日程", "一般", "推薦", "総合型"]
                    .iter()
                    .any(|v| category.contains(v))
                {
                    "regular"
                } else {
                    "unknown"
                };
                let mut enrollment = json!({"stage":stage,"entryKind":kind,"admittedTerm":format!("{year}-{quarter}Q")});
                if kind == "regular" {
                    enrollment["entryGrade"] = json!(1);
                }
                workspace["academicProfile"]["studyEnrollments"] = json!([enrollment]);
            }
        }
    }
    Ok(())
}

/// What the workspace's courses show of the catalog offerings they name,
/// which the workspace does not carry.
pub(super) fn catalog_entries(
    connection: &Connection,
    workspace: &Value,
) -> Result<BTreeMap<String, crate::schedule::CatalogEntry>, DataError> {
    let catalog = super::course_merge::offerings_by_id(
        connection,
        array(workspace, "courses")
            .iter()
            .map(|course| text(course, "offeringId"))
            .filter(|id| !id.is_empty()),
    )?;
    let mut named = BTreeMap::new();
    for course in array(workspace, "courses") {
        let id = text(course, "offeringId");
        if let Some(offering) = catalog.get(id).filter(|_| !named.contains_key(id)) {
            named.insert(
                id.to_owned(),
                crate::schedule::CatalogEntry {
                    code: text(offering, "code").to_owned(),
                    format: serde_json::from_value(offering["format"].clone())?,
                },
            );
        }
    }
    Ok(named)
}
