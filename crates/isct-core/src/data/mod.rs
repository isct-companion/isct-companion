pub mod cloud_merge;
mod error;
pub mod migrations;
pub mod versioned;

pub use error::DataError;

use chrono::{DateTime, Datelike, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use versioned::{content_hash, VersionedValue};

const COLLECTIONS: &[(&str, &str)] = &[
    ("courses", "timetable_courses"),
    ("memberships", "memberships"),
    ("assignments", "assignments"),
    ("materials", "lms_items"),
    ("events", "personal_events"),
    ("planItems", "plan_items"),
    ("exceptions", "calendar_exceptions"),
    ("offerings", "offerings"),
    ("intensiveSchedules", "intensive_schedules"),
    ("plans", "plans"),
    ("academicCalendar", "academic_calendar"),
    ("gpaSummary", "gpa_summary"),
    ("departments", "departments"),
    ("recommendedCourses", "recommended_courses"),
    ("importantDates", "important_dates"),
    ("exams", "exams"),
    ("scheduleOverrides", "schedule_overrides"),
    ("subscriptions", "subscriptions"),
    ("scheduledTasks", "scheduled_tasks"),
];

const SCHOOL_CACHE_TABLES: &[&str] = &[
    "timetable_courses",
    "memberships",
    "assignments",
    "offerings",
    "lms_courses",
    "lms_items",
    "registrations",
    "registration_states",
    "grades",
    "requirement_progress",
    "gpa_summary",
    "gpa_by_term",
    "academic_calendar",
    "departments",
    "recommended_courses",
    "important_dates",
    "exams",
];

/// Local backup payload. School/reference caches are intentionally absent:
/// they can be fetched again and must never become user-owned by export.
pub(crate) const USER_BACKUP_TABLES: &[&str] = &[
    "manual_courses",
    "manual_memberships",
    "course_overrides",
    "plan_items",
    "personal_events",
    "calendar_exceptions",
    "assignment_marks",
    "intensive_schedules",
    "plans",
    "schedule_overrides",
    "subscriptions",
];

pub(crate) const USER_BACKUP_SETTINGS: &[&str] = &["userPreferences", "academicProfile", "terms"];
const BACKUP_KIND: &str = "IsctCompanion.local-backup";
const BACKUP_SCHEMA_VERSION: u64 = 6;

fn merge_object(target: &mut Value, source: Value) {
    let (Some(target), Value::Object(source)) = (target.as_object_mut(), source) else {
        return;
    };
    for (key, value) in source {
        if value.is_null() {
            continue;
        }
        if value.is_object() && target.get(&key).is_some_and(Value::is_object) {
            if let Some(target_value) = target.get_mut(&key) {
                merge_object(target_value, value);
            }
        } else {
            target.insert(key, value);
        }
    }
}

/// Optional contract fields are omitted on the wire. The two model fields
/// whose absence has domain meaning remain explicitly nullable.
fn normalize_optional_nulls(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, child| {
                if child.is_null() {
                    return matches!(key.as_str(), "dueAt" | "periods");
                }
                normalize_optional_nulls(child);
                true
            });
        }
        Value::Array(values) => values.iter_mut().for_each(normalize_optional_nulls),
        _ => {}
    }
}

fn settings_parts(settings: &Value) -> (Value, Value, Value) {
    let user = json!({
        "departmentId": settings.get("departmentId"),
        "notifications": {
            "classes": settings.pointer("/notifications/classes"),
            "assignments": settings.pointer("/notifications/assignments"),
            "classLeadMinutes": settings.pointer("/notifications/classLeadMinutes")
        },
        "customization": settings.get("customization")
    });
    let device = json!({
        "lmsBilingualFilter": settings.get("lmsBilingualFilter"),
        "autoDownloadLimitMb": settings.get("autoDownloadLimitMb"),
        "autoCleanDays": settings.get("autoCleanDays"),
        "agentAccess": settings.get("agentAccess"),
        "appearance": settings.get("appearance"),
        "density": settings.get("density"),
        "textSize": settings.get("textSize"),
        "restoreSchoolPage": settings.get("restoreSchoolPage"),
        "activeSchoolSite": settings.get("activeSchoolSite"),
        "schoolServiceWidth": settings.get("schoolServiceWidth"),
        "schoolWebZoom": settings.get("schoolWebZoom"),
        "showGrades": settings.get("showGrades"),
        "siteConsents": settings.get("siteConsents"),
        "locale": settings.get("locale"),
        "contentLanguage": settings.get("contentLanguage"),
        "subscriptionRefresh": settings.get("subscriptionRefresh"),
        "notifications": {"localPermission": settings.pointer("/notifications/localPermission")}
    });
    let cloud = settings.get("cloud").cloned().unwrap_or_else(|| json!({}));
    (user, device, cloud)
}

/// Enumerated preference values. Settings forms render their own option lists;
/// Core is the one owner of which values may be stored.
const PREFERENCE_CHOICES: &[(&str, &[&str])] = &[
    ("/appearance", &["system", "light", "dark"]),
    ("/density", &["comfortable", "compact"]),
    ("/textSize", &["small", "medium", "large"]),
    ("/locale", &["en", "ja", "zh"]),
    ("/contentLanguage", &["auto", "ja", "en"]),
    ("/notifications/localPermission", &["default", "denied", "granted"]),
    ("/subscriptionRefresh/frequency", &["daily"]),
];
const CLOUD_CHOICES: &[(&str, &[&str])] = &[
    ("/provider", &["none", "google-drive", "onedrive", "icloud"]),
    ("/conflict", &["keep-current", "use-latest", "resolve-manually"]),
];
/// Study-guide cohorts are four-digit years from the 2000 curriculum onwards.
pub(crate) const ADMISSION_YEARS: std::ops::RangeInclusive<i64> = 2000..=9999;

fn validate_choices(value: &Value, choices: &[(&str, &[&str])]) -> Result<(), DataError> {
    for (pointer, allowed) in choices {
        let Some(stored) = value.pointer(pointer).filter(|value| !value.is_null()) else {
            continue;
        };
        if !stored.as_str().is_some_and(|stored| allowed.contains(&stored)) {
            return Err(DataError::InvalidInput(format!(
                "setting {} must be one of {}",
                &pointer[1..],
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

/// Checks the values present in whole settings or in one stored part of them.
fn validate_preference_values(settings: &Value) -> Result<(), DataError> {
    validate_choices(settings, PREFERENCE_CHOICES)?;
    settings
        .get("cloud")
        .map_or(Ok(()), |cloud| validate_choices(cloud, CLOUD_CHOICES))
}

fn validate_settings(settings: &Value) -> Result<(), DataError> {
    validate_contract::<crate::contracts::Settings>("settings", settings)?;
    validate_preference_values(settings)
}

pub(crate) fn validate_admission_year(profile: &Value) -> Result<(), DataError> {
    match profile.get("admissionYear").filter(|value| !value.is_null()) {
        None => Ok(()),
        Some(year) if year.as_i64().is_some_and(|year| ADMISSION_YEARS.contains(&year)) => Ok(()),
        Some(_) => Err(DataError::InvalidInput(format!(
            "admission year must be between {} and {}",
            ADMISSION_YEARS.start(),
            ADMISSION_YEARS.end()
        ))),
    }
}

/// Every writer of a settings row meets the same value rules.
fn validate_setting(key: &str, value: &Value) -> Result<(), DataError> {
    match key {
        "userPreferences" | "devicePreferences" => validate_preference_values(value),
        "cloud" => validate_choices(value, CLOUD_CHOICES),
        "academicProfile" => validate_admission_year(value),
        _ => Ok(()),
    }
}

/// Stored preference parts are complete snapshots of their fields: a null
/// field was cleared and removes any default or legacy value beneath it.
fn apply_preferences(target: &mut Value, part: Value) {
    let (Some(target), Value::Object(part)) = (target.as_object_mut(), part) else {
        return;
    };
    for (key, value) in part {
        if value.is_null() {
            target.remove(&key);
        } else if value.is_object() && target.get(&key).is_some_and(Value::is_object) {
            if let Some(child) = target.get_mut(&key) {
                apply_preferences(child, value);
            }
        } else {
            target.insert(key, value);
        }
    }
}

#[derive(Clone)]
pub struct DataStore {
    path: PathBuf,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn quarter_for_month(month: u32) -> &'static str {
    match month {
        12 | 1..=3 => "4Q",
        10 | 11 => "3Q",
        6..=9 => "2Q",
        _ => "1Q",
    }
}

/// One authoritative resolver for every native consumer. User-configured
/// starts intentionally override subscribed calendar boundaries. When no
/// usable boundary exists, keep the user's selected term instead of guessing
/// from hard-coded month ranges.
fn current_term_for(workspace: &Value, date: chrono::NaiveDate) -> Option<String> {
    let mut starts: Vec<(chrono::NaiveDate, String, u8)> = Vec::new();
    let mut configured = HashSet::new();
    if let Some(years) = workspace
        .pointer("/academicProfile/years")
        .and_then(Value::as_array)
    {
        for year in years {
            let Some(academic_year) = year.get("academicYear").and_then(Value::as_i64) else {
                continue;
            };
            for quarter in ["1Q", "2Q", "3Q", "4Q"] {
                if let Some(start) = year
                    .pointer(&format!("/quarterStarts/{quarter}"))
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                {
                    configured.insert((academic_year, quarter));
                    starts.push((start, format!("{academic_year}-{quarter}"), 2));
                }
            }
        }
    }
    if let Some(days) = workspace.get("academicCalendar").and_then(Value::as_array) {
        for day in days {
            if day.get("deletedAt").is_some() || day.get("missingSince").is_some() {
                continue;
            }
            if day.get("type").and_then(Value::as_str) != Some("quarter-start") {
                continue;
            }
            if let (Some(start), Some(year), Some(quarter)) = (
                day.get("date")
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()),
                day.get("academicYear").and_then(Value::as_i64),
                day.get("quarter").and_then(Value::as_str),
            ) {
                if configured.contains(&(year, quarter)) {
                    continue;
                }
                starts.push((start, format!("{year}-{quarter}"), 1));
            }
        }
    }
    starts
        .into_iter()
        .filter(|(start, _, _)| start <= &date)
        .max_by_key(|(start, _, priority)| (*start, *priority))
        .map(|(_, term, _)| term)
        .or_else(|| {
            workspace
                .get("selectedTerm")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn empty_workspace() -> Value {
    let date = (Utc::now() + chrono::Duration::hours(9)).date_naive();
    let year = if date.month() < 4 {
        date.year() - 1
    } else {
        date.year()
    };
    let term = format!("{year}-{}", quarter_for_month(date.month()));
    let mut workspace = json!({
        "schemaVersion": 3,
        "deviceId": Uuid::new_v4().to_string(),
        "selectedTerm": term,
        "terms": [term],
        "courses": [],
        "memberships": [],
        "assignments": [],
        "materials": [],
        "events": [],
        "planItems": [],
        "exceptions": [],
        "offerings": [],
        "intensiveSchedules": [],
        "plans": [],
        "academicCalendar": [],
        "gpaSummary": [],
        "departments": [],
        "recommendedCourses": [],
        "importantDates": [],
        "exams": [],
        "scheduleOverrides": [],
        "subscriptions": [],
        "scheduledTasks": [],
        "settings": {
            "appearance": "system",
            "agentAccess": {"enabled": false, "lastWrites": []},
            "density": "comfortable",
            "locale": "en",
            "notifications": {
                "classes": true,
                "assignments": true,
                "localPermission": "default"
            },
            "cloud": {
                "provider": "none",
                "clientId": "",
                "redirectUri": "http://127.0.0.1:42813/oauth/callback",
                "conflict": "resolve-manually"
            },
            "customization": {
                "background": "",
                "appCss": "",
                "sites": {
                    "portal": {"css": "", "js": "", "enabled": false},
                    "lms": {"css": "", "js": "", "enabled": false},
                    "syllabus": {"css": "", "js": "", "enabled": false}
                },
                "hooks": []
            }
        },
        "academicProfile": {"name": "Personal program", "gradePath": [], "years": []},
        "importStatus": {"school": "not-configured"},
        "syncStatus": {
            "state": "needs-configuration",
            "message": "Cloud sync is off."
        }
    });
    workspace["currentTerm"] = Value::String(term);
    merge_object(
        &mut workspace["settings"],
        json!({
            "restoreSchoolPage": false,
            "schoolServiceWidth": 168,
            "schoolWebZoom": 1,
            "showGrades": false,
            "siteConsents": {"portal": false, "lms": false, "syllabus": false},
            "subscriptionRefresh": {"frequency": "daily", "allowCellular": false}
        }),
    );
    workspace
}

fn reference_refresh_task(device_id: &str) -> Value {
    json!({
        "id": "builtin:reference-subscriptions",
        "rev": 1,
        "updatedAt": now(),
        "updatedBy": device_id,
        "name": "Reference subscriptions",
        "action": "reference-refresh",
        "schedule": "RRULE:FREQ=HOURLY;INTERVAL=1;BYMINUTE=0",
        "enabled": true
    })
}

fn lms_background_refresh_task(device_id: &str) -> Value {
    json!({
        "id": "builtin:lms-background-refresh",
        "rev": 1,
        "updatedAt": now(),
        "updatedBy": device_id,
        "name": "LMS background refresh",
        "action": "lms-refresh",
        "schedule": "RRULE:FREQ=HOURLY;INTERVAL=4;BYMINUTE=0",
        "enabled": false
    })
}

fn content_value(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for key in ["rev", "updatedAt", "updatedBy", "deletedAt"] {
            object.remove(key);
        }
    }
    value
}

fn string_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, DataError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DataError::from(format!("record is missing {key}")))
}

fn validate_contract<T: DeserializeOwned>(table: &str, value: &Value) -> Result<(), DataError> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|error| DataError::from(format!("{table} validation failed: {error}")))
}

/// Domain rules every local writer (workspace intents, MCP, scheduler) must
/// meet beyond the wire shape. Synced and imported rows are only shape-checked;
/// read models skip and report a stored record they cannot evaluate.
fn validate_local_record(table: &str, value: &Value) -> Result<(), DataError> {
    let invalid = |error: &dyn std::fmt::Display| DataError::from(format!("{table}: {error}"));
    match table {
        "plans" => serde_json::from_value::<crate::contracts::Plan>(value.clone())
            .map_err(|error| invalid(&error))?
            .validate()
            .map_err(|error| invalid(&error)),
        "schedule_overrides" => {
            serde_json::from_value::<crate::contracts::ScheduleOverride>(value.clone())
                .map_err(|error| invalid(&error))?
                .validate()
                .map_err(|error| invalid(&error))
        }
        _ => Ok(()),
    }
}

fn validate_user_record(table: &str, value: &Value) -> Result<(), DataError> {
    use crate::contracts::*;
    match table {
        "manual_courses" => validate_contract::<Course>(table, value),
        "manual_memberships" => validate_contract::<Membership>(table, value),
        "plan_items" => validate_contract::<PlanItem>(table, value),
        "personal_events" => validate_contract::<PersonalEvent>(table, value),
        "calendar_exceptions" => validate_contract::<CalendarException>(table, value),
        "assignment_marks" => validate_contract::<AssignmentMark>(table, value),
        "course_overrides" => validate_contract::<CourseOverride>(table, value),
        "intensive_schedules" => validate_contract::<IntensiveSchedule>(table, value),
        "plans" => validate_contract::<Plan>(table, value),
        "schedule_overrides" => validate_contract::<ScheduleOverride>(table, value),
        "subscriptions" => validate_contract::<Subscription>(table, value),
        "scheduled_tasks" => validate_contract::<ScheduledTask>(table, value),
        _ => Err("table has no user-data contract".into()),
    }
}

fn row_version(value: &Value) -> Result<VersionedValue, DataError> {
    let content = content_value(value);
    Ok(VersionedValue {
        hash: content_hash(&content)?,
        value: content,
        updated_at: string_field(value, "updatedAt")?.to_string(),
        updated_by: string_field(value, "updatedBy")?.to_string(),
        rev: value
            .get("rev")
            .and_then(Value::as_u64)
            .ok_or_else(|| "record is missing rev".to_string())?,
        source: value
            .pointer("/source/provider")
            .and_then(Value::as_str)
            .map(str::to_string),
        deleted_at: value
            .get("deletedAt")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn read_rows(connection: &Connection, table: &str) -> Result<Vec<Value>, DataError> {
    let sql = format!("SELECT value FROM {table} ORDER BY id");
    let mut statement = connection.prepare(&sql).map_err(DataError::from)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(DataError::from)?
        .map(|row| {
            let raw = row.map_err(DataError::from)?;
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage(format!("{table} contains invalid JSON")))
        })
        .collect();
    rows
}

fn read_visible_rows(connection: &Connection, table: &str) -> Result<Vec<Value>, DataError> {
    if !SCHOOL_CACHE_TABLES.contains(&table) {
        return read_rows(connection, table);
    }
    let sql = format!("SELECT value FROM {table} WHERE missing_since IS NULL ORDER BY id");
    let mut statement = connection.prepare(&sql).map_err(DataError::from)?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(DataError::from)?
        .map(|row| {
            let raw = row.map_err(DataError::from)?;
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage(format!("{table} contains invalid JSON")))
        })
        .collect();
    rows
}

fn fill_missing(value: &mut Value, fallback: &Value) {
    match (value, fallback) {
        (Value::Object(value), Value::Object(fallback)) => {
            for (key, fallback) in fallback {
                if let Some(current) = value.get_mut(key) {
                    fill_missing(current, fallback);
                } else {
                    value.insert(key.clone(), fallback.clone());
                }
            }
        }
        (Value::Array(value), Value::Array(fallback)) if value.len() == fallback.len() => {
            for (current, fallback) in value.iter_mut().zip(fallback) {
                fill_missing(current, fallback);
            }
        }
        _ => {}
    }
}

pub(crate) fn canonical_course_id(
    academic_year: i32,
    offering_id: &str,
) -> Result<String, DataError> {
    if !(1000..=9999).contains(&academic_year)
        || offering_id.len() != 9
        || !offering_id.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("course identity requires an academic year and 9-digit offering id".into());
    }
    Ok(format!("{academic_year}-{offering_id}"))
}

/// Resolve every school-facing identifier through one internal course id.
/// Offering ids are authoritative; source-only courses keep a deterministic
/// provider-scoped id until an offering becomes known.
pub(crate) fn resolve_course_id(
    transaction: &Transaction<'_>,
    provider: &str,
    external_id: &str,
    academic_year: i32,
    offering_id: Option<&str>,
) -> Result<String, DataError> {
    if !matches!(provider, "lms" | "syllabus" | "kyomu" | "manual") || external_id.trim().is_empty()
    {
        return Err("invalid course identity source".into());
    }
    let stored: Option<String> = transaction
        .query_row(
            "SELECT course_id FROM course_identity_map
             WHERE provider=?1 AND external_id=?2 AND academic_year=?3",
            params![provider, external_id, academic_year],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?;
    let course_id = if let Some(offering_id) = offering_id {
        canonical_course_id(academic_year, offering_id)?
    } else if let Some(stored) = stored {
        stored
    } else {
        format!("{academic_year}-{provider}-{external_id}")
    };
    transaction
        .execute(
            "INSERT INTO course_identity_map
             (provider,external_id,academic_year,course_id,offering_id,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(provider,external_id,academic_year) DO UPDATE SET
               course_id=excluded.course_id,offering_id=excluded.offering_id,
               updated_at=excluded.updated_at",
            params![
                provider,
                external_id,
                academic_year,
                course_id,
                offering_id,
                now()
            ],
        )
        .map_err(DataError::from)?;
    Ok(course_id)
}

type StoredRow = (String, u64, String, String, Option<String>, Option<String>);

fn read_stored_row(
    transaction: &Transaction<'_>,
    table: &str,
    id: &str,
) -> Result<Option<StoredRow>, DataError> {
    let sql = format!(
        "SELECT hash, rev, updated_at, updated_by, source, deleted_at FROM {table} WHERE id=?1"
    );
    transaction
        .query_row(&sql, [id], |row| {
            Ok((
                row.get(0)?,
                row.get::<_, i64>(1)? as u64,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .optional()
        .map_err(DataError::from)
}

fn upsert_row(
    transaction: &Transaction<'_>,
    table: &str,
    value: &Value,
    force: bool,
) -> Result<bool, DataError> {
    let id = string_field(value, "id")?;
    let version = row_version(value)?;
    if let Some((stored_hash, stored_rev, _, _, _, stored_deleted)) =
        read_stored_row(transaction, table, id)?
    {
        if !force && stored_rev == version.rev {
            return Ok(false);
        }
        // A stale in-memory copy never rolls back a newer stored version
        // (school imports bump rev behind the open window's back).
        if !force && version.rev < stored_rev {
            return Ok(false);
        }
        if !force && stored_hash == version.hash && stored_deleted == version.deleted_at {
            return Ok(false);
        }
    }
    let raw = serde_json::to_string(value).map_err(DataError::from)?;
    let sql = format!(
        "INSERT INTO {table}
         (id, value, hash, updated_at, updated_by, rev, source, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
           value=excluded.value, hash=excluded.hash, updated_at=excluded.updated_at,
           updated_by=excluded.updated_by, rev=excluded.rev, source=excluded.source,
           deleted_at=excluded.deleted_at"
    );
    transaction
        .execute(
            &sql,
            params![
                id,
                raw,
                version.hash,
                version.updated_at,
                version.updated_by,
                version.rev as i64,
                version.source,
                version.deleted_at
            ],
        )
        .map_err(DataError::from)?;
    Ok(true)
}

fn read_setting(connection: &Connection, key: &str) -> Result<Option<Value>, DataError> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key=?1 AND deleted_at IS NULL",
            [key],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|_| DataError::storage("settings contain invalid JSON"))
    })
    .transpose()
}

fn read_settings_export(connection: &Connection) -> Result<Vec<Value>, DataError> {
    let mut statement = connection
        .prepare(
            "SELECT key, value, hash, updated_at, updated_by, rev, source, deleted_at
             FROM settings ORDER BY key",
        )
        .map_err(DataError::from)?;
    let rows = statement
        .query_map([], |row| {
            let raw: String = row.get(1)?;
            Ok(json!({
                "key": row.get::<_, String>(0)?,
                "value": serde_json::from_str::<Value>(&raw).unwrap_or(Value::Null),
                "hash": row.get::<_, String>(2)?,
                "updatedAt": row.get::<_, String>(3)?,
                "updatedBy": row.get::<_, String>(4)?,
                "rev": row.get::<_, i64>(5)?,
                "source": row.get::<_, Option<String>>(6)?,
                "deletedAt": row.get::<_, Option<String>>(7)?
            }))
        })
        .map_err(DataError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DataError::from)?;
    Ok(rows)
}

fn setting_version(row: &Value) -> Result<VersionedValue, DataError> {
    let value = row
        .get("value")
        .cloned()
        .ok_or_else(|| "setting row is missing value".to_string())?;
    Ok(VersionedValue {
        hash: content_hash(&value)?,
        value,
        updated_at: string_field(row, "updatedAt")?.to_string(),
        updated_by: string_field(row, "updatedBy")?.to_string(),
        rev: row
            .get("rev")
            .and_then(Value::as_u64)
            .ok_or_else(|| "setting row is missing rev".to_string())?,
        source: row
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string),
        deleted_at: row
            .get("deletedAt")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn import_setting(transaction: &Transaction<'_>, row: &Value) -> Result<(), DataError> {
    let key = string_field(row, "key")?;
    let incoming = setting_version(row)?;
    transaction
        .execute(
            "INSERT INTO settings
             (key, value, hash, updated_at, updated_by, rev, source, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, hash=excluded.hash,
             updated_at=excluded.updated_at, updated_by=excluded.updated_by,
             rev=excluded.rev, source=excluded.source, deleted_at=excluded.deleted_at",
            params![
                key,
                serde_json::to_string(&incoming.value).map_err(DataError::from)?,
                incoming.hash,
                incoming.updated_at,
                incoming.updated_by,
                incoming.rev as i64,
                incoming.source,
                incoming.deleted_at
            ],
        )
        .map_err(DataError::from)?;
    Ok(())
}

fn save_setting(
    transaction: &Transaction<'_>,
    key: &str,
    value: &Value,
    device_id: &str,
) -> Result<bool, DataError> {
    let hash = content_hash(value)?;
    let existing: Option<(String, i64)> = transaction
        .query_row(
            "SELECT hash, rev FROM settings WHERE key=?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(DataError::from)?;
    if existing.as_ref().is_some_and(|(stored, _)| stored == &hash)
        && read_setting(transaction, key)?.is_some()
    {
        return Ok(false);
    }
    let rev = existing.map_or(1, |(_, rev)| rev + 1);
    transaction
        .execute(
            "INSERT INTO settings
             (key, value, hash, updated_at, updated_by, rev, source, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'user', NULL)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, hash=excluded.hash,
             updated_at=excluded.updated_at, updated_by=excluded.updated_by, rev=excluded.rev,
             source=excluded.source, deleted_at=NULL",
            params![
                key,
                serde_json::to_string(value).map_err(DataError::from)?,
                hash,
                now(),
                device_id,
                rev
            ],
        )
        .map_err(DataError::from)?;
    Ok(true)
}

/// Checks a local backup document before anything is restored from it. The
/// kind and schema come first so an old or foreign file is reported as such
/// rather than by whichever field it happens to fail on.
pub fn parse_import(document: &str) -> Result<Value, DataError> {
    let parsed: Value = serde_json::from_str(document)
        .map_err(|_| DataError::from("the file is not valid JSON".to_string()))?;
    if parsed.get("kind").and_then(Value::as_str) != Some(BACKUP_KIND) {
        return Err("the file is not an IsctCompanion backup".into());
    }
    match parsed.get("schemaVersion") {
        Some(version) if version.as_u64() == Some(BACKUP_SCHEMA_VERSION) => {}
        Some(version) => {
            return Err(DataError::compatibility(format!(
                "the backup uses schema version {version}; this app reads version {BACKUP_SCHEMA_VERSION}"
            )))
        }
        None => return Err("the backup has no schema version".into()),
    }
    let exported_at = string_field(&parsed, "exportedAt")?;
    DateTime::parse_from_rfc3339(exported_at)
        .map_err(|_| DataError::from("import exportedAt must be RFC 3339".to_string()))?;
    let tables = parsed
        .get("tables")
        .and_then(Value::as_object)
        .ok_or_else(|| "import tables must be an object".to_string())?;
    if !parsed.get("localNotes").is_some_and(Value::is_array) {
        return Err("import localNotes must be an array".into());
    }
    let required: HashSet<_> = USER_BACKUP_TABLES
        .iter()
        .copied()
        .chain(["settings"])
        .collect();
    for table in &required {
        if !tables.get(*table).is_some_and(Value::is_array) {
            return Err(DataError::compatibility(format!(
                "import table {table} must be an array"
            )));
        }
    }
    for (name, rows) in tables {
        let rows = rows
            .as_array()
            .ok_or_else(|| format!("import table {name} must be an array"))?;
        for row in rows {
            if name == "settings" {
                string_field(row, "key")?;
                setting_version(row)?;
            } else {
                string_field(row, "id")?;
                row_version(row)?;
            }
        }
    }

    Ok(parsed)
}

pub fn validate_sync_document(document: &Value) -> Result<(), DataError> {
    parse_import(&document.to_string())?;
    for table in USER_BACKUP_TABLES {
        for row in document["tables"][table].as_array().into_iter().flatten() {
            validate_user_record(table, row)?;
        }
    }
    Ok(())
}

fn put_user_record_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    table: &str,
    mut value: Value,
    actor: &str,
    expected_rev: Option<u64>,
) -> Result<Value, DataError> {
    if !DataStore::user_table(table) {
        return Err("table is not user-owned".into());
    }
    let object = value
        .as_object_mut()
        .ok_or_else(|| "record must be an object".to_string())?;
    if table == "manual_courses" {
        let offering = object
            .get("offeringId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let year = object
            .get("termId")
            .and_then(Value::as_str)
            .and_then(|term| term.get(..4))
            .and_then(|year| year.parse::<i32>().ok());
        if let (Some(offering), Some(year)) = (offering, year) {
            let source_provider = object
                .get("source")
                .and_then(|source| source.get("provider"))
                .and_then(Value::as_str);
            let provider = source_provider
                .filter(|provider| matches!(*provider, "lms" | "syllabus" | "kyomu"))
                .unwrap_or("manual");
            let external = object
                .get("source")
                .and_then(|source| source.get("externalId"))
                .and_then(Value::as_str)
                .unwrap_or(&offering)
                .to_string();
            let course_id =
                resolve_course_id(transaction, provider, &external, year, Some(&offering))?;
            object.insert("id".into(), Value::String(course_id));
        }
    }
    let requested_id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    let source_key = object
        .get("sourceKey")
        .and_then(Value::as_str)
        .map(str::to_string);
    let target_key = object
        .get("targetKey")
        .and_then(Value::as_str)
        .map(str::to_string);
    let overlay_id = if matches!(table, "assignment_marks" | "course_overrides") {
        source_key
            .as_deref()
            .map(|source_key| {
                transaction
                    .query_row(
                        &format!("SELECT id FROM {table} WHERE source_key=?1"),
                        [source_key],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(DataError::from)
            })
            .transpose()?
            .flatten()
    } else if table == "schedule_overrides" {
        target_key
            .as_deref()
            .map(|target_key| {
                transaction
                    .query_row(
                        "SELECT id FROM schedule_overrides WHERE target_key=?1",
                        [target_key],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(DataError::from)
            })
            .transpose()?
            .flatten()
    } else {
        None
    };
    let id = requested_id
        .or(overlay_id)
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    let previous: Option<(String, i64)> = transaction
        .query_row(
            &format!("SELECT value, rev FROM {table} WHERE id=?1"),
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(DataError::from)?;
    if let Some(expected) = expected_rev {
        let actual = previous.as_ref().map(|(_, rev)| *rev as u64).unwrap_or(0);
        if expected != actual {
            return Err(DataError::conflict(format!(
                "revision conflict: {table}/{id} expected {expected}, actual {actual}"
            )));
        }
    }
    let revision = previous.as_ref().map_or(1, |(_, rev)| rev + 1);
    object.insert("id".into(), Value::String(id.clone()));
    object.insert("rev".into(), Value::from(revision));
    object.insert("updatedAt".into(), Value::String(now()));
    object.insert(
        "updatedBy".into(),
        read_setting(transaction, "deviceId")?.unwrap_or_else(|| Value::String(actor.to_string())),
    );
    object
        .entry("source")
        .or_insert_with(|| json!({"provider":"manual"}));
    object.remove("deletedAt");
    if let Some(source_key) = source_key {
        if matches!(table, "assignment_marks" | "course_overrides") {
            object.insert("sourceKey".into(), Value::String(source_key));
        }
    }
    normalize_optional_nulls(&mut value);
    validate_user_record(table, &value)?;
    validate_local_record(table, &value)?;
    let unchanged = previous
        .as_ref()
        .and_then(|(raw, _)| serde_json::from_str::<Value>(raw).ok())
        .is_some_and(|old| {
            content_value(&old) == content_value(&value)
                && old.get("deletedAt") == value.get("deletedAt")
        });
    if unchanged || !upsert_row(transaction, table, &value, true)? {
        // Unchanged content keeps the stored revision; returning the
        // bumped copy would make the caller's next expected rev wrong.
        let (raw, _) = previous.ok_or_else(|| "record vanished during save".to_string())?;
        return serde_json::from_str(&raw)
            .map_err(|_| DataError::storage(format!("{table} contains invalid JSON")));
    }
    match table {
        "assignment_marks" => {
            transaction.execute(
                    "UPDATE assignment_marks SET source_key=?2,assignment_id=?3,hidden=?4,completed_at=?5 WHERE id=?1",
                    params![
                        id,
                        value.get("sourceKey").and_then(Value::as_str),
                        value.get("assignmentId").and_then(Value::as_str),
                        value.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                        value.get("completedAt").and_then(Value::as_str),
                    ],
                ).map_err(DataError::from)?;
        }
        "course_overrides" => {
            transaction.execute(
                    "UPDATE course_overrides SET source_key=?2,course_id=?3,title=?4,instructor=?5,color=?6 WHERE id=?1",
                    params![
                        id,
                        value.get("sourceKey").and_then(Value::as_str),
                        value.get("courseId").and_then(Value::as_str),
                        value.get("title").and_then(Value::as_str),
                        value.get("instructor").and_then(Value::as_str),
                        value.get("color").and_then(Value::as_str),
                    ],
                ).map_err(DataError::from)?;
        }
        "schedule_overrides" => {
            transaction
                .execute(
                    "UPDATE schedule_overrides SET target_key=?2 WHERE id=?1",
                    params![id, value.get("targetKey").and_then(Value::as_str)],
                )
                .map_err(DataError::from)?;
        }
        _ => {}
    }
    Ok(value)
}

fn delete_user_record_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    table: &str,
    id: &str,
    actor: &str,
    expected_rev: Option<u64>,
) -> Result<bool, DataError> {
    if !DataStore::user_table(table) {
        return Err("table is not user-owned".into());
    }
    let raw: Option<(String, i64)> = transaction
        .query_row(
            &format!("SELECT value, rev FROM {table} WHERE id=?1"),
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(DataError::from)?;
    let Some((raw, revision)) = raw else {
        if let Some(expected) = expected_rev.filter(|expected| *expected != 0) {
            return Err(DataError::conflict(format!(
                "revision conflict: {table}/{id} expected {expected}, actual 0"
            )));
        }
        return Ok(false);
    };
    if let Some(expected) = expected_rev.filter(|expected| *expected != revision as u64) {
        return Err(DataError::conflict(format!(
            "revision conflict: {table}/{id} expected {expected}, actual {revision}"
        )));
    }
    let mut value: Value = serde_json::from_str(&raw)
        .map_err(|_| DataError::storage(format!("{table} contains invalid JSON")))?;
    let timestamp = now();
    let object = value
        .as_object_mut()
        .ok_or_else(|| DataError::storage(format!("{table} contains a non-object record")))?;
    object.insert("rev".into(), Value::from(revision + 1));
    object.insert("updatedAt".into(), Value::String(timestamp.clone()));
    object.insert(
        "updatedBy".into(),
        read_setting(transaction, "deviceId")?.unwrap_or_else(|| Value::String(actor.to_string())),
    );
    object.insert("deletedAt".into(), Value::String(timestamp));
    upsert_row(transaction, table, &value, true)?;
    Ok(true)
}

pub(crate) enum AgentWriteError {
    Disabled,
    Data(DataError),
}

impl From<DataError> for AgentWriteError {
    fn from(error: DataError) -> Self {
        Self::Data(error)
    }
}

pub(crate) struct AgentWriteTransaction<'transaction, 'connection> {
    transaction: &'transaction rusqlite::Transaction<'connection>,
}

/// Narrow database capabilities used by the workspace domain service.  The
/// service owns policy; this type only provides revision-checked persistence
/// inside the service's single IMMEDIATE transaction.
pub(crate) struct WorkspaceWrite<'transaction, 'connection> {
    transaction: &'transaction rusqlite::Transaction<'connection>,
}

impl WorkspaceWrite<'_, '_> {
    pub(crate) fn snapshot_parts(&self) -> Result<(Value, Value), DataError> {
        Ok((
            DataStore::load_workspace_from(self.transaction)?,
            DataStore::school_records_from(self.transaction)?,
        ))
    }

    pub(crate) fn put(
        &self,
        table: &str,
        value: Value,
        expected_rev: Option<u64>,
    ) -> Result<Value, DataError> {
        put_user_record_in_transaction(self.transaction, table, value, "workspace", expected_rev)
    }

    pub(crate) fn delete(
        &self,
        table: &str,
        id: &str,
        expected_rev: Option<u64>,
    ) -> Result<bool, DataError> {
        delete_user_record_in_transaction(self.transaction, table, id, "workspace", expected_rev)
    }

    /// Whether a live row with this ID is stored in the given user table.
    pub(crate) fn has_user_row(&self, table: &str, id: &str) -> Result<bool, DataError> {
        if !DataStore::user_table(table) {
            return Err("table is not user-owned".into());
        }
        self.transaction
            .query_row(
                &format!("SELECT 1 FROM {table} WHERE id=?1 AND deleted_at IS NULL"),
                [id],
                |_| Ok(()),
            )
            .optional()
            .map(|row| row.is_some())
            .map_err(DataError::from)
    }

    pub(crate) fn save_setting(&self, key: &str, value: &Value) -> Result<(), DataError> {
        if !matches!(
            key,
            "selectedTerm"
                | "terms"
                | "academicProfile"
                | "userPreferences"
                | "devicePreferences"
                | "cloud"
                | "importStatus"
                | "syncStatus"
                | "logs"
        ) {
            return Err("setting key is not writable".into());
        }
        validate_setting(key, value)?;
        let actor = read_setting(self.transaction, "deviceId")?
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "workspace".to_string());
        save_setting(self.transaction, key, value, &actor).map(|_| ())
    }

    pub(crate) fn save_settings(&self, settings: &Value) -> Result<(), DataError> {
        validate_settings(settings)?;
        let (user, device, cloud) = settings_parts(settings);
        self.save_setting("userPreferences", &user)?;
        self.save_setting("devicePreferences", &device)?;
        self.save_setting("cloud", &cloud)
    }
}

impl AgentWriteTransaction<'_, '_> {
    pub(crate) fn rows(&self, table: &str) -> Result<Vec<Value>, DataError> {
        if !DataStore::user_table(table) {
            return Err("table is not user-owned".into());
        }
        read_rows(self.transaction, table)
    }

    pub(crate) fn put(
        &self,
        table: &str,
        value: Value,
        expected_rev: Option<u64>,
    ) -> Result<Value, DataError> {
        put_user_record_in_transaction(self.transaction, table, value, "mcp", expected_rev)
    }

    pub(crate) fn delete(
        &self,
        table: &str,
        id: &str,
        expected_rev: Option<u64>,
    ) -> Result<bool, DataError> {
        delete_user_record_in_transaction(self.transaction, table, id, "mcp", expected_rev)
    }
}

fn agent_access_enabled_in(connection: &Connection) -> Result<bool, DataError> {
    Ok(read_setting(connection, "devicePreferences")?
        .or(read_setting(connection, "settings")?)
        .and_then(|value| {
            value
                .pointer("/agentAccess/enabled")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false))
}

fn record_agent_write_in(
    transaction: &rusqlite::Transaction<'_>,
    tool: &str,
) -> Result<(), DataError> {
    let key = if read_setting(transaction, "devicePreferences")?.is_some() {
        "devicePreferences"
    } else {
        "settings"
    };
    let mut settings = read_setting(transaction, key)?.unwrap_or_else(|| json!({}));
    let object = settings
        .as_object_mut()
        .ok_or_else(|| "settings must be an object".to_string())?;
    let access = object
        .entry("agentAccess")
        .or_insert_with(|| json!({"enabled":false,"lastWrites":[]}));
    let writes = access
        .as_object_mut()
        .and_then(|access| {
            access
                .entry("lastWrites")
                .or_insert_with(|| json!([]))
                .as_array_mut()
        })
        .ok_or_else(|| "agent access history must be an array".to_string())?;
    writes.insert(0, json!({"at": now(), "tool": tool}));
    writes.truncate(20);
    save_setting(transaction, key, &settings, "mcp").map(|_| ())
}

impl DataStore {
    pub fn initialize(path: PathBuf) -> Result<(Self, bool), DataError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(DataError::from)?;
        }
        let mut connection = Connection::open(&path).map_err(DataError::from)?;
        let rebuilt = migrations::migrate(&mut connection)?;
        // Catalog offerings are never deleted by the person; earlier workspace
        // saves tombstoned rows the window had not loaded yet. Restore them.
        connection
            .execute(
                "UPDATE offerings SET deleted_at=NULL, value=json_remove(value,'$.deletedAt')
                 WHERE deleted_at IS NOT NULL",
                [],
            )
            .map_err(DataError::from)?;
        let store = Self { path };
        store.ensure_defaults()?;
        Ok((store, rebuilt))
    }

    fn open(&self) -> Result<Connection, DataError> {
        let mut connection = Connection::open(&self.path).map_err(DataError::from)?;
        migrations::migrate(&mut connection)?;
        Ok(connection)
    }

    fn ensure_defaults(&self) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let existing_device_id = read_setting(&connection, "deviceId")?;
        let workspace = empty_workspace();
        let device_id = existing_device_id
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or(string_field(&workspace, "deviceId")?);
        let transaction = connection.transaction().map_err(DataError::from)?;
        if existing_device_id.is_none() {
            for key in [
                "deviceId",
                "selectedTerm",
                "terms",
                "academicProfile",
                "importStatus",
                "syncStatus",
                "logs",
            ] {
                if let Some(value) = workspace.get(key) {
                    save_setting(&transaction, key, value, device_id)?;
                }
            }
            let (user, device, cloud) = settings_parts(&workspace["settings"]);
            save_setting(&transaction, "userPreferences", &user, device_id)?;
            save_setting(&transaction, "devicePreferences", &device, device_id)?;
            save_setting(&transaction, "cloud", &cloud, device_id)?;
        }
        upsert_row(
            &transaction,
            "scheduled_tasks",
            &reference_refresh_task(device_id),
            false,
        )?;
        upsert_row(
            &transaction,
            "scheduled_tasks",
            &lms_background_refresh_task(device_id),
            false,
        )?;
        transaction.commit().map_err(DataError::from)
    }

    fn user_table(table: &str) -> bool {
        matches!(
            table,
            "plan_items"
                | "manual_courses"
                | "manual_memberships"
                | "personal_events"
                | "calendar_exceptions"
                | "assignment_marks"
                | "course_overrides"
                | "intensive_schedules"
                | "plans"
                | "schedule_overrides"
                | "subscriptions"
                | "scheduled_tasks"
        )
    }

    /// User-owned records are changed one row at a time. The database owns
    /// IDs and revisions; callers cannot accidentally roll them backwards.
    pub fn put_user_record(
        &self,
        table: &str,
        value: Value,
        actor: &str,
    ) -> Result<Value, DataError> {
        self.put_user_record_checked(table, value, actor, None)
    }

    pub fn put_user_record_checked(
        &self,
        table: &str,
        value: Value,
        actor: &str,
        expected_rev: Option<u64>,
    ) -> Result<Value, DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let value =
            put_user_record_in_transaction(&transaction, table, value, actor, expected_rev)?;
        transaction.commit().map_err(DataError::from)?;
        Ok(value)
    }

    pub fn delete_user_record(
        &self,
        table: &str,
        id: &str,
        actor: &str,
    ) -> Result<bool, DataError> {
        self.delete_user_record_checked(table, id, actor, None)
    }

    pub fn delete_user_record_checked(
        &self,
        table: &str,
        id: &str,
        actor: &str,
        expected_rev: Option<u64>,
    ) -> Result<bool, DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let deleted =
            delete_user_record_in_transaction(&transaction, table, id, actor, expected_rev)?;
        transaction.commit().map_err(DataError::from)?;
        Ok(deleted)
    }

    pub fn user_rows(&self, table: &str) -> Result<Vec<Value>, DataError> {
        if !Self::user_table(table) {
            return Err("table is not user-owned".into());
        }
        read_rows(&self.open()?, table)
    }

    pub fn agent_access_enabled(&self) -> Result<bool, DataError> {
        let connection = self.open()?;
        agent_access_enabled_in(&connection)
    }

    pub(crate) fn agent_write<T>(
        &self,
        tool: &str,
        action: impl FnOnce(&AgentWriteTransaction<'_, '_>) -> Result<T, DataError>,
    ) -> Result<T, AgentWriteError> {
        let mut connection = self.open().map_err(AgentWriteError::Data)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)
            .map_err(AgentWriteError::Data)?;
        if !agent_access_enabled_in(&transaction).map_err(AgentWriteError::Data)? {
            return Err(AgentWriteError::Disabled);
        }
        let result = action(&AgentWriteTransaction {
            transaction: &transaction,
        })
        .map_err(AgentWriteError::Data)?;
        record_agent_write_in(&transaction, tool).map_err(AgentWriteError::Data)?;
        transaction
            .commit()
            .map_err(DataError::from)
            .map_err(AgentWriteError::Data)?;
        Ok(result)
    }

    // Local reference markers are excluded from user backups and cloud exports.
    // Empty feeds need this local evidence too: metadata may survive a restore.
    pub fn reference_cache_marker(
        &self,
        kind: &str,
        year: i32,
    ) -> Result<Option<Value>, DataError> {
        read_setting(&self.open()?, &format!("referenceCache:{kind}:{year}"))
    }

    pub fn record_reference_cache(
        &self,
        kind: &str,
        year: i32,
        marker: Value,
    ) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        save_setting(
            &transaction,
            &format!("referenceCache:{kind}:{year}"),
            &marker,
            "reference-pull",
        )?;
        transaction.commit().map_err(DataError::from)
    }

    /// Patch only the poll clock under the same SQL lock as local preferences.
    pub fn record_reference_poll(&self) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let key = if read_setting(&transaction, "devicePreferences")?.is_some() {
            "devicePreferences"
        } else {
            "settings"
        };
        let mut settings = read_setting(&transaction, key)?.unwrap_or_else(|| json!({}));
        let refresh = settings
            .as_object_mut()
            .ok_or("settings must be an object")?
            .entry("subscriptionRefresh")
            .or_insert_with(|| json!({"frequency":"daily","allowCellular":false}));
        refresh
            .as_object_mut()
            .ok_or("subscription refresh must be an object")?
            .insert("lastCheckedAt".into(), json!(now()));
        save_setting(&transaction, key, &settings, "pull-coordinator")?;
        transaction.commit().map_err(DataError::from)
    }

    #[cfg(test)]
    pub fn record_agent_write(&self, tool: &str) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        record_agent_write_in(&transaction, tool)?;
        transaction.commit().map_err(DataError::from)
    }

    pub fn put_workspace_setting(
        &self,
        key: &str,
        value: Value,
        actor: &str,
    ) -> Result<(), DataError> {
        if !matches!(
            key,
            "selectedTerm"
                | "terms"
                | "academicProfile"
                | "userPreferences"
                | "devicePreferences"
                | "cloud"
                | "importStatus"
                | "syncStatus"
                | "logs"
        ) {
            return Err("setting key is not writable".into());
        }
        validate_setting(key, &value)?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let device = read_setting(&transaction, "deviceId")?;
        save_setting(
            &transaction,
            key,
            &value,
            device.as_ref().and_then(Value::as_str).unwrap_or(actor),
        )?;
        transaction.commit().map_err(DataError::from)
    }

    pub fn load_workspace(&self) -> Result<Value, DataError> {
        self.workspace_snapshot_parts()
            .map(|(workspace, _)| workspace)
    }

    pub(crate) fn workspace_snapshot_parts(&self) -> Result<(Value, Value), DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let workspace = Self::load_workspace_from(&transaction)?;
        let school_records = Self::school_records_from(&transaction)?;
        transaction.commit().map_err(DataError::from)?;
        Ok((workspace, school_records))
    }

    pub(crate) fn workspace_write<T>(
        &self,
        expected_revision: &str,
        operation: impl FnOnce(&WorkspaceWrite<'_, '_>, &Value, &Value) -> Result<T, DataError>,
    ) -> Result<T, DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let current = Self::load_workspace_from(&transaction)?;
        let school_records = Self::school_records_from(&transaction)?;
        let actual = crate::workspace::snapshot_revision(&current, &school_records)?;
        if actual != expected_revision {
            return Err(DataError::conflict(format!(
                "workspace revision conflict: expected {expected_revision}, actual {actual}"
            )));
        }
        let result = operation(
            &WorkspaceWrite {
                transaction: &transaction,
            },
            &current,
            &school_records,
        )?;
        transaction.commit().map_err(DataError::from)?;
        Ok(result)
    }

    fn load_workspace_from(connection: &Connection) -> Result<Value, DataError> {
        let mut workspace = empty_workspace();
        for (field, table) in COLLECTIONS {
            workspace[*field] = Value::Array(read_visible_rows(connection, table)?);
        }
        if let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) {
            courses.extend(read_rows(connection, "manual_courses")?);
        }
        if let Some(memberships) = workspace
            .get_mut("memberships")
            .and_then(Value::as_array_mut)
        {
            memberships.extend(read_rows(connection, "manual_memberships")?);
        }
        let marks = read_rows(connection, "assignment_marks")?;
        if let Some(assignments) = workspace
            .get_mut("assignments")
            .and_then(Value::as_array_mut)
        {
            for assignment in assignments {
                let Some(id) = assignment.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let source_key = assignment
                    .get("sourceKey")
                    .and_then(Value::as_str)
                    .unwrap_or(id);
                let Some(mark) = marks.iter().find(|mark| {
                    mark.get("deletedAt").is_none()
                        && (mark.get("sourceKey").and_then(Value::as_str) == Some(source_key)
                            || mark.get("assignmentId").and_then(Value::as_str) == Some(id))
                }) else {
                    continue;
                };
                if let Some(object) = assignment.as_object_mut() {
                    if mark.get("hidden").and_then(Value::as_bool) == Some(true) {
                        object.insert("hidden".into(), Value::Bool(true));
                    }
                    object.insert(
                        "pinned".into(),
                        json!(mark.get("pinned").and_then(Value::as_bool).unwrap_or(false)),
                    );
                    if let Some(completed) = mark.get("completedAt").and_then(Value::as_str) {
                        object.insert("completedAt".into(), Value::String(completed.to_string()));
                    }
                }
            }
        }
        let overrides = read_rows(connection, "course_overrides")?;
        if let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) {
            for course in courses {
                let Some(id) = course.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let source_key = course
                    .get("sourceKey")
                    .and_then(Value::as_str)
                    .unwrap_or(id);
                if let Some(overlay) = overrides.iter().find(|row| {
                    row.get("deletedAt").is_none()
                        && (row.get("sourceKey").and_then(Value::as_str) == Some(source_key)
                            || row.get("courseId").and_then(Value::as_str) == Some(id))
                }) {
                    if let Some(object) = course.as_object_mut() {
                        object.insert(
                            "overrides".into(),
                            json!({
                                "title": overlay.get("title"),
                                "instructor": overlay.get("instructor"),
                                "color": overlay.get("color"),
                                "lessonLog": overlay.get("lessonLog")
                            }),
                        );
                    }
                }
            }
        }
        for key in [
            "deviceId",
            "selectedTerm",
            "terms",
            "settings",
            "academicProfile",
            "importStatus",
            "syncStatus",
            "logs",
        ] {
            if let Some(value) = read_setting(connection, key)? {
                workspace[key] = value;
            }
        }
        for key in ["userPreferences", "devicePreferences"] {
            if let Some(value) = read_setting(connection, key)? {
                apply_preferences(&mut workspace["settings"], value);
            }
        }
        if let Some(value) = read_setting(connection, "cloud")? {
            workspace["settings"]["cloud"] = value;
        }
        let today_jst = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        workspace["currentTerm"] = Value::String(
            current_term_for(&workspace, today_jst)
                .ok_or_else(|| "current academic term is not configured".to_string())?,
        );
        normalize_optional_nulls(&mut workspace);
        normalize_meeting_clocks(&mut workspace);
        let _: crate::contracts::Workspace = serde_json::from_value(workspace.clone())
            .map_err(|error| format!("workspace validation failed: {error}"))?;
        Ok(workspace)
    }

    fn school_records_from(connection: &Connection) -> Result<Value, DataError> {
        let mut records = Map::new();
        for (key, table) in [
            ("lmsCourses", "lms_courses"),
            ("registrations", "registrations"),
            ("registrationStates", "registration_states"),
            ("grades", "grades"),
            ("gpaByTerm", "gpa_by_term"),
            ("requirements", "requirement_progress"),
        ] {
            records.insert(
                key.into(),
                Value::Array(read_visible_rows(connection, table)?),
            );
        }
        records.insert("lastRuns".into(), Self::last_import_runs_from(connection)?);
        Ok(Value::Object(records))
    }

    fn last_import_runs_from(connection: &Connection) -> Result<Value, DataError> {
        let mut statement = connection
            .prepare(
                "SELECT provider, scope, MAX(finished_at) FROM import_runs
                 WHERE state='complete' AND finished_at IS NOT NULL GROUP BY provider, scope",
            )
            .map_err(DataError::from)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(DataError::from)?;
        let mut runs = Map::new();
        for row in rows {
            let (provider, scope, finished) = row.map_err(DataError::from)?;
            runs.insert(format!("{provider}:{scope}"), Value::String(finished));
        }
        Ok(Value::Object(runs))
    }

    pub fn reset_workspace(&self) -> Result<Value, DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let reset_tables = COLLECTIONS
            .iter()
            .map(|(_, table)| *table)
            .chain(SCHOOL_CACHE_TABLES.iter().copied())
            .chain(USER_BACKUP_TABLES.iter().copied())
            .chain([
                "offering_content",
                "offerings_fts",
                "catalog_search_items",
                "manual_courses",
                "manual_memberships",
                "course_overrides",
                "assignment_marks",
                "course_identity_map",
            ])
            .collect::<HashSet<_>>();
        for table in reset_tables {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(DataError::from)?;
        }
        transaction
            .execute("DELETE FROM settings", [])
            .map_err(DataError::from)?;
        transaction.commit().map_err(DataError::from)?;
        self.ensure_defaults()?;
        self.load_workspace()
    }

    pub fn export_data(&self, notes: Value) -> Result<String, DataError> {
        serde_json::to_string_pretty(&self.export_document(notes)?).map_err(DataError::from)
    }

    /// Read every backup table from one SQLite snapshot.
    pub fn export_document(&self, notes: Value) -> Result<Value, DataError> {
        let mut database = self.open()?;
        let connection = database.transaction()?;
        let mut tables = Map::new();
        for table in USER_BACKUP_TABLES {
            tables.insert(
                (*table).into(),
                Value::Array(read_rows(&connection, table)?),
            );
        }
        let settings = read_settings_export(&connection)?
            .into_iter()
            .filter(|row| {
                row.get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|key| USER_BACKUP_SETTINGS.contains(&key))
            })
            .collect();
        tables.insert("settings".into(), Value::Array(settings));

        Ok(json!({
            "schemaVersion": BACKUP_SCHEMA_VERSION,
            "exportedAt": now(),
            "kind": BACKUP_KIND,
            "deviceId": "local-backup",
            "sync": read_setting(&connection, "cloudSyncMetadata")?,
            "tables": tables,
            "localNotes": notes
        }))
    }

    /// Merge under an IMMEDIATE transaction, including edits committed during
    /// the network round trip. Never replace the device's settings or identity.
    pub fn merge_cloud_data(&self, remote: &Value) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let local_meta = json!({"sync":read_setting(&transaction, "cloudSyncMetadata")?});
        if cloud_merge::generation(remote) < cloud_merge::generation(&local_meta) {
            return Ok(());
        }
        if cloud_merge::generation(remote) > cloud_merge::generation(&local_meta) {
            for table in USER_BACKUP_TABLES {
                transaction
                    .execute(&format!("DELETE FROM {table}"), [])
                    .map_err(DataError::from)?;
            }
            for key in USER_BACKUP_SETTINGS {
                transaction
                    .execute("DELETE FROM settings WHERE key=?1", [key])
                    .map_err(DataError::from)?;
            }
        }
        for table in USER_BACKUP_TABLES {
            let local = read_rows(&transaction, table)?;
            for incoming in remote["tables"][table].as_array().into_iter().flatten() {
                validate_user_record(table, incoming)?;
                let old = local.iter().find(|row| row["id"] == incoming["id"]);
                if old.is_none_or(|old| cloud_merge::compare(incoming, old).is_gt()) {
                    upsert_row(&transaction, table, incoming, true)?;
                }
            }
        }
        let local = read_settings_export(&transaction)?;
        for incoming in remote["tables"]["settings"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut incoming = incoming.clone();
            cloud_merge::sanitize_setting(&mut incoming);
            let incoming = &incoming;
            let key = string_field(incoming, "key")?;
            if !USER_BACKUP_SETTINGS.contains(&key) {
                continue;
            }
            if local
                .iter()
                .find(|row| row["key"] == key)
                .is_none_or(|old| cloud_merge::compare(incoming, old).is_gt())
            {
                import_setting(&transaction, incoming)?;
            }
        }
        if remote["sync"].is_object() {
            transaction.execute("INSERT INTO settings(key,value,hash,updated_at,updated_by,rev) VALUES('cloudSyncMetadata',?1,?3,?2,'sync',1) ON CONFLICT(key) DO UPDATE SET value=excluded.value,hash=excluded.hash,updated_at=excluded.updated_at",
                params![remote["sync"].to_string(), now(), content_hash(&remote["sync"])?]).map_err(DataError::from)?;
        }
        transaction.commit().map_err(DataError::from)
    }

    /// Restores a document that passed [`parse_import`].
    pub fn import_data(&self, parsed: &Value) -> Result<Value, DataError> {
        let tables = parsed
            .get("tables")
            .and_then(Value::as_object)
            .ok_or_else(|| DataError::compatibility("import tables must be an object"))?;
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let allowed: HashSet<_> = USER_BACKUP_TABLES.iter().copied().collect();
        // A local backup restore is an explicit replacement, not sync/merge.
        for table in USER_BACKUP_TABLES {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(DataError::from)?;
        }
        for (table, rows) in tables {
            if table == "settings" {
                continue;
            }
            if !allowed.contains(table.as_str()) {
                return Err(DataError::compatibility(format!(
                    "import contains unsupported table {table}"
                )));
            }
            let incoming_rows = rows.as_array().ok_or_else(|| {
                DataError::compatibility(format!("import table {table} must be an array"))
            })?;
            for incoming in incoming_rows {
                validate_user_record(table, incoming)?;
                upsert_row(&transaction, table, incoming, true)?;
            }
        }
        if let Some(settings) = tables.get("settings").and_then(Value::as_array) {
            for key in USER_BACKUP_SETTINGS {
                transaction
                    .execute("DELETE FROM settings WHERE key=?1", [key])
                    .map_err(DataError::from)?;
            }
            for row in settings {
                let key = string_field(row, "key")?;
                if !USER_BACKUP_SETTINGS.contains(&key) {
                    return Err(DataError::compatibility(format!(
                        "import contains device or secret setting {key}"
                    )));
                }
                import_setting(&transaction, row)?;
            }
        }
        transaction.commit().map_err(DataError::from)?;
        self.load_workspace()
    }

    pub fn catalog_upsert_offerings(&self, offerings: &[Value]) -> Result<usize, DataError> {
        for offering in offerings {
            string_field(offering, "id")?;
        }
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let mut changed = 0;
        for offering in offerings {
            let mut offering = offering.clone();
            let offering_id = string_field(&offering, "id")?.to_string();
            let search_item: Option<String> = transaction
                .query_row(
                    "SELECT value FROM catalog_search_items WHERE id=?1",
                    [&offering_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(DataError::from)?;
            if let Some(search_item) =
                search_item.and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            {
                for field in ["title", "instructors", "academicUnit"] {
                    if let (Some(current), Some(fallback)) =
                        (offering.get_mut(field), search_item.get(field))
                    {
                        fill_missing(current, fallback);
                    }
                }
            }
            let timestamp = now();
            let source_key = format!(
                "syllabus:{}:offering:{}",
                offering
                    .get("year")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
                offering
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            );
            let object = offering
                .as_object_mut()
                .ok_or_else(|| "offering must be an object".to_string())?;
            object.entry("rev").or_insert(Value::from(1));
            object
                .entry("updatedAt")
                .or_insert(Value::String(timestamp.clone()));
            object
                .entry("updatedBy")
                .or_insert(Value::String("catalog".into()));
            object
                .entry("sourceKey")
                .or_insert(Value::String(source_key));
            object
                .entry("lastSeenAt")
                .or_insert(Value::String(timestamp.clone()));
            serde_json::from_value::<crate::contracts::Offering>(offering.clone())
                .map_err(|error| format!("offering validation failed: {error}"))?;
            changed += usize::from(upsert_row(&transaction, "offerings", &offering, true)?);

            let id = string_field(&offering, "id")?;
            let code = offering
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title_ja = offering
                .pointer("/title/ja")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title_en = offering
                .pointer("/title/en")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let instructors_ja = offering
                .get("instructors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| name.get("ja").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            let instructors_en = offering
                .get("instructors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| name.get("en").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            transaction
                .execute("DELETE FROM offerings_fts WHERE offering_id=?1", [id])
                .map_err(DataError::from)?;
            transaction
                .execute(
                    "INSERT INTO offerings_fts
                     (offering_id, code, title_ja, title_en, instructors_ja, instructors_en)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, code, title_ja, title_en, instructors_ja, instructors_en],
                )
                .map_err(DataError::from)?;
        }
        transaction.commit().map_err(DataError::from)?;
        Ok(changed)
    }

    /// Search rows are deliberately stored apart from complete offerings.
    /// A partial result can therefore never masquerade as a persisted entity.
    pub fn catalog_upsert_search_items(&self, items: &[Value]) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        for item in items {
            validate_contract::<crate::contracts::CatalogSearchItem>("catalog_search_items", item)?;
            let id = string_field(item, "id")?;
            let code = item.get("code").and_then(Value::as_str).unwrap_or_default();
            let title_ja = item
                .pointer("/title/ja")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title_en = item
                .pointer("/title/en")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let instructors_ja = item
                .get("instructors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| name.get("ja").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            let instructors_en = item
                .get("instructors")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|name| name.get("en").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ");
            transaction
                .execute(
                    "INSERT INTO catalog_search_items(id,value,updated_at) VALUES(?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
                    params![
                        id,
                        serde_json::to_string(item).map_err(DataError::from)?,
                        now()
                    ],
                )
                .map_err(DataError::from)?;
            transaction
                .execute("DELETE FROM offerings_fts WHERE offering_id=?1", [id])
                .map_err(DataError::from)?;
            transaction.execute(
                "INSERT INTO offerings_fts(offering_id,code,title_ja,title_en,instructors_ja,instructors_en)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![id,code,title_ja,title_en,instructors_ja,instructors_en],
            ).map_err(DataError::from)?;
        }
        transaction.commit().map_err(DataError::from)
    }

    pub fn upsert_offering_content(&self, rows: &[Value]) -> Result<(), DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        for row in rows {
            let offering_id = string_field(row, "offeringId")?;
            let lang = string_field(row, "lang")?;
            if !matches!(lang, "ja" | "en") {
                return Err("offering content language must be ja or en".into());
            }
            let content = row
                .get("content")
                .ok_or_else(|| "offering content is missing content".to_string())?;
            let fetched_at = content
                .get("fetchedAt")
                .and_then(Value::as_str)
                .unwrap_or("");
            transaction
                .execute(
                    "INSERT INTO offering_content(offering_id, lang, content, fetched_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(offering_id, lang) DO UPDATE SET
                     content=excluded.content, fetched_at=excluded.fetched_at",
                    params![
                        offering_id,
                        lang,
                        serde_json::to_string(content).map_err(DataError::from)?,
                        fetched_at
                    ],
                )
                .map_err(DataError::from)?;
        }
        transaction.commit().map_err(DataError::from)
    }

    pub fn catalog_search(&self, query: &str) -> Result<Vec<Value>, DataError> {
        let connection = self.open()?;
        if query.trim().is_empty() {
            let mut statement = connection
                .prepare(
                    "SELECT value FROM offerings
                     WHERE deleted_at IS NULL AND missing_since IS NULL
                     UNION ALL
                     SELECT s.value FROM catalog_search_items s
                     WHERE NOT EXISTS (
                       SELECT 1 FROM offerings o
                       WHERE o.id=s.id AND o.deleted_at IS NULL AND o.missing_since IS NULL
                     )
                     ORDER BY 1",
                )
                .map_err(DataError::from)?;
            return statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(DataError::from)?
                .map(|row| {
                    serde_json::from_str(&row.map_err(DataError::from)?)
                        .map_err(|_| DataError::storage("catalog row contains invalid JSON"))
                })
                .collect();
        }
        let expression = query
            .split_whitespace()
            .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
            .map(|token| format!("{token}*"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut statement = connection
            .prepare(
                "SELECT COALESCE(o.value,s.value) FROM offerings_fts f
                 LEFT JOIN offerings o ON o.id=f.offering_id AND o.deleted_at IS NULL AND o.missing_since IS NULL
                 LEFT JOIN catalog_search_items s ON s.id=f.offering_id
                 WHERE offerings_fts MATCH ?1 AND (o.id IS NOT NULL OR s.id IS NOT NULL)
                 ORDER BY rank LIMIT 100",
            )
            .map_err(DataError::from)?;
        let rows = statement
            .query_map([expression], |row| row.get::<_, String>(0))
            .map_err(DataError::from)?
            .map(|row| {
                serde_json::from_str(&row.map_err(DataError::from)?)
                    .map_err(|_| DataError::storage("offering contains invalid JSON"))
            })
            .collect();
        rows
    }

    pub fn catalog_offering(&self, id: &str) -> Result<Option<Value>, DataError> {
        let connection = self.open()?;
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM offerings WHERE id=?1 AND deleted_at IS NULL AND missing_since IS NULL",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(DataError::from)?;
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage("offering contains invalid JSON"))
        })
        .transpose()
    }

    /// A syllabus search row stored before its offering's details are read.
    pub fn catalog_search_item(&self, id: &str) -> Result<Option<Value>, DataError> {
        let connection = self.open()?;
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM catalog_search_items WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(DataError::from)?;
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage("search row contains invalid JSON"))
        })
        .transpose()
    }

    pub fn lms_material_for_url(&self, url: &str) -> Result<Option<Value>, DataError> {
        let connection = self.open()?;
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM lms_items
                 WHERE missing_since IS NULL AND deleted_at IS NULL
                   AND (json_extract(value,'$.url')=?1 OR json_extract(value,'$.resourceUrl')=?1)
                 LIMIT 1",
                [url],
                |row| row.get(0),
            )
            .optional()
            .map_err(DataError::from)?;
        raw.map(|raw| {
            serde_json::from_str(&raw)
                .map_err(|_| DataError::storage("LMS material contains invalid JSON"))
        })
        .transpose()
    }

    /// Store a lazy `view.php` resolution exactly like an LMS ingest update:
    /// unchanged content keeps its revision, while a changed remote URL gets
    /// one new version. Local cache paths never enter the database.
    pub fn update_lms_resource_url(
        &self,
        cmid: u64,
        source_url: &str,
        resource_url: &str,
    ) -> Result<bool, DataError> {
        self.update_lms_item_detail(
            cmid,
            source_url,
            "resourceUrl",
            Value::String(resource_url.into()),
        )
    }

    /// Keeps a folder's resolved file list on its material, so reopening it
    /// needs no LMS request. A later crawl keeps it until a new read replaces it.
    pub fn update_lms_folder_files(
        &self,
        cmid: u64,
        source_url: &str,
        files: &[crate::contracts::FolderFile],
    ) -> Result<bool, DataError> {
        let files = serde_json::to_value(files).map_err(DataError::from)?;
        self.update_lms_item_detail(cmid, source_url, "folderFiles", files)
    }

    /// Writes one resolved detail onto a live LMS material, as a versioned
    /// change; an unchanged value is not rewritten.
    fn update_lms_item_detail(
        &self,
        cmid: u64,
        source_url: &str,
        key: &str,
        detail: Value,
    ) -> Result<bool, DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let stored: Option<(String, String, u64)> = transaction
            .query_row(
                "SELECT id,value,rev FROM lms_items WHERE cmid=?1 AND missing_since IS NULL LIMIT 1",
                [cmid as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? as u64)),
            )
            .optional()
            .map_err(DataError::from)?;
        let Some((id, raw, revision)) = stored else {
            return Ok(false);
        };
        let mut value: Value = serde_json::from_str(&raw)
            .map_err(|_| DataError::storage("LMS material contains invalid JSON"))?;
        if value.get("url").and_then(Value::as_str) != Some(source_url) {
            return Ok(false);
        }
        if value.get(key) == Some(&detail) {
            return Ok(false);
        }
        value[key] = detail;
        let timestamp = now();
        let next_revision = revision + 1;
        value["rev"] = next_revision.into();
        value["updatedAt"] = timestamp.clone().into();
        value["updatedBy"] = "lms".into();
        value["source"] = json!({"provider":"lms"});
        value["lastSeenAt"] = timestamp.clone().into();
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
        let hash = content_hash(&content)?;
        transaction
            .execute(
                "UPDATE lms_items SET value=?2,hash=?3,updated_at=?4,updated_by='lms',
                 rev=?5,source='lms',last_seen_at=?4,missing_since=NULL,deleted_at=NULL WHERE id=?1",
                params![
                    id,
                    serde_json::to_string(&value).map_err(DataError::from)?,
                    hash,
                    timestamp,
                    next_revision as i64
                ],
            )
            .map_err(DataError::from)?;
        transaction.commit().map_err(DataError::from)?;
        Ok(true)
    }

    pub fn table_rows(&self, table: &str) -> Result<Vec<Value>, DataError> {
        const READABLE: &[&str] = &[
            "registrations",
            "registration_states",
            "grades",
            "requirement_progress",
            "lms_courses",
            "lms_items",
            "assignments",
            "gpa_summary",
            "gpa_by_term",
            "offering_content",
            "recommended_courses",
        ];
        if !READABLE.contains(&table) {
            return Err("table is not available to agent access".into());
        }
        let connection = self.open()?;
        if table == "offering_content" {
            let mut statement = connection
                .prepare(
                    "SELECT json_object('offeringId', offering_id, 'lang', lang,
                     'content', json(content), 'fetchedAt', fetched_at)
                     FROM offering_content ORDER BY offering_id, lang",
                )
                .map_err(DataError::from)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(DataError::from)?
                .map(|row| {
                    serde_json::from_str(&row.map_err(DataError::from)?)
                        .map_err(|_| DataError::storage("offering content is invalid JSON"))
                })
                .collect();
            return rows;
        }
        read_visible_rows(&connection, table)
    }

    /// When each importer (`lms`, `kyomu` per page kind) last finished a
    /// complete run: the cache age the app compares with its TTLs.
    pub fn last_import_runs(&self) -> Result<Value, DataError> {
        let connection = self.open()?;
        Self::last_import_runs_from(&connection)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn normalize_meeting_clocks(workspace: &mut Value) {
    let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) else {
        return;
    };
    for meeting in courses
        .iter_mut()
        .filter_map(|course| course.get_mut("meetings"))
        .filter_map(Value::as_array_mut)
        .flatten()
    {
        let valid_clock = |key: &str| {
            meeting
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|clock| {
                    let bytes = clock.as_bytes();
                    matches!(bytes.len(), 4 | 5)
                        && bytes[bytes.len() - 3] == b':'
                        && bytes
                            .iter()
                            .enumerate()
                            .all(|(index, byte)| index == bytes.len() - 3 || byte.is_ascii_digit())
                })
        };
        if valid_clock("startTime") && valid_clock("endTime") {
            continue;
        }
        let Some(periods) = meeting.get("periods").and_then(Value::as_array) else {
            continue;
        };
        let mut periods = periods.iter().filter_map(Value::as_u64);
        let Some(first) = periods.next() else {
            continue;
        };
        let (minimum, maximum) = periods.fold((first, first), |(minimum, maximum), value| {
            (minimum.min(value), maximum.max(value))
        });
        let (Ok(minimum), Ok(maximum)) = (u8::try_from(minimum), u8::try_from(maximum)) else {
            continue;
        };
        if let Ok((start, end)) = crate::schedule::clock_for_periods([minimum, maximum]) {
            if let Some(object) = meeting.as_object_mut() {
                object.insert("startTime".into(), Value::String(start.into()));
                object.insert("endTime".into(), Value::String(end.into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_stored_rows_are_storage_failures() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE plans (id TEXT, value TEXT);
             CREATE TABLE assignments (id TEXT, value TEXT, missing_since TEXT);
             INSERT INTO plans VALUES ('broken', 'not JSON');
             INSERT INTO assignments VALUES ('broken', 'not JSON', NULL);",
            )
            .unwrap();
        for result in [
            read_rows(&connection, "plans"),
            read_visible_rows(&connection, "assignments"),
        ] {
            let error = result.unwrap_err();
            assert!(matches!(error, DataError::Storage(_)));
            assert!(error.to_string().ends_with("contains invalid JSON"));
        }
    }
    use std::fs;

    fn store(name: &str) -> DataStore {
        let path =
            std::env::temp_dir().join(format!("isct-companion-{name}-{}.db", Uuid::new_v4()));
        DataStore::initialize(path).unwrap().0
    }

    #[test]
    fn legacy_meeting_periods_receive_the_shared_core_clock() {
        let mut workspace = json!({"courses":[{"meetings":[{
            "periods":[5,6,7,8], "startTime":"", "endTime":""
        }]}]});
        normalize_meeting_clocks(&mut workspace);
        assert_eq!(workspace["courses"][0]["meetings"][0]["startTime"], "13:30");
        assert_eq!(workspace["courses"][0]["meetings"][0]["endTime"], "17:05");
    }

    #[test]
    fn reference_poll_clock_preserves_other_device_preferences() {
        let store = store("reference-poll");
        store
            .put_workspace_setting(
                "devicePreferences",
                json!({
                    "appearance":"dark", "locale":"ja", "subscriptionRefresh":{
                        "frequency":"daily", "allowCellular":true
                    }
                }),
                "test",
            )
            .unwrap();
        store.record_reference_poll().unwrap();
        let workspace = store.load_workspace().unwrap();
        assert_eq!(workspace["settings"]["appearance"], "dark");
        assert_eq!(workspace["settings"]["locale"], "ja");
        assert_eq!(
            workspace["settings"]["subscriptionRefresh"]["allowCellular"],
            true
        );
        assert!(DateTime::parse_from_rfc3339(
            workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"]
                .as_str()
                .unwrap()
        )
        .is_ok());
    }

    #[test]
    fn restore_generation_is_committed_with_replacement_and_rejects_old_uploads() {
        let store = store("restore-generation");
        store.put_user_record("plans",json!({"title":"Later","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),"test").unwrap();
        let old: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
        let mut restored = old.clone();
        restored["tables"]["plans"] = json!([]);
        restored["sync"] =
            json!({"workspaceId":"workspace-a","generation":{"counter":1,"id":"restore-a"}});
        store.merge_cloud_data(&restored).unwrap();
        store.merge_cloud_data(&old).unwrap();
        assert!(store.user_rows("plans").unwrap().is_empty());
        let current: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
        assert_eq!(current["sync"], restored["sync"]);
        assert!(store.load_workspace().unwrap()["deviceId"].is_string());
    }
    #[test]
    fn cloud_merge_preserves_offline_edits_device_state_and_concurrent_local_writes() {
        let a = store("cloud-a");
        let b = store("cloud-b");
        a.put_workspace_setting("devicePreferences", json!({"density":"compact"}), "test")
            .unwrap();
        b.put_workspace_setting(
            "devicePreferences",
            json!({"density":"comfortable"}),
            "test",
        )
        .unwrap();
        let row_a = a
            .put_user_record(
                "plans",
                json!({"title":"A","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
                "desktop",
            )
            .unwrap();
        b.put_user_record(
            "plans",
            json!({"title":"B","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
            "desktop",
        )
        .unwrap();
        let exported_a: Value = serde_json::from_str(&a.export_data(json!([])).unwrap()).unwrap();
        let exported_b: Value = serde_json::from_str(&b.export_data(json!([])).unwrap()).unwrap();
        a.delete_user_record("plans", row_a["id"].as_str().unwrap(), "desktop")
            .unwrap();
        a.merge_cloud_data(&cloud_merge::merge(&exported_a, &exported_b))
            .unwrap();
        let latest: Value = serde_json::from_str(&a.export_data(json!([])).unwrap()).unwrap();
        b.merge_cloud_data(&latest).unwrap();
        assert_eq!(
            a.user_rows("plans")
                .unwrap()
                .iter()
                .filter(|row| row.get("deletedAt").is_none())
                .count(),
            1
        );
        assert_eq!(a.user_rows("plans").unwrap(), b.user_rows("plans").unwrap());
        assert_eq!(
            a.load_workspace().unwrap()["settings"]["density"],
            "compact"
        );
        assert_eq!(
            b.load_workspace().unwrap()["settings"]["density"],
            "comfortable"
        );
        assert_eq!(latest["tables"]["plans"].as_array().unwrap().len(), 2); // includes tombstone
        let _ = fs::remove_file(a.path());
        let _ = fs::remove_file(b.path());
    }

    #[test]
    fn a_course_override_keeps_the_lessons_marked_for_its_classes() {
        let store = store("lesson-log");
        let course = json!({"id":"c1","rev":1,"updatedAt":"2026-10-01T00:00:00Z","updatedBy":"test",
            "kind":"course","termId":"2026-3Q","code":"CSC.T273","title":"OOP","instructor":"","credits":2,
            "creditSplit":{"lecture":2,"exercise":0,"lab":0},"instructionLanguage":"ja","color":"blue",
            "meetings":[],"source":{"provider":"manual"}});
        store
            .open()
            .unwrap()
            .execute(
                "INSERT INTO manual_courses (id,value,hash,updated_at,updated_by,rev,source)
             VALUES ('c1',?1,'h','2026-10-01T00:00:00Z','test',1,'manual')",
                [course.to_string()],
            )
            .unwrap();
        store
            .put_user_record(
                "course_overrides",
                json!({"sourceKey":"c1","courseId":"c1","source":{"provider":"manual"},
                    "lessonLog":[{"occurrence":"2026-10-08T10:45:00+09:00","sectionId":12}]}),
                "test",
            )
            .unwrap();
        let workspace = store.load_workspace().unwrap();
        let course = &workspace["courses"].as_array().unwrap()[0];
        assert_eq!(course["overrides"]["lessonLog"][0]["sectionId"], 12);
    }

    #[test]
    fn keeps_a_resolved_folder_listing_once_and_reads_it_back() {
        let store = store("folder-files");
        let url = "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=9";
        {
            let connection = store.open().unwrap();
            connection
                .execute(
                    "INSERT INTO lms_items (id,cmid,value,hash,updated_at,updated_by,rev,source)
                     VALUES ('m9',9,?1,'h','2026-10-01T00:00:00Z','lms',1,'lms')",
                    [json!({"id":"m9","cmid":9,"module":"folder","url":url}).to_string()],
                )
                .unwrap();
        }
        let files = vec![crate::contracts::FolderFile {
            name: "20260928.zip".into(),
            url: "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_folder/content/0/20260928.zip"
                .into(),
        }];
        assert!(store.update_lms_folder_files(9, url, &files).unwrap());
        // The same listing again is not a change.
        assert!(!store.update_lms_folder_files(9, url, &files).unwrap());
        let row = store.lms_material_for_url(url).unwrap().unwrap();
        assert_eq!(row["rev"], 2);
        assert_eq!(row["folderFiles"][0]["name"], "20260928.zip");
        // A different source URL never takes another material's listing.
        assert!(!store
            .update_lms_folder_files(
                9,
                "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=10",
                &files
            )
            .unwrap());
    }

    #[test]
    fn summary_metadata_keeps_other_search_languages() {
        let mut summary = json!({
            "title": {"en": "Assembly Language"},
            "instructors": [{"en": "Teacher"}]
        });
        let search = json!({
            "title": {"ja": "アセンブリ言語", "en": "Old English"},
            "instructors": [{"ja": "教員", "en": "Old Teacher"}]
        });
        fill_missing(&mut summary, &search);
        assert_eq!(summary["title"]["en"], "Assembly Language");
        assert_eq!(summary["title"]["ja"], "アセンブリ言語");
        assert_eq!(summary["instructors"][0]["ja"], "教員");
    }

    #[test]
    fn current_term_uses_profile_then_calendar_then_selected_term() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 4).unwrap();
        let profile = json!({
            "academicProfile": {"years":[{"academicYear":2026,"quarterStarts":{
                "1Q":"2026-04-01","2Q":"2026-06-05","3Q":"2026-10-01","4Q":"2026-12-01"
            }}]},
            "academicCalendar":[{"date":"2026-06-01","type":"quarter-start","academicYear":2026,"quarter":"2Q"}]
        });
        assert_eq!(current_term_for(&profile, date).as_deref(), Some("2026-1Q"));
        let calendar = json!({
            "academicProfile":{"years":[]},
            "academicCalendar":[{"date":"2026-06-01","type":"quarter-start","academicYear":2026,"quarter":"2Q"}]
        });
        assert_eq!(
            current_term_for(&calendar, date).as_deref(),
            Some("2026-2Q")
        );
        assert_eq!(
            current_term_for(
                &json!({
                    "selectedTerm":"2027-3Q",
                    "academicProfile":{"years":[{"academicYear":2027,"quarterStarts":{
                        "1Q":"","2Q":"","3Q":"","4Q":""
                    }}]},
                    "academicCalendar":[]
                }),
                chrono::NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()
            ),
            Some("2027-3Q".to_string())
        );
    }

    #[test]
    fn installs_reference_refresh_as_a_user_controllable_scheduled_task() {
        let store = store("reference-refresh-task");
        let workspace = store.load_workspace().unwrap();
        let tasks = workspace["scheduledTasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
        let reference = tasks
            .iter()
            .find(|task| task["id"] == "builtin:reference-subscriptions")
            .unwrap();
        assert_eq!(reference["action"], "reference-refresh");
        assert_eq!(reference["enabled"], true);
        let lms = tasks
            .iter()
            .find(|task| task["id"] == "builtin:lms-background-refresh")
            .unwrap();
        assert_eq!(lms["action"], "lms-refresh");
        assert_eq!(lms["enabled"], false);

        store
            .delete_user_record("scheduled_tasks", "builtin:reference-subscriptions", "test")
            .unwrap();
        store.ensure_defaults().unwrap();
        let workspace = store.load_workspace().unwrap();
        let tasks = workspace["scheduledTasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks
            .iter()
            .find(|task| task["id"] == "builtin:reference-subscriptions")
            .unwrap()["deletedAt"]
            .is_string());
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn record_mutations_never_delete_absent_rows() {
        let store = store("record-mutations");
        let first = store.put_user_record("plans", json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}), "test").unwrap();
        let second = store.put_user_record("plans", json!({"title":"Two","start":"2026-10-07T10:00:00+09:00","duration":60,"exdates":[]}), "test").unwrap();
        let mut changed = first.clone();
        changed["title"] = json!("Updated");
        store.put_user_record("plans", changed, "test").unwrap();
        let rows = store.user_rows("plans").unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row["id"] == second["id"]));
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn checked_record_mutations_reject_stale_writers() {
        let store = store("record-conflict");
        let row = store
            .put_user_record(
                "plans",
                json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
                "test",
            )
            .unwrap();
        let mut first = row.clone();
        first["title"] = json!("First writer");
        store
            .put_user_record_checked("plans", first, "first", Some(1))
            .unwrap();
        let mut stale = row;
        stale["title"] = json!("Stale writer");
        let error = store
            .put_user_record_checked("plans", stale, "stale", Some(1))
            .unwrap_err();
        assert!(error.to_string().contains("revision conflict"));
        assert_eq!(
            store.user_rows("plans").unwrap()[0]["title"],
            "First writer"
        );
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn unchanged_checked_put_returns_the_stored_revision() {
        let store = store("record-unchanged");
        let row = store
            .put_user_record(
                "plans",
                json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
                "test",
            )
            .unwrap();
        let mut edited = row.clone();
        edited["title"] = json!("Edited");
        let edited = store
            .put_user_record_checked("plans", edited, "desktop", Some(1))
            .unwrap();
        assert_eq!(edited["rev"], 2);
        let same = store
            .put_user_record_checked("plans", edited.clone(), "desktop", Some(2))
            .unwrap();
        let stored = store.user_rows("plans").unwrap()[0].clone();
        assert_eq!(same["rev"], stored["rev"]);
        assert_eq!(same, stored);
        assert_eq!(same["rev"], 2);
        let mut next = same;
        next["title"] = json!("Edited again");
        let next = store
            .put_user_record_checked("plans", next, "desktop", Some(2))
            .unwrap();
        assert_eq!(next["rev"], 3);
        assert_eq!(
            store.user_rows("plans").unwrap()[0]["title"],
            "Edited again"
        );
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn explicit_delete_is_a_user_tombstone() {
        let store = store("explicit-delete");
        let row = store.put_user_record("plans", json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}), "test").unwrap();
        assert!(store
            .delete_user_record("plans", row["id"].as_str().unwrap(), "test")
            .unwrap());
        assert!(store.user_rows("plans").unwrap()[0]["deletedAt"].is_string());
        let _ = fs::remove_file(store.path());
    }

    fn insert_school_assignment(store: &DataStore) {
        let mut connection = store.open().unwrap();
        let transaction = connection.transaction().unwrap();
        upsert_row(
            &transaction,
            "assignments",
            &json!({
                "id":"assignment-1", "kind":"assignment", "courseId":"course-1",
                "title":"Work", "details":"", "dueAt":null,
                "sourceKey":"lms:default:assignment:42", "source":{"provider":"lms"},
                "rev":1, "updatedAt":"2026-10-06T00:00:00Z", "updatedBy":"lms"
            }),
            true,
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn course_identity_is_shared_by_every_provider() {
        let store = store("course-identity");
        let mut connection = store.open().unwrap();
        let transaction = connection.transaction().unwrap();
        let lms = resolve_course_id(&transaction, "lms", "42", 2026, Some("202602440")).unwrap();
        let syllabus = resolve_course_id(
            &transaction,
            "syllabus",
            "202602440",
            2026,
            Some("202602440"),
        )
        .unwrap();
        let kyomu =
            resolve_course_id(&transaction, "kyomu", "202602440", 2026, Some("202602440")).unwrap();
        transaction.commit().unwrap();
        assert_eq!(lms, "2026-202602440");
        assert_eq!(syllabus, lms);
        assert_eq!(kyomu, lms);
        let rows: i64 = store
            .open()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM course_identity_map WHERE course_id='2026-202602440'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 3);
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn catalog_list_includes_search_rows_until_details_replace_them() {
        let store = store("catalog-search-fallback");
        store.catalog_upsert_search_items(&[json!({
            "id":"202602427", "year":2026, "code":"CSC.T263",
            "title":{"en":"Assembly Language"}, "instructors":[],
            "quarters":["3Q"], "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/2026/CSC.T263/202602427"
        })]).unwrap();
        let rows = store.catalog_search("").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], "202602427");
        assert_eq!(rows[0]["detailed"], Value::Null);
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn assignment_pin_round_trips_as_a_syncable_overlay() {
        let store = store("pinned-assignment");
        insert_school_assignment(&store);
        store
            .put_user_record(
                "assignment_marks",
                json!({"sourceKey":"lms:default:assignment:42","hidden":false,"pinned":true}),
                "test",
            )
            .unwrap();
        assert_eq!(
            store.load_workspace().unwrap()["assignments"][0]["pinned"],
            true
        );
        let exported: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
        assert_eq!(exported["tables"]["assignment_marks"][0]["pinned"], true);
        store
            .put_user_record(
                "assignment_marks",
                json!({"sourceKey":"lms:default:assignment:42","hidden":false,"pinned":false}),
                "test",
            )
            .unwrap();
        assert_eq!(
            store.load_workspace().unwrap()["assignments"][0]["pinned"],
            false
        );
    }

    #[test]
    fn overlay_survives_school_disappearance_and_reappearance() {
        let store = store("overlay-lifetime");
        insert_school_assignment(&store);
        store
            .put_user_record(
                "assignment_marks",
                json!({
                    "sourceKey":"lms:default:assignment:42", "hidden":true
                }),
                "test",
            )
            .unwrap();
        assert_eq!(
            store.load_workspace().unwrap()["assignments"][0]["hidden"],
            true
        );

        let connection = store.open().unwrap();
        connection.execute(
            "UPDATE assignments SET missing_since='2026-10-07T00:00:00Z',value=json_set(value,'$.missingSince','2026-10-07T00:00:00Z') WHERE id='assignment-1'",
            [],
        ).unwrap();
        assert_eq!(
            store.load_workspace().unwrap()["assignments"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(store.user_rows("assignment_marks").unwrap().len(), 1);

        connection.execute(
            "UPDATE assignments SET missing_since=NULL,value=json_remove(value,'$.missingSince') WHERE id='assignment-1'",
            [],
        ).unwrap();
        assert_eq!(
            store.load_workspace().unwrap()["assignments"][0]["hidden"],
            true
        );
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn local_backup_contains_user_data_overlays_and_notes_but_no_school_cache() {
        let store = store("backup-boundary");
        insert_school_assignment(&store);
        store.put_user_record("plans", json!({
            "title":"Study", "start":"2026-10-06T10:00:00+09:00", "duration":60, "exdates":[]
        }), "test").unwrap();
        store
            .put_user_record(
                "assignment_marks",
                json!({
                    "sourceKey":"lms:default:assignment:42", "hidden":true
                }),
                "test",
            )
            .unwrap();
        let document: Value = serde_json::from_str(
            &store
                .export_data(json!([{
                    "id":"018f0000-0000-7000-8000-000000000001", "term":"2026-3Q",
                    "course":"TEST", "section":"section-1", "text":"# local", "deleted":false
                }]))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(document["schemaVersion"], 6);
        assert_eq!(document["localNotes"][0]["text"], "# local");
        assert!(document.pointer("/tables/assignments").is_none());
        assert!(document.pointer("/tables/lms_items").is_none());
        assert_eq!(document["tables"]["plans"].as_array().unwrap().len(), 1);
        assert_eq!(
            document["tables"]["assignment_marks"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn backup_from_another_schema_is_rejected_by_its_version() {
        let store = store("backup-schema");
        let mut document: Value =
            serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
        document["schemaVersion"] = json!(5);
        // Older backups also carried localNotes in a different shape.
        document["localNotes"] = json!({ "notes": [] });
        assert_eq!(
            parse_import(&document.to_string()).unwrap_err().to_string(),
            "the backup uses schema version 5; this app reads version 6"
        );
        let _ = fs::remove_file(store.path());
    }
}
