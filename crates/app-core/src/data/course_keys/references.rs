//! Course references in stored records, including the course part of a weekly session key.
use super::super::*;

const SYNCED_TABLES: &[&str] = &[
    "manual_memberships",
    "plan_items",
    "intensive_schedules",
    "schedule_overrides",
    "course_overrides",
];
const CACHE_TABLES: &[&str] = &["memberships", "assignments", "lms_items", "exams"];

pub(super) fn map_synced(
    table: &str,
    row: &mut Value,
    mut map: impl FnMut(&str) -> Result<String, DataError>,
) -> Result<(), DataError> {
    if SYNCED_TABLES.contains(&table) {
        map_row(table, row, |reference| map(reference).map(Some))?;
    }
    Ok(())
}

fn map_row(
    table: &str,
    row: &mut Value,
    mut map: impl FnMut(&str) -> Result<Option<String>, DataError>,
) -> Result<bool, DataError> {
    let mut changed = map_field(row, "courseId", &mut map)?;
    if table == "schedule_overrides" {
        if let Some(base) = row.get_mut("base") {
            changed |= map_field(base, "courseId", &mut map)?;
        }
        if let Some((course, session)) = weekly_target(row) {
            if let Some(mapped) = map(course)?.filter(|mapped| mapped != course) {
                row["targetKey"] = format!("{mapped}-weekly-{session}").into();
                changed = true;
            }
        }
    }
    Ok(changed)
}

fn map_field(
    row: &mut Value,
    key: &str,
    map: &mut impl FnMut(&str) -> Result<Option<String>, DataError>,
) -> Result<bool, DataError> {
    let Some(reference) = row
        .get(key)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(false);
    };
    let Some(mapped) = map(reference)?.filter(|mapped| mapped != reference) else {
        return Ok(false);
    };
    row[key] = mapped.into();
    Ok(true)
}

fn weekly_target(row: &Value) -> Option<(&str, &str)> {
    if row["targetKind"] == "exam" {
        return None;
    }
    let (course, session) = row["targetKey"].as_str()?.rsplit_once("-weekly-")?;
    let (index, date) = session.split_once('-')?;
    (!course.is_empty()
        && index.parse::<usize>().is_ok()
        && crate::schedule::parse_civil_date(date).is_ok())
    .then_some((course, session))
}

/// Rewrite local references in the same transaction that moves the course's keys.
/// The records describe the same edits, so their revisions and clocks stay put.
pub(super) fn move_local(
    transaction: &Transaction<'_>,
    from: &str,
    into: &str,
) -> Result<(), DataError> {
    for table in SYNCED_TABLES.iter().chain(CACHE_TABLES).copied() {
        let rows = if table == "schedule_overrides" {
            let prefix = format!("{from}-weekly-");
            read_json_rows(
                transaction,
                table,
                "SELECT value FROM schedule_overrides
                 WHERE json_extract(value, '$.courseId')=?1
                    OR json_extract(value, '$.base.courseId')=?1
                    OR substr(json_extract(value, '$.targetKey'), 1, length(?2))=?2",
                params![from, prefix],
            )?
        } else {
            read_json_rows(
                transaction,
                table,
                &format!("SELECT value FROM {table} WHERE json_extract(value, '$.courseId')=?1"),
                [from],
            )?
        };
        for mut row in rows {
            if !map_row(table, &mut row, |reference| {
                Ok((reference == from).then(|| into.to_owned()))
            })? {
                continue;
            }
            if table == "schedule_overrides" {
                upsert_row(transaction, table, &row, true)?;
                continue;
            }
            let hash = content_hash(&content_value(&row))?;
            transaction.execute(
                &format!("UPDATE {table} SET value=?2, hash=?3 WHERE id=?1"),
                params![
                    string_field(&row, "id")?,
                    serde_json::to_string(&row)?,
                    hash
                ],
            )?;
        }
    }
    for table in ["plan_items", "course_overrides"] {
        transaction.execute(
            &format!("UPDATE {table} SET course_id=?2 WHERE course_id=?1"),
            params![from, into],
        )?;
    }
    Ok(())
}
