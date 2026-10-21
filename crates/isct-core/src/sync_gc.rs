//! Drops cached school content nobody has used for a long time, so the
//! database does not only grow.
//!
//! TTLs (`sync_state::ttl`) say when a key is read again; gc says when its
//! content is deleted. Whatever gc deletes, it invalidates the key in the
//! same transaction, so the key reads as missing and the next open reads it
//! in full instead of renewing an empty copy by validator or freeze.
use crate::data::{versioned::content_hash, DataError};
use crate::sync_state::{self, Dataset, Key, KeyState, Term};
use chrono::{DateTime, Datelike, Duration, Utc};
use rusqlite::{params, Connection};
use serde_json::Value;

/// Module details of a past course nobody opened for this long.
const COURSE_DETAILS_UNUSED_DAYS: i64 = 180;
/// A シラバス detail page nobody opened for this long.
const CATALOG_DETAIL_UNUSED_DAYS: i64 = 365;
const RUNS_KEPT_DAYS: i64 = 30;
/// gc leaves its own `sync_runs` row under this dataset name, so it runs at
/// most once a day however often the app starts.
const GC_RUN: &str = "gc";

/// What one call removed. `ran` is false when gc already ran within a day.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GcReport {
    pub ran: bool,
    /// `lms.course` keys whose module details were stripped.
    pub courses_stripped: usize,
    /// `catalog.detail` keys whose content was deleted.
    pub details_deleted: usize,
    pub runs_deleted: usize,
}

/// One gc pass inside the caller's transaction (`DataStore::write`).
pub fn gc(connection: &Connection, now: DateTime<Utc>) -> Result<GcReport, DataError> {
    let last: Option<String> = connection
        .query_row(
            "SELECT MAX(finished_at) FROM sync_runs WHERE dataset=?1",
            [GC_RUN],
            |row| row.get(0),
        )
        .map_err(DataError::from)?;
    let ran_recently = last
        .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
        .is_some_and(|at| now < at.with_timezone(&Utc) + Duration::days(1));
    if ran_recently {
        return Ok(GcReport::default());
    }
    let mut report = GcReport {
        ran: true,
        ..GcReport::default()
    };
    for state in sync_state::list(connection, Some(Dataset::LmsCourse))? {
        if !unused(&state, now, COURSE_DETAILS_UNUSED_DAYS) {
            continue;
        }
        let Ok(lms_id) = state.key.parse::<i64>() else {
            continue;
        };
        let Some(course) = course_row(connection, lms_id)? else {
            continue;
        };
        // Structure and assignment records stay; only details go.
        if course_term(&course, now) == Term::Current {
            continue;
        }
        strip_course(connection, lms_id, now)?;
        sync_state::invalidate(connection, &Key::new(Dataset::LmsCourse, state.key))?;
        report.courses_stripped += 1;
    }
    for state in sync_state::list(connection, Some(Dataset::CatalogDetail))? {
        if !unused(&state, now, CATALOG_DETAIL_UNUSED_DAYS) {
            continue;
        }
        // The catalog row itself stays searchable; only the detail text goes.
        connection
            .execute(
                "DELETE FROM offering_content WHERE offering_id=?1",
                [&state.key],
            )
            .map_err(DataError::from)?;
        sync_state::invalidate(connection, &Key::new(Dataset::CatalogDetail, state.key))?;
        report.details_deleted += 1;
    }
    report.runs_deleted = connection
        .execute(
            "DELETE FROM sync_runs WHERE finished_at < ?1",
            [(now - Duration::days(RUNS_KEPT_DAYS)).to_rfc3339()],
        )
        .map_err(DataError::from)?;
    let at = now.to_rfc3339();
    connection
        .execute(
            "INSERT INTO sync_runs(run,dataset,key,account,started_at,finished_at,outcome,changed,source)
             VALUES(?1,?2,'all','',?3,?3,'complete',?4,'sync_gc')",
            params![
                uuid::Uuid::now_v7().simple().to_string(),
                GC_RUN,
                at,
                (report.courses_stripped + report.details_deleted + report.runs_deleted) as i64,
            ],
        )
        .map_err(DataError::from)?;
    Ok(report)
}

/// Content is stored and was neither opened nor read for `days`. A key never
/// opened counts from its last read.
fn unused(state: &KeyState, now: DateTime<Utc>, days: i64) -> bool {
    state.fetched_at.is_some()
        && state
            .last_used
            .or(state.fetched_at)
            .is_some_and(|at| at < now - Duration::days(days))
}

fn course_row(connection: &Connection, lms_id: i64) -> Result<Option<Value>, DataError> {
    use rusqlite::OptionalExtension;
    let raw: Option<String> = connection
        .query_row(
            "SELECT value FROM lms_courses WHERE lms_id=?1",
            [lms_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?;
    Ok(raw.and_then(|raw| serde_json::from_str(&raw).ok()))
}

/// The term rule of `lms.course` keys from the course's academic year and
/// quarters. Quarter bounds by month are approximate; gc only acts on content
/// unused for half a year, which a course of the running term never is.
fn course_term(course: &Value, now: DateTime<Utc>) -> Term {
    let today = (now + Duration::hours(9)).date_naive();
    let academic_year = i64::from(today.year() - i32::from(today.month() < 4));
    let Some(year) = course.get("year").and_then(Value::as_i64) else {
        return Term::Current;
    };
    if year < academic_year {
        return Term::Frozen;
    }
    if year > academic_year {
        return Term::Current;
    }
    let current_quarter = match today.month() {
        4 | 5 => 1,
        6..=9 => 2,
        10 | 11 => 3,
        _ => 4,
    };
    let last_quarter = course
        .get("quarters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|quarter| quarter.as_str()?.strip_suffix('Q')?.parse::<u32>().ok())
        .max();
    match last_quarter {
        Some(quarter) if quarter < current_quarter => Term::Past,
        _ => Term::Current,
    }
}

/// Removes page bodies, resource links and folder listings of one course's
/// modules, and the bodies of its sections. Titles, structure and
/// assignments stay.
fn strip_course(connection: &Connection, lms_id: i64, now: DateTime<Utc>) -> Result<(), DataError> {
    for (id, mut value) in rows(
        connection,
        "SELECT id,value FROM lms_items WHERE lms_course_id=?1",
        lms_id,
    )? {
        let mut stripped = false;
        for field in ["bodyHtml", "resourceUrl", "folderFiles"] {
            if let Some(slot) = value.get_mut(field).filter(|slot| !slot.is_null()) {
                *slot = Value::Null;
                stripped = true;
            }
        }
        if stripped {
            rewrite(connection, "lms_items", &id, value, now)?;
        }
    }
    for (id, mut value) in rows(
        connection,
        "SELECT id,value FROM lms_courses WHERE lms_id=?1",
        lms_id,
    )? {
        let mut stripped = false;
        for section in value
            .get_mut("sections")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            stripped |= section
                .as_object_mut()
                .and_then(|section| section.remove("bodyHtml"))
                .is_some();
        }
        if stripped {
            rewrite(connection, "lms_courses", &id, value, now)?;
        }
    }
    Ok(())
}

fn rows(
    connection: &Connection,
    sql: &str,
    lms_id: i64,
) -> Result<Vec<(String, Value)>, DataError> {
    let mut statement = connection.prepare(sql).map_err(DataError::from)?;
    let rows = statement
        .query_map([lms_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(DataError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DataError::from)?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, raw)| Some((id, serde_json::from_str(&raw).ok()?)))
        .collect())
}

/// Stores a stripped row with a new revision and the hash of its new
/// content, so the next complete read writes the details back instead of
/// matching the hash of the content gc removed.
fn rewrite(
    connection: &Connection,
    table: &str,
    id: &str,
    mut value: Value,
    now: DateTime<Utc>,
) -> Result<(), DataError> {
    let timestamp = now.to_rfc3339();
    let revision = value.get("rev").and_then(Value::as_u64).unwrap_or(0) + 1;
    if let Some(object) = value.as_object_mut() {
        object.insert("rev".into(), revision.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
    }
    let mut content = value.clone();
    if let Some(object) = content.as_object_mut() {
        for key in [
            "rev",
            "updatedAt",
            "updatedBy",
            "source",
            "lastSeenAt",
            "missingSince",
            "deletedAt",
        ] {
            object.remove(key);
        }
    }
    connection
        .execute(
            &format!("UPDATE {table} SET value=?2,hash=?3,updated_at=?4,rev=?5 WHERE id=?1"),
            params![
                id,
                serde_json::to_string(&value).map_err(DataError::from)?,
                content_hash(&content)?,
                timestamp,
                revision as i64,
            ],
        )
        .map_err(DataError::from)?;
    Ok(())
}

#[cfg(test)]
#[path = "tests/sync_gc.rs"]
mod tests;
