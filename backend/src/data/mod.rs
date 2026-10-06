pub mod migrations;
pub mod versioned;

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
const USER_BACKUP_TABLES: &[&str] = &[
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

const USER_BACKUP_SETTINGS: &[&str] = &["userPreferences", "academicProfile"];

fn merge_object(target: &mut Value, source: Value) {
    let (Some(target), Value::Object(source)) = (target.as_object_mut(), source) else {
        return;
    };
    for (key, value) in source {
        if value.is_null() {
            continue;
        }
        if value.is_object() && target.get(&key).is_some_and(Value::is_object) {
            merge_object(target.get_mut(&key).expect("checked"), value);
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
            "assignments": settings.pointer("/notifications/assignments")
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
        "subscriptionRefresh": settings.get("subscriptionRefresh"),
        "notifications": {"localPermission": settings.pointer("/notifications/localPermission")}
    });
    let cloud = settings.get("cloud").cloned().unwrap_or_else(|| json!({}));
    (user, device, cloud)
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
        12 | 1 => "4Q",
        10 | 11 => "3Q",
        6..=9 => "2Q",
        _ => "1Q",
    }
}

fn empty_workspace() -> Value {
    let date = Utc::now().date_naive();
    let year = if date.month() == 1 {
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
                "frequency": "launch",
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
            "message": "Choose a provider and add its OAuth client ID."
        }
    });
    merge_object(&mut workspace["settings"], json!({
        "restoreSchoolPage": false,
        "schoolServiceWidth": 168,
        "schoolWebZoom": 1,
        "showGrades": false,
        "siteConsents": {"portal": false, "lms": false, "syllabus": false},
        "subscriptionRefresh": {"frequency": "daily", "allowCellular": false}
    }));
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

fn content_value(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for key in ["rev", "updatedAt", "updatedBy", "deletedAt"] {
            object.remove(key);
        }
    }
    value
}

fn string_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("record is missing {key}"))
}

fn validate_contract<T: DeserializeOwned>(table: &str, value: &Value) -> Result<(), String> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|error| format!("{table} validation failed: {error}"))
}

fn validate_user_record(table: &str, value: &Value) -> Result<(), String> {
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

fn row_version(value: &Value) -> Result<VersionedValue, String> {
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

fn read_rows(connection: &Connection, table: &str) -> Result<Vec<Value>, String> {
    let sql = format!("SELECT value FROM {table} ORDER BY id");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .map(|row| {
            let raw = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&raw).map_err(|_| format!("{table} contains invalid JSON"))
        })
        .collect();
    rows
}

fn read_visible_rows(connection: &Connection, table: &str) -> Result<Vec<Value>, String> {
    if !SCHOOL_CACHE_TABLES.contains(&table) {
        return read_rows(connection, table);
    }
    let sql = format!("SELECT value FROM {table} WHERE missing_since IS NULL ORDER BY id");
    let mut statement = connection.prepare(&sql).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .map(|row| {
            let raw = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&raw).map_err(|_| format!("{table} contains invalid JSON"))
        })
        .collect();
    rows
}

fn read_stored_row(
    transaction: &Transaction<'_>,
    table: &str,
    id: &str,
) -> Result<Option<(String, u64, String, String, Option<String>, Option<String>)>, String> {
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
        .map_err(|error| error.to_string())
}

fn upsert_row(
    transaction: &Transaction<'_>,
    table: &str,
    value: &Value,
    force: bool,
) -> Result<bool, String> {
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
        if stored_hash == version.hash && stored_deleted == version.deleted_at {
            return Ok(false);
        }
    }
    let raw = serde_json::to_string(value).map_err(|error| error.to_string())?;
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
        .map_err(|error| error.to_string())?;
    Ok(true)
}

fn read_setting(connection: &Connection, key: &str) -> Result<Option<Value>, String> {
    let raw: Option<String> = connection
        .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| error.to_string())?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|_| "settings contain invalid JSON".to_string())
    })
    .transpose()
}

fn read_settings_export(connection: &Connection) -> Result<Vec<Value>, String> {
    let mut statement = connection
        .prepare(
            "SELECT key, value, hash, updated_at, updated_by, rev, source, deleted_at
             FROM settings ORDER BY key",
        )
        .map_err(|error| error.to_string())?;
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
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn setting_version(row: &Value) -> Result<VersionedValue, String> {
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

fn import_setting(
    transaction: &Transaction<'_>,
    row: &Value,
) -> Result<(), String> {
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
                serde_json::to_string(&incoming.value).map_err(|error| error.to_string())?,
                incoming.hash,
                incoming.updated_at,
                incoming.updated_by,
                incoming.rev as i64,
                incoming.source,
                incoming.deleted_at
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_setting(
    transaction: &Transaction<'_>,
    key: &str,
    value: &Value,
    device_id: &str,
) -> Result<bool, String> {
    let hash = content_hash(value)?;
    let existing: Option<(String, i64)> = transaction
        .query_row(
            "SELECT hash, rev FROM settings WHERE key=?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if existing.as_ref().is_some_and(|(stored, _)| stored == &hash) {
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
                serde_json::to_string(value).map_err(|error| error.to_string())?,
                hash,
                now(),
                device_id,
                rev
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(true)
}

impl DataStore {
    pub fn initialize(path: PathBuf) -> Result<(Self, bool), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut connection = Connection::open(&path).map_err(|error| error.to_string())?;
        let rebuilt = migrations::migrate(&mut connection)?;
        // Catalog offerings are never deleted by the person; earlier workspace
        // saves tombstoned rows the window had not loaded yet. Restore them.
        connection
            .execute(
                "UPDATE offerings SET deleted_at=NULL, value=json_remove(value,'$.deletedAt')
                 WHERE deleted_at IS NOT NULL",
                [],
            )
            .map_err(|error| error.to_string())?;
        let store = Self { path };
        store.ensure_defaults()?;
        Ok((store, rebuilt))
    }

    fn open(&self) -> Result<Connection, String> {
        let mut connection = Connection::open(&self.path).map_err(|error| error.to_string())?;
        migrations::migrate(&mut connection)?;
        Ok(connection)
    }

    fn ensure_defaults(&self) -> Result<(), String> {
        let mut connection = self.open()?;
        let existing_device_id = read_setting(&connection, "deviceId")?;
        let workspace = empty_workspace();
        let device_id = existing_device_id
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or(string_field(&workspace, "deviceId")?);
        let transaction = connection.transaction().map_err(|error| error.to_string())?;
        if existing_device_id.is_none() {
            for key in ["deviceId", "selectedTerm", "terms", "academicProfile", "importStatus", "syncStatus", "logs"] {
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
        transaction.commit().map_err(|error| error.to_string())
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
        mut value: Value,
        actor: &str,
    ) -> Result<Value, String> {
        if !Self::user_table(table) {
            return Err("table is not user-owned".into());
        }
        let object = value
            .as_object_mut()
            .ok_or_else(|| "record must be an object".to_string())?;
        let requested_id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let source_key = object.get("sourceKey").and_then(Value::as_str).map(str::to_string);
        let target_key = object.get("targetKey").and_then(Value::as_str).map(str::to_string);
        let overlay_id = if matches!(table, "assignment_marks" | "course_overrides") {
            source_key.as_deref().map(|source_key| {
                transaction.query_row(
                    &format!("SELECT id FROM {table} WHERE source_key=?1"),
                    [source_key],
                    |row| row.get::<_, String>(0),
                ).optional().map_err(|error| error.to_string())
            }).transpose()?.flatten()
        } else if table == "schedule_overrides" {
            target_key.as_deref().map(|target_key| {
                transaction.query_row(
                    "SELECT id FROM schedule_overrides WHERE target_key=?1",
                    [target_key],
                    |row| row.get::<_, String>(0),
                ).optional().map_err(|error| error.to_string())
            }).transpose()?.flatten()
        } else { None };
        let id = requested_id.or(overlay_id).unwrap_or_else(|| Uuid::now_v7().to_string());
        let previous: Option<(String, i64)> = transaction
            .query_row(
                &format!("SELECT value, rev FROM {table} WHERE id=?1"),
                [&id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let revision = previous.as_ref().map_or(1, |(_, rev)| rev + 1);
        object.insert("id".into(), Value::String(id.clone()));
        object.insert("rev".into(), Value::from(revision));
        object.insert("updatedAt".into(), Value::String(now()));
        object.insert("updatedBy".into(), Value::String(actor.to_string()));
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
        upsert_row(&transaction, table, &value, true)?;
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
                ).map_err(|error| error.to_string())?;
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
                ).map_err(|error| error.to_string())?;
            }
            "schedule_overrides" => {
                transaction.execute(
                    "UPDATE schedule_overrides SET target_key=?2 WHERE id=?1",
                    params![id, value.get("targetKey").and_then(Value::as_str)],
                ).map_err(|error| error.to_string())?;
            }
            _ => {}
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(value)
    }

    pub fn delete_user_record(&self, table: &str, id: &str, actor: &str) -> Result<bool, String> {
        if !Self::user_table(table) {
            return Err("table is not user-owned".into());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let raw: Option<(String, i64)> = transaction
            .query_row(
                &format!("SELECT value, rev FROM {table} WHERE id=?1"),
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((raw, revision)) = raw else { return Ok(false) };
        let mut value: Value = serde_json::from_str(&raw)
            .map_err(|_| format!("{table} contains invalid JSON"))?;
        let timestamp = now();
        let object = value
            .as_object_mut()
            .ok_or_else(|| format!("{table} contains a non-object record"))?;
        object.insert("rev".into(), Value::from(revision + 1));
        object.insert("updatedAt".into(), Value::String(timestamp.clone()));
        object.insert("updatedBy".into(), Value::String(actor.to_string()));
        object.insert("deletedAt".into(), Value::String(timestamp));
        upsert_row(&transaction, table, &value, true)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub fn user_rows(&self, table: &str) -> Result<Vec<Value>, String> {
        if !Self::user_table(table) {
            return Err("table is not user-owned".into());
        }
        read_rows(&self.open()?, table)
    }

    pub fn agent_access_enabled(&self) -> Result<bool, String> {
        let connection = self.open()?;
        Ok(read_setting(&connection, "devicePreferences")?
            .or(read_setting(&connection, "settings")?)
            .and_then(|value| value.pointer("/agentAccess/enabled").and_then(Value::as_bool))
            .unwrap_or(false))
    }

    pub fn record_agent_write(&self, tool: &str) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let key = if read_setting(&transaction, "devicePreferences")?.is_some() {
            "devicePreferences"
        } else {
            "settings"
        };
        let mut settings = read_setting(&transaction, key)?.unwrap_or_else(|| json!({}));
        let object = settings
            .as_object_mut()
            .ok_or_else(|| "settings must be an object".to_string())?;
        let access = object
            .entry("agentAccess")
            .or_insert_with(|| json!({"enabled":false,"lastWrites":[]}));
        let writes = access
            .as_object_mut()
            .and_then(|access| access.entry("lastWrites").or_insert_with(|| json!([])).as_array_mut())
            .ok_or_else(|| "agent access history must be an array".to_string())?;
        writes.insert(0, json!({"at": now(), "tool": tool}));
        writes.truncate(20);
        save_setting(&transaction, key, &settings, "mcp")?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn put_workspace_setting(&self, key: &str, value: Value, actor: &str) -> Result<(), String> {
        if !matches!(key, "selectedTerm" | "terms" | "academicProfile" | "userPreferences" | "devicePreferences" | "cloud" | "importStatus" | "syncStatus" | "logs") {
            return Err("setting key is not writable".into());
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        save_setting(&transaction, key, &value, actor)?;
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn load_workspace(&self) -> Result<Value, String> {
        let connection = self.open()?;
        let mut workspace = empty_workspace();
        for (field, table) in COLLECTIONS {
            workspace[*field] = Value::Array(read_visible_rows(&connection, table)?);
        }
        if let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) {
            courses.extend(read_rows(&connection, "manual_courses")?);
        }
        if let Some(memberships) = workspace.get_mut("memberships").and_then(Value::as_array_mut) {
            memberships.extend(read_rows(&connection, "manual_memberships")?);
        }
        let marks = read_rows(&connection, "assignment_marks")?;
        if let Some(assignments) = workspace
            .get_mut("assignments")
            .and_then(Value::as_array_mut)
        {
            for assignment in assignments {
                let Some(id) = assignment.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let source_key = assignment.get("sourceKey").and_then(Value::as_str).unwrap_or(id);
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
                    if let Some(completed) = mark.get("completedAt").and_then(Value::as_str) {
                        object.insert("completedAt".into(), Value::String(completed.to_string()));
                    }
                }
            }
        }
        let overrides = read_rows(&connection, "course_overrides")?;
        if let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) {
            for course in courses {
                let Some(id) = course.get("id").and_then(Value::as_str) else { continue };
                let source_key = course.get("sourceKey").and_then(Value::as_str).unwrap_or(id);
                if let Some(overlay) = overrides.iter().find(|row| {
                    row.get("deletedAt").is_none()
                        && (row.get("sourceKey").and_then(Value::as_str) == Some(source_key)
                            || row.get("courseId").and_then(Value::as_str) == Some(id))
                }) {
                    if let Some(object) = course.as_object_mut() {
                        object.insert("overrides".into(), json!({
                            "title": overlay.get("title"),
                            "instructor": overlay.get("instructor"),
                            "color": overlay.get("color")
                        }));
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
            if let Some(value) = read_setting(&connection, key)? {
                workspace[key] = value;
            }
        }
        for key in ["userPreferences", "devicePreferences"] {
            if let Some(value) = read_setting(&connection, key)? {
                merge_object(&mut workspace["settings"], value);
            }
        }
        if let Some(value) = read_setting(&connection, "cloud")? {
            workspace["settings"]["cloud"] = value;
        }
        normalize_optional_nulls(&mut workspace);
        let _: crate::contracts::Workspace = serde_json::from_value(workspace.clone())
            .map_err(|error| format!("workspace validation failed: {error}"))?;
        Ok(workspace)
    }

    pub fn reset_workspace(&self) -> Result<Value, String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
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
            ])
            .collect::<HashSet<_>>();
        for table in reset_tables {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute("DELETE FROM settings", [])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        self.ensure_defaults()?;
        self.load_workspace()
    }

    pub fn export_data(&self, notes: Value) -> Result<String, String> {
        let connection = self.open()?;
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
        tables.insert(
            "settings".into(),
            Value::Array(settings),
        );

        serde_json::to_string_pretty(&json!({
            "schemaVersion": 4,
            "exportedAt": now(),
            "kind": "campus-life-local-backup",
            "deviceId": "local-backup",
            "tables": tables,
            "localNotes": notes
        }))
        .map_err(|error| error.to_string())
    }

    pub fn import_data(&self, document: &str) -> Result<Value, String> {
        let parsed: Value =
            serde_json::from_str(document).map_err(|_| "import is not valid JSON".to_string())?;
        if parsed.get("schemaVersion").and_then(Value::as_u64) != Some(4)
            || parsed.get("kind").and_then(Value::as_str)
                != Some("campus-life-local-backup")
        {
            return Err("import must be a version 4 local backup".into());
        }
        let exported_at = string_field(&parsed, "exportedAt")?;
        DateTime::parse_from_rfc3339(exported_at)
            .map_err(|_| "import exportedAt must be RFC 3339".to_string())?;
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
                return Err(format!("import table {table} must be an array"));
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

        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let allowed: HashSet<_> = USER_BACKUP_TABLES
            .iter()
            .copied()
            .collect();
        // A local backup restore is an explicit replacement, not sync/merge.
        for table in USER_BACKUP_TABLES {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(|error| error.to_string())?;
        }
        for (table, rows) in tables {
            if table == "settings" {
                continue;
            }
            if !allowed.contains(table.as_str()) {
                return Err(format!("import contains unsupported table {table}"));
            }
            for incoming in rows.as_array().expect("validated") {
                validate_user_record(table, incoming)?;
                upsert_row(&transaction, table, incoming, true)?;
            }
        }
        if let Some(settings) = tables.get("settings").and_then(Value::as_array) {
            for key in USER_BACKUP_SETTINGS {
                transaction
                    .execute("DELETE FROM settings WHERE key=?1", [key])
                    .map_err(|error| error.to_string())?;
            }
            for row in settings {
                let key = string_field(row, "key")?;
                if !USER_BACKUP_SETTINGS.contains(&key) {
                    return Err(format!("import contains device or secret setting {key}"));
                }
                import_setting(&transaction, row)?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        self.load_workspace()
    }

    pub fn catalog_upsert_offerings(&self, offerings: &[Value]) -> Result<usize, String> {
        for offering in offerings {
            string_field(offering, "id")?;
        }
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let mut changed = 0;
        for offering in offerings {
            let mut offering = offering.clone();
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
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "INSERT INTO offerings_fts
                     (offering_id, code, title_ja, title_en, instructors_ja, instructors_en)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![id, code, title_ja, title_en, instructors_ja, instructors_en],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(changed)
    }

    /// Search rows are deliberately stored apart from complete offerings.
    /// A partial result can therefore never masquerade as a persisted entity.
    pub fn catalog_upsert_search_items(&self, items: &[Value]) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(|error| error.to_string())?;
        for item in items {
            validate_contract::<crate::contracts::CatalogSearchItem>("catalog_search_items", item)?;
            let id = string_field(item, "id")?;
            let code = item.get("code").and_then(Value::as_str).unwrap_or_default();
            let title_ja = item.pointer("/title/ja").and_then(Value::as_str).unwrap_or_default();
            let title_en = item.pointer("/title/en").and_then(Value::as_str).unwrap_or_default();
            let instructors_ja = item.get("instructors").and_then(Value::as_array).into_iter().flatten()
                .filter_map(|name| name.get("ja").and_then(Value::as_str)).collect::<Vec<_>>().join(" ");
            let instructors_en = item.get("instructors").and_then(Value::as_array).into_iter().flatten()
                .filter_map(|name| name.get("en").and_then(Value::as_str)).collect::<Vec<_>>().join(" ");
            transaction.execute(
                "INSERT INTO catalog_search_items(id,value,updated_at) VALUES(?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
                params![id, serde_json::to_string(item).map_err(|error| error.to_string())?, now()],
            ).map_err(|error| error.to_string())?;
            transaction.execute("DELETE FROM offerings_fts WHERE offering_id=?1", [id])
                .map_err(|error| error.to_string())?;
            transaction.execute(
                "INSERT INTO offerings_fts(offering_id,code,title_ja,title_en,instructors_ja,instructors_en)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                params![id,code,title_ja,title_en,instructors_ja,instructors_en],
            ).map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn upsert_offering_content(&self, rows: &[Value]) -> Result<(), String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
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
                .unwrap_or_else(|| "");
            transaction
                .execute(
                    "INSERT INTO offering_content(offering_id, lang, content, fetched_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(offering_id, lang) DO UPDATE SET
                     content=excluded.content, fetched_at=excluded.fetched_at",
                    params![
                        offering_id,
                        lang,
                        serde_json::to_string(content).map_err(|error| error.to_string())?,
                        fetched_at
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn catalog_search(&self, query: &str) -> Result<Vec<Value>, String> {
        let connection = self.open()?;
        if query.trim().is_empty() {
            return read_rows(&connection, "offerings");
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
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([expression], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .map(|row| {
                serde_json::from_str(&row.map_err(|error| error.to_string())?)
                    .map_err(|_| "offering contains invalid JSON".to_string())
            })
            .collect();
        rows
    }

    pub fn catalog_offering(&self, id: &str) -> Result<Option<Value>, String> {
        let connection = self.open()?;
        let raw: Option<String> = connection
            .query_row(
                "SELECT value FROM offerings WHERE id=?1 AND deleted_at IS NULL AND missing_since IS NULL",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        raw.map(|raw| {
            serde_json::from_str(&raw).map_err(|_| "offering contains invalid JSON".to_string())
        })
        .transpose()
    }

    pub fn table_rows(&self, table: &str) -> Result<Vec<Value>, String> {
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
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .map(|row| {
                    serde_json::from_str(&row.map_err(|error| error.to_string())?)
                        .map_err(|_| "offering content is invalid JSON".to_string())
                })
                .collect();
            return rows;
        }
        read_visible_rows(&connection, table)
    }

    /// When each importer (`lms`, `kyomu` per page kind) last finished a
    /// complete run: the cache age the app compares with its TTLs.
    pub fn last_import_runs(&self) -> Result<Value, String> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT provider, scope, MAX(finished_at) FROM import_runs
                 WHERE state='complete' AND finished_at IS NOT NULL GROUP BY provider, scope",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut runs = Map::new();
        for row in rows {
            let (provider, scope, finished) = row.map_err(|error| error.to_string())?;
            runs.insert(format!("{provider}:{scope}"), Value::String(finished));
        }
        Ok(Value::Object(runs))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn store(name: &str) -> DataStore {
        let path = std::env::temp_dir().join(format!("campus-life-{name}-{}.db", Uuid::new_v4()));
        DataStore::initialize(path).unwrap().0
    }

    #[test]
    fn installs_reference_refresh_as_a_user_controllable_scheduled_task() {
        let store = store("reference-refresh-task");
        let workspace = store.load_workspace().unwrap();
        let tasks = workspace["scheduledTasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["id"], "builtin:reference-subscriptions");
        assert_eq!(tasks[0]["action"], "reference-refresh");
        assert_eq!(tasks[0]["enabled"], true);

        store
            .delete_user_record(
                "scheduled_tasks",
                "builtin:reference-subscriptions",
                "test",
            )
            .unwrap();
        store.ensure_defaults().unwrap();
        let workspace = store.load_workspace().unwrap();
        let tasks = workspace["scheduledTasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks[0]["deletedAt"].is_string());
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
    fn explicit_delete_is_a_user_tombstone() {
        let store = store("explicit-delete");
        let row = store.put_user_record("plans", json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}), "test").unwrap();
        assert!(store.delete_user_record("plans", row["id"].as_str().unwrap(), "test").unwrap());
        assert!(store.user_rows("plans").unwrap()[0]["deletedAt"].is_string());
        let _ = fs::remove_file(store.path());
    }

    fn insert_school_assignment(store: &DataStore) {
        let mut connection = store.open().unwrap();
        let transaction = connection.transaction().unwrap();
        upsert_row(&transaction, "assignments", &json!({
            "id":"assignment-1", "kind":"assignment", "courseId":"course-1",
            "title":"Work", "details":"", "dueAt":null,
            "sourceKey":"lms:default:assignment:42", "source":{"provider":"lms"},
            "rev":1, "updatedAt":"2026-10-06T00:00:00Z", "updatedBy":"lms"
        }), true).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn overlay_survives_school_disappearance_and_reappearance() {
        let store = store("overlay-lifetime");
        insert_school_assignment(&store);
        store.put_user_record("assignment_marks", json!({
            "sourceKey":"lms:default:assignment:42", "hidden":true
        }), "test").unwrap();
        assert_eq!(store.load_workspace().unwrap()["assignments"][0]["hidden"], true);

        let connection = store.open().unwrap();
        connection.execute(
            "UPDATE assignments SET missing_since='2026-10-07T00:00:00Z',value=json_set(value,'$.missingSince','2026-10-07T00:00:00Z') WHERE id='assignment-1'",
            [],
        ).unwrap();
        assert_eq!(store.load_workspace().unwrap()["assignments"].as_array().unwrap().len(), 0);
        assert_eq!(store.user_rows("assignment_marks").unwrap().len(), 1);

        connection.execute(
            "UPDATE assignments SET missing_since=NULL,value=json_remove(value,'$.missingSince') WHERE id='assignment-1'",
            [],
        ).unwrap();
        assert_eq!(store.load_workspace().unwrap()["assignments"][0]["hidden"], true);
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn local_backup_contains_user_data_overlays_and_notes_but_no_school_cache() {
        let store = store("backup-boundary");
        insert_school_assignment(&store);
        store.put_user_record("plans", json!({
            "title":"Study", "start":"2026-10-06T10:00:00+09:00", "duration":60, "exdates":[]
        }), "test").unwrap();
        store.put_user_record("assignment_marks", json!({
            "sourceKey":"lms:default:assignment:42", "hidden":true
        }), "test").unwrap();
        let document: Value = serde_json::from_str(&store.export_data(json!([{
            "id":"018f0000-0000-7000-8000-000000000001", "term":"2026-3Q",
            "course":"TEST", "section":"section-1", "text":"# local", "deleted":false
        }])).unwrap()).unwrap();
        assert_eq!(document["schemaVersion"], 4);
        assert_eq!(document["localNotes"][0]["text"], "# local");
        assert!(document.pointer("/tables/assignments").is_none());
        assert!(document.pointer("/tables/lms_items").is_none());
        assert_eq!(document["tables"]["plans"].as_array().unwrap().len(), 1);
        assert_eq!(document["tables"]["assignment_marks"].as_array().unwrap().len(), 1);
        let _ = fs::remove_file(store.path());
    }
}
