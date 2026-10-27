//! Course identity (`approach/course-internal-id`, `approach/course-id-merge`).
//!
//! Every course has one internal id, minted on this device and never
//! rewritten. Adapters know a course only by external keys: the 9-digit
//! offering id (the same number LMS keeps as `idnumber`, 教務 as jwc and the
//! syllabus as its course id), the LMS course id, a 教務 key. `course_keys`
//! maps each key to the internal id; ids that existed before this table
//! (`2026-202602431`, `2026-lms-42`) stay as they were.
//!
//! A write resolves the keys of the record it stores by exact match only.
//! When a record links keys that so far named two internal ids, the course
//! of the offering key absorbs the other in the same transaction: every key
//! and every local record that named the other id moves to it, and the other
//! id stays reachable as the key `internal:<id>`.
use super::*;
use std::collections::HashMap;

/// The key of a catalog offering, when `offering` is a 9-digit offering id.
pub(crate) fn offering_key(year: i32, offering: &str) -> Option<String> {
    ((1000..=9999).contains(&year)
        && offering.len() == 9
        && offering.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| format!("offering:{year}:{offering}"))
}

pub(crate) fn lms_key(lms_id: impl std::fmt::Display) -> String {
    format!("lms:{lms_id}")
}

pub(crate) fn kyomu_key(year: i32, external: &str) -> String {
    format!("kyomu:{year}:{external}")
}

/// The key a course made on a device keeps everywhere: its own id.
pub(crate) fn manual_key(id: &str) -> String {
    format!("manual:{id}")
}

/// An internal id another one absorbed.
fn internal_key(id: &str) -> String {
    format!("internal:{id}")
}

/// The internal id `key` names on this device.
pub(crate) fn lookup(connection: &Connection, key: &str) -> Result<Option<String>, DataError> {
    connection
        .query_row(
            "SELECT course_id FROM course_keys WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)
}

/// Every LMS course id with the internal id its key names.
pub(crate) fn lms_links(connection: &Connection) -> Result<HashMap<u64, String>, DataError> {
    let mut statement = connection
        .prepare_cached("SELECT key, course_id FROM course_keys WHERE key LIKE 'lms:%'")
        .map_err(DataError::from)?;
    let links = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(DataError::from)?
        .filter_map(|row| {
            row.map(|(key, id)| Some((key.strip_prefix("lms:")?.parse().ok()?, id)))
                .transpose()
        })
        .collect::<Result<_, _>>()
        .map_err(DataError::from)?;
    Ok(links)
}

/// The external keys of `course_id`, the most authoritative first: an
/// offering, then LMS, then 教務, then any other.
pub(crate) fn keys_of(connection: &Connection, course_id: &str) -> Result<Vec<String>, DataError> {
    let mut statement = connection
        .prepare_cached("SELECT key FROM course_keys WHERE course_id=?1")
        .map_err(DataError::from)?;
    let mut keys = statement
        .query_map([course_id], |row| row.get::<_, String>(0))
        .map_err(DataError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DataError::from)?;
    keys.sort_by_key(|key| (rank(key), key.clone()));
    Ok(keys)
}

fn rank(key: &str) -> u8 {
    match key.split_once(':').map(|(kind, _)| kind) {
        Some("offering") => 0,
        Some("lms") => 1,
        Some("kyomu") => 2,
        Some("manual") => 3,
        _ => 4,
    }
}

/// The internal id of the course `keys` name, given most authoritative
/// first. A key not seen yet is added for it; keys never move to another id
/// except when two ids turn out to be one course (see the module docs). No
/// key known mints a new id.
pub(crate) fn resolve(transaction: &Transaction<'_>, keys: &[String]) -> Result<String, DataError> {
    resolve_or(transaction, keys, None)
}

/// As `resolve`, but a course no key names yet takes `new_id` (a record's
/// own id, minted where it was made) instead of a new one.
pub(crate) fn resolve_or(
    transaction: &Transaction<'_>,
    keys: &[String],
    new_id: Option<&str>,
) -> Result<String, DataError> {
    if keys.is_empty() {
        return Err("a course needs at least one external key".into());
    }
    let mut found = Vec::new();
    for key in keys {
        found.push(lookup(transaction, key)?);
    }
    let Some(course_id) = found.iter().flatten().next().cloned() else {
        let course_id = new_id.map_or_else(|| Uuid::now_v7().to_string(), str::to_owned);
        for key in keys {
            insert(transaction, key, &course_id)?;
        }
        return Ok(course_id);
    };
    for (key, known) in keys.iter().zip(&found) {
        match known {
            None => insert(transaction, key, &course_id)?,
            Some(other) if *other != course_id => absorb(transaction, other, &course_id)?,
            Some(_) => {}
        }
    }
    Ok(course_id)
}

fn insert(transaction: &Transaction<'_>, key: &str, course_id: &str) -> Result<(), DataError> {
    transaction
        .execute(
            "INSERT INTO course_keys(key, course_id, updated_at) VALUES (?1, ?2, ?3)",
            params![key, course_id, now()],
        )
        .map_err(DataError::from)?;
    Ok(())
}

/// Local records that name a course, with the field that names it.
const COURSE_REFERENCES: &[(&str, &str)] = &[
    ("main.manual_memberships", "courseId"),
    ("main.plan_items", "courseId"),
    ("main.intensive_schedules", "courseId"),
    ("main.plans", "courseId"),
    ("main.schedule_overrides", "courseId"),
    ("main.course_overrides", "courseId"),
    ("cache.memberships", "courseId"),
    ("cache.assignments", "courseId"),
    ("cache.lms_items", "courseId"),
    ("cache.exams", "courseId"),
];

/// Moves everything that named `from` to `into`: its keys, and the local
/// records above. A schedule override's target key starts with the course
/// id. Records keep their revision: what they mean did not change, and
/// synced copies name the course by external key.
fn absorb(transaction: &Transaction<'_>, from: &str, into: &str) -> Result<(), DataError> {
    transaction
        .execute(
            "UPDATE course_keys SET course_id=?2, updated_at=?3 WHERE course_id=?1",
            params![from, into, now()],
        )
        .map_err(DataError::from)?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO course_keys(key, course_id, updated_at) VALUES (?1, ?2, ?3)",
            params![internal_key(from), into, now()],
        )
        .map_err(DataError::from)?;
    let prefix = format!("{from}-");
    for (table, field) in COURSE_REFERENCES {
        let rows: Vec<(String, String)> = {
            let mut statement = transaction
                .prepare(&format!(
                    "SELECT id, value FROM {table}
                     WHERE json_extract(value, '$.{field}')=?1
                        OR json_extract(value, '$.targetKey') LIKE ?2 || '%'"
                ))
                .map_err(DataError::from)?;
            let rows = statement
                .query_map(params![from, prefix], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(DataError::from)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(DataError::from)?;
            rows
        };
        for (id, raw) in rows {
            let mut value: Value = serde_json::from_str(&raw)
                .map_err(|_| DataError::storage(format!("{table} contains invalid JSON")))?;
            if value.get(*field).and_then(Value::as_str) == Some(from) {
                value[*field] = into.into();
            }
            if let Some(rest) = value
                .get("targetKey")
                .and_then(Value::as_str)
                .and_then(|key| key.strip_prefix(&prefix))
            {
                value["targetKey"] = format!("{into}-{rest}").into();
            }
            let hash = content_hash(&content_value(&value))?;
            transaction
                .execute(
                    &format!("UPDATE {table} SET value=?2, hash=?3 WHERE id=?1"),
                    params![id, serde_json::to_string(&value)?, hash],
                )
                .map_err(DataError::from)?;
        }
    }
    for table in ["main.plan_items", "main.course_overrides"] {
        transaction
            .execute(
                &format!("UPDATE {table} SET course_id=?2 WHERE course_id=?1"),
                params![from, into],
            )
            .map_err(DataError::from)?;
    }
    transaction
        .execute(
            "UPDATE main.schedule_overrides
             SET target_key=json_extract(value, '$.targetKey')
             WHERE json_extract(value, '$.targetKey') LIKE ?1 || '%'",
            [format!("{into}-")],
        )
        .map_err(DataError::from)?;
    log::info!(target: "app_core::data", "Merged two ids of one course");
    Ok(())
}

/// Fills `course_keys` from the identity map of user.db schema 1: the course
/// id each source and offering resolved to. An id stays the course's id;
/// when the old map gave one offering two ids (it rewrote ids, so it rarely
/// does), the canonical `{year}-{offering}` one keeps the key and the other
/// is absorbed into it.
pub(super) fn migrate_identity_map(transaction: &Transaction<'_>) -> Result<(), DataError> {
    let rows: Vec<(String, String, i32, String, Option<String>)> = {
        let mut statement = transaction.prepare(
            "SELECT provider, external_id, academic_year, course_id, offering_id
             FROM course_identity_map ORDER BY course_id NOT LIKE '%-lms-%', course_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    // Canonical ids first, so an offering key goes to `{year}-{offering}`.
    let mut ordered = rows;
    ordered.sort_by_key(|(_, _, year, course_id, offering)| {
        let canonical = offering
            .as_deref()
            .is_some_and(|offering| *course_id == format!("{year}-{offering}"));
        (!canonical, course_id.clone())
    });
    for (provider, external, year, course_id, offering) in ordered {
        let source = match provider.as_str() {
            "lms" => Some(lms_key(&external)),
            "kyomu" => Some(kyomu_key(year, &external)),
            _ => None,
        };
        let offering = offering.and_then(|offering| offering_key(year, &offering));
        for key in offering.iter().chain(&source) {
            match lookup(transaction, key)? {
                None => insert(transaction, key, &course_id)?,
                Some(known) if known != course_id => absorb(transaction, &course_id, &known)?,
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// The fields of a synced user record that name a course.
fn course_fields(table: &str) -> &'static [&'static str] {
    match table {
        "manual_memberships"
        | "plan_items"
        | "intensive_schedules"
        | "plans"
        | "schedule_overrides"
        | "course_overrides" => &["courseId"],
        _ => &[],
    }
}

/// The key a synced copy names `course_id` by: its most authoritative
/// external key, or the id itself when it has none (a course made on a
/// device is known by its own id).
fn external_of(connection: &Connection, course_id: &str) -> Result<String, DataError> {
    Ok(keys_of(connection, course_id)?
        .into_iter()
        .find(|key| !key.starts_with("internal:"))
        .unwrap_or_else(|| course_id.to_owned()))
}

/// The internal id a synced reference names on this device. A key not seen
/// yet gets a course of its own that school data joins once it names the same
/// key; an id from before course keys reads as the key it was made from
/// (`2026-202602431` the offering, `2026-lms-42` the LMS course); anything
/// else names a course made on a device by its own id.
fn internal_of(transaction: &Transaction<'_>, reference: &str) -> Result<String, DataError> {
    let (kind, rest) = reference.split_once(':').unwrap_or_default();
    if matches!(kind, "offering" | "lms" | "kyomu" | "catalog") && !rest.is_empty() {
        return resolve(transaction, &[reference.to_owned()]);
    }
    if matches!(kind, "manual" | "internal") && !rest.is_empty() {
        return resolve_or(transaction, &[reference.to_owned()], Some(rest));
    }
    if let Some((year, tail)) = reference.split_once('-') {
        if let Ok(year) = year.parse::<i32>() {
            if let Some(key) = offering_key(year, tail) {
                return resolve(transaction, &[key]);
            }
            if let Some(lms_id) = tail
                .strip_prefix("lms-")
                .filter(|id| id.parse::<u64>().is_ok())
            {
                return resolve(transaction, &[lms_key(lms_id)]);
            }
        }
    }
    resolve_or(transaction, &[manual_key(reference)], Some(reference))
}

/// `row` as a synced copy carries it: course references by external key
/// (`approach/synced-records-use-course-keys`).
pub(crate) fn to_synced(
    connection: &Connection,
    table: &str,
    row: &mut Value,
) -> Result<(), DataError> {
    for field in course_fields(table) {
        let Some(id) = row.get(*field).and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let external = external_of(connection, &id)?;
        replace_reference(row, field, &id, &external);
    }
    Ok(())
}

/// A synced copy of `row` as this device stores it: course references by
/// internal id. A course made on another device also gets its own key.
pub(crate) fn from_synced(
    transaction: &Transaction<'_>,
    table: &str,
    row: &mut Value,
) -> Result<(), DataError> {
    if table == "manual_courses" {
        if let Some(id) = row.get("id").and_then(Value::as_str) {
            resolve_or(transaction, &[manual_key(id)], Some(id))?;
        }
    }
    for field in course_fields(table) {
        let Some(reference) = row.get(*field).and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let internal = internal_of(transaction, &reference)?;
        replace_reference(row, field, &reference, &internal);
    }
    Ok(())
}

/// Writes `to` into `field`, and into a schedule override's target key,
/// which starts with the course it marks.
fn replace_reference(row: &mut Value, field: &str, from: &str, to: &str) {
    if from == to {
        return;
    }
    row[field] = to.into();
    let prefix = format!("{from}-");
    if let Some(rest) = row
        .get("targetKey")
        .and_then(Value::as_str)
        .and_then(|key| key.strip_prefix(&prefix))
    {
        row["targetKey"] = format!("{to}-{rest}").into();
    }
}

/// Every course made on this device is known by its own id, and by its
/// offering when no other course holds that key.
pub(super) fn key_manual_courses(transaction: &Transaction<'_>) -> Result<(), DataError> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO course_keys(key, course_id, updated_at)
             SELECT 'manual:' || id, id, ?1 FROM manual_courses",
            [now()],
        )
        .map_err(DataError::from)?;
    let courses: Vec<(String, Value)> = read_rows(transaction, "manual_courses")?
        .into_iter()
        .filter_map(|row| Some((row.get("id")?.as_str()?.to_owned(), row)))
        .collect();
    for (id, row) in courses {
        let year = row
            .get("termId")
            .and_then(Value::as_str)
            .and_then(|term| term.get(..4))
            .and_then(|year| year.parse().ok());
        let key = year
            .zip(row.get("offeringId").and_then(Value::as_str))
            .and_then(|(year, offering)| offering_key(year, offering));
        if let Some(key) = key {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO course_keys(key, course_id, updated_at)
                     VALUES (?1, ?2, ?3)",
                    params![key, id, now()],
                )
                .map_err(DataError::from)?;
        }
    }
    Ok(())
}
