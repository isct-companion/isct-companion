pub mod migrations;
pub mod versioned;

use chrono::{DateTime, Datelike, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use versioned::{content_hash, update, MergeMethod, VersionedValue};

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
    ("subscriptions", "subscriptions"),
    ("scheduledTasks", "scheduled_tasks"),
];

const SCHOOL_OWNED_TABLES: &[&str] = &["assignments", "lms_items", "gpa_summary", "offerings"];

const EXPORTED_TABLES: &[&str] = &[
    "timetable_courses",
    "course_overrides",
    "plan_items",
    "memberships",
    "personal_events",
    "calendar_exceptions",
    "assignment_marks",
    "intensive_schedules",
    "plans",
    "subscriptions",
    "scheduled_tasks",
    "registrations",
    "registration_states",
    "grades",
    "requirement_progress",
    "lms_courses",
    "lms_items",
    "assignments",
    "gpa_summary",
    "gpa_by_term",
    "academic_calendar",
    "departments",
    "recommended_courses",
];

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
    json!({
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

fn save_collection(
    transaction: &Transaction<'_>,
    table: &str,
    rows: &[Value],
    updated_by: &str,
) -> Result<usize, String> {
    let mut changed = 0;
    let mut incoming = HashSet::new();
    for row in rows {
        incoming.insert(string_field(row, "id")?.to_string());
        changed += usize::from(upsert_row(transaction, table, row, false)?);
    }
    let active_ids = {
        let sql = format!("SELECT id, value, rev FROM {table} WHERE deleted_at IS NULL");
        let mut statement = transaction
            .prepare(&sql)
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    // Rows imported from LMS/教務Web are owned by the importers; a workspace
    // save that has not reloaded them yet must not tombstone them.
    if SCHOOL_OWNED_TABLES.contains(&table) {
        return Ok(changed);
    }
    for (id, raw, rev) in active_ids {
        if incoming.contains(&id) {
            continue;
        }
        let mut value: Value =
            serde_json::from_str(&raw).map_err(|_| format!("{table} contains invalid JSON"))?;
        let timestamp = now();
        if let Some(object) = value.as_object_mut() {
            object.insert("rev".into(), Value::from(rev + 1));
            object.insert("updatedAt".into(), Value::String(timestamp.clone()));
            object.insert("updatedBy".into(), Value::String(updated_by.to_string()));
            object.insert("deletedAt".into(), Value::String(timestamp));
        }
        changed += usize::from(upsert_row(transaction, table, &value, true)?);
    }
    Ok(changed)
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
    method: MergeMethod,
) -> Result<(), String> {
    let key = string_field(row, "key")?;
    let incoming = setting_version(row)?;
    let current: Option<(String, String, String, i64, Option<String>, Option<String>)> =
        transaction
            .query_row(
                "SELECT value, hash, updated_at, rev, source, deleted_at
                 FROM settings WHERE key=?1",
                [key],
                |stored| {
                    Ok((
                        stored.get(0)?,
                        stored.get(1)?,
                        stored.get(2)?,
                        stored.get(3)?,
                        stored.get(4)?,
                        stored.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
    if let Some((raw, hash, updated_at, rev, source, deleted_at)) = current {
        let current = VersionedValue {
            value: serde_json::from_str(&raw)
                .map_err(|_| "stored setting is invalid JSON".to_string())?,
            hash,
            updated_at,
            updated_by: "local".into(),
            rev: rev as u64,
            source,
            deleted_at,
        };
        let merged = update(&current, &incoming, method);
        if method == MergeMethod::ResolveManually && !merged.conflict.is_empty() {
            return Err(serde_json::to_string(&json!({
                "kind": "merge-conflict",
                "table": "settings",
                "id": key,
                "changes": merged.conflict
            }))
            .map_err(|error| error.to_string())?);
        }
        if !merged.changed {
            return Ok(());
        }
    }
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
        Ok((Self { path }, rebuilt))
    }

    fn open(&self) -> Result<Connection, String> {
        let mut connection = Connection::open(&self.path).map_err(|error| error.to_string())?;
        migrations::migrate(&mut connection)?;
        Ok(connection)
    }

    pub fn load_workspace(&self) -> Result<Value, String> {
        let connection = self.open()?;
        let mut workspace = empty_workspace();
        for (field, table) in COLLECTIONS {
            workspace[*field] = Value::Array(read_rows(&connection, table)?);
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
                let Some(mark) = marks.iter().find(|mark| {
                    mark.get("assignmentId").and_then(Value::as_str) == Some(id)
                        && mark.get("deletedAt").is_none()
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
        Ok(workspace)
    }

    pub fn save_workspace(&self, workspace: &Value) -> Result<usize, String> {
        if workspace.get("schemaVersion").and_then(Value::as_u64) != Some(3) {
            return Err("workspace schema must be version 3".into());
        }
        let device_id = string_field(workspace, "deviceId")?;
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut changed = 0;
        for (field, table) in COLLECTIONS {
            let source_rows = workspace
                .get(*field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("workspace {field} must be an array"))?;
            if *field == "assignments" {
                let assignments = source_rows
                    .iter()
                    .map(|row| {
                        let mut row = row.clone();
                        if let Some(object) = row.as_object_mut() {
                            object.remove("hidden");
                            object.remove("completedAt");
                        }
                        row
                    })
                    .collect::<Vec<_>>();
                changed += save_collection(&transaction, table, &assignments, device_id)?;
                let marks = source_rows
                    .iter()
                    .filter(|row| {
                        row.get("hidden").and_then(Value::as_bool) == Some(true)
                            || row.get("completedAt").is_some()
                    })
                    .map(|row| {
                        json!({
                            "id": format!("assignment-mark:{}", row.get("id").and_then(Value::as_str).unwrap_or_default()),
                            "assignmentId": row.get("id"),
                            "hidden": row.get("hidden").and_then(Value::as_bool).unwrap_or(false),
                            "completedAt": row.get("completedAt"),
                            "rev": row.get("rev"),
                            "updatedAt": row.get("updatedAt"),
                            "updatedBy": row.get("updatedBy"),
                            "source": {"provider": "manual"}
                        })
                    })
                    .collect::<Vec<_>>();
                changed += save_collection(&transaction, "assignment_marks", &marks, device_id)?;
            } else {
                changed += save_collection(&transaction, table, source_rows, device_id)?;
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
            if let Some(value) = workspace.get(key) {
                changed += usize::from(save_setting(&transaction, key, value, device_id)?);
            }
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(changed)
    }

    pub fn reset_workspace(&self) -> Result<Value, String> {
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let reset_tables = COLLECTIONS
            .iter()
            .map(|(_, table)| *table)
            .chain(EXPORTED_TABLES.iter().copied())
            .chain(["offering_content", "offerings_fts"])
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
        let workspace = empty_workspace();
        self.save_workspace(&workspace)?;
        Ok(workspace)
    }

    pub fn export_data(&self) -> Result<String, String> {
        let connection = self.open()?;
        let workspace = self.load_workspace()?;
        let mut tables = Map::new();
        for table in EXPORTED_TABLES {
            tables.insert(
                (*table).into(),
                Value::Array(read_rows(&connection, table)?),
            );
        }
        tables.insert(
            "settings".into(),
            Value::Array(read_settings_export(&connection)?),
        );

        let referenced: HashSet<String> = tables
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(|row| {
                row.get("offeringId")
                    .or_else(|| row.get("offering_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
        let offerings = read_rows(&connection, "offerings")?
            .into_iter()
            .filter(|row| {
                row.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| referenced.contains(id))
            })
            .collect();
        tables.insert("offerings".into(), Value::Array(offerings));

        serde_json::to_string_pretty(&json!({
            "schemaVersion": 3,
            "exportedAt": now(),
            "deviceId": workspace["deviceId"],
            "tables": tables
        }))
        .map_err(|error| error.to_string())
    }

    pub fn import_data(&self, document: &str) -> Result<Value, String> {
        let parsed: Value =
            serde_json::from_str(document).map_err(|_| "import is not valid JSON".to_string())?;
        if parsed.get("schemaVersion").and_then(Value::as_u64) != Some(3) {
            return Err("import schema must be version 3".into());
        }
        string_field(&parsed, "deviceId")?;
        let exported_at = string_field(&parsed, "exportedAt")?;
        DateTime::parse_from_rfc3339(exported_at)
            .map_err(|_| "import exportedAt must be RFC 3339".to_string())?;
        let tables = parsed
            .get("tables")
            .and_then(Value::as_object)
            .ok_or_else(|| "import tables must be an object".to_string())?;
        let required: HashSet<_> = EXPORTED_TABLES
            .iter()
            .copied()
            .chain(["offerings", "settings"])
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

        let current = self.load_workspace()?;
        let method = match current
            .pointer("/settings/cloud/conflict")
            .and_then(Value::as_str)
        {
            Some("keep-current") => MergeMethod::UseCurrent,
            Some("use-latest") => MergeMethod::UseLatest,
            _ => MergeMethod::ResolveManually,
        };
        let mut connection = self.open()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let allowed: HashSet<_> = EXPORTED_TABLES
            .iter()
            .copied()
            .chain(["offerings"])
            .collect();
        for (table, rows) in tables {
            if table == "settings" {
                continue;
            }
            if !allowed.contains(table.as_str()) {
                return Err(format!("import contains unsupported table {table}"));
            }
            for incoming in rows.as_array().expect("validated") {
                let id = string_field(incoming, "id")?;
                let sql = format!("SELECT value FROM {table} WHERE id=?1");
                let existing: Option<String> = transaction
                    .query_row(&sql, [id], |row| row.get(0))
                    .optional()
                    .map_err(|error| error.to_string())?;
                let Some(existing) = existing else {
                    upsert_row(&transaction, table, incoming, true)?;
                    continue;
                };
                let existing: Value = serde_json::from_str(&existing)
                    .map_err(|_| format!("{table} contains invalid JSON"))?;
                let current_version = row_version(&existing)?;
                let incoming_version = row_version(incoming)?;
                let merged = update(&current_version, &incoming_version, method);
                if method == MergeMethod::ResolveManually && !merged.conflict.is_empty() {
                    return Err(serde_json::to_string(&json!({
                        "kind": "merge-conflict",
                        "table": table,
                        "id": id,
                        "changes": merged.conflict
                    }))
                    .map_err(|error| error.to_string())?);
                }
                if merged.changed {
                    upsert_row(&transaction, table, incoming, true)?;
                }
            }
        }
        if let Some(settings) = tables.get("settings").and_then(Value::as_array) {
            for row in settings {
                import_setting(&transaction, row, method)?;
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
                "SELECT o.value FROM offerings_fts f
                 JOIN offerings o ON o.id=f.offering_id
                 WHERE offerings_fts MATCH ?1 AND o.deleted_at IS NULL
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
                "SELECT value FROM offerings WHERE id=?1 AND deleted_at IS NULL",
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
        read_rows(&connection, table)
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
    fn save_waits_for_an_import_writer_without_requiring_manual_retry() {
        let store = store("concurrent-save");
        let mut workspace = empty_workspace();
        store.save_workspace(&workspace).unwrap();
        let path = store.path().to_path_buf();
        let (ready, receive) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            let mut connection = Connection::open(path).unwrap();
            let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            tx.execute("UPDATE settings SET updated_at=updated_at", []).unwrap();
            ready.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(150));
            tx.commit().unwrap();
        });
        receive.recv().unwrap();
        workspace["settings"]["locale"] = json!("ja");
        let result = store.save_workspace(&workspace);
        writer.join().unwrap();
        assert!(result.is_ok(), "Concurrent save failed: {result:?}");
        assert_eq!(store.load_workspace().unwrap()["settings"]["locale"], "ja");
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn save_only_writes_changed_revisions_and_tombstones_removed_rows() {
        let store = store("save");
        let mut workspace = empty_workspace();
        let timestamp = now();
        workspace["courses"] = json!([{
            "id": "course-1", "kind": "course", "rev": 1,
            "updatedAt": timestamp, "updatedBy": "device", "termId": "2026-1Q",
            "code": "TEST.1", "title": "Test", "instructor": "Teacher",
            "credits": 1, "creditSplit": {"lecture": 1, "exercise": 0, "lab": 0},
            "instructionLanguage": "en", "color": "#000", "meetings": [],
            "source": {"provider": "manual"}
        }]);
        assert!(store.save_workspace(&workspace).unwrap() > 0);
        assert_eq!(store.save_workspace(&workspace).unwrap(), 0);
        workspace["courses"] = json!([]);
        assert_eq!(store.save_workspace(&workspace).unwrap(), 1);
        let loaded = store.load_workspace().unwrap();
        assert!(loaded["courses"][0]["deletedAt"].is_string());
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn export_excludes_caches_and_unreferenced_offerings() {
        let store = store("export");
        let workspace = empty_workspace();
        store.save_workspace(&workspace).unwrap();
        store
            .catalog_upsert_offerings(&[json!({
                "id": "unreferenced", "code": "TEST.1", "title": {"ja": "試験", "en": "Test"}
            })])
            .unwrap();
        let export: Value = serde_json::from_str(&store.export_data().unwrap()).unwrap();
        assert!(export.pointer("/tables/offering_content").is_none());
        assert!(export.pointer("/tables/offerings_fts").is_none());
        assert_eq!(export["tables"]["offerings"], json!([]));
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn fts_finds_both_titles() {
        let store = store("fts");
        store
            .catalog_upsert_offerings(&[json!({
                "id": "2026001", "code": "CSC.T001",
                "title": {"ja": "並列プログラミング", "en": "Parallel Programming"}
            })])
            .unwrap();
        assert_eq!(store.catalog_search("並列").unwrap().len(), 1);
        assert_eq!(store.catalog_search("Parallel").unwrap().len(), 1);
        let _ = fs::remove_file(store.path());
    }

    fn course(title: &str, rev: u64, updated_at: &str, deleted_at: Option<&str>) -> Value {
        let mut value = json!({
            "id": "course-1", "kind": "course", "rev": rev,
            "updatedAt": updated_at, "updatedBy": "device", "termId": "2026-1Q",
            "code": "TEST.1", "title": title, "instructor": "Teacher",
            "credits": 1, "creditSplit": {"lecture": 1, "exercise": 0, "lab": 0},
            "instructionLanguage": "en", "color": "#000", "meetings": [],
            "source": {"provider": "manual"}
        });
        if let Some(deleted_at) = deleted_at {
            value["deletedAt"] = Value::String(deleted_at.into());
        }
        value
    }

    #[test]
    fn import_uses_latest_and_merges_tombstones() {
        let store = store("import-latest");
        let mut workspace = empty_workspace();
        workspace["settings"]["cloud"]["conflict"] = Value::String("use-latest".into());
        workspace["courses"] = json!([course("Old", 1, "2026-01-01T00:00:00.000Z", None)]);
        store.save_workspace(&workspace).unwrap();

        let mut imported: Value = serde_json::from_str(&store.export_data().unwrap()).unwrap();
        imported["exportedAt"] = Value::String("2026-01-03T00:00:00.000Z".into());
        imported["deviceId"] = Value::String("remote".into());
        imported["tables"]["timetable_courses"] = json!([course(
            "New",
            2,
            "2026-01-02T00:00:00.000Z",
            Some("2026-01-02T00:00:00.000Z")
        )]);
        let loaded = store.import_data(&imported.to_string()).unwrap();
        assert_eq!(loaded["courses"][0]["title"], "New");
        assert_eq!(
            loaded["courses"][0]["deletedAt"],
            "2026-01-02T00:00:00.000Z"
        );
        let _ = fs::remove_file(store.path());
    }

    #[test]
    fn manual_import_reports_field_conflicts_without_writing() {
        let store = store("import-conflict");
        let mut workspace = empty_workspace();
        workspace["courses"] = json!([course("Current", 1, "2026-01-01T00:00:00.000Z", None)]);
        store.save_workspace(&workspace).unwrap();
        let mut imported: Value = serde_json::from_str(&store.export_data().unwrap()).unwrap();
        imported["exportedAt"] = Value::String("2026-01-03T00:00:00.000Z".into());
        imported["deviceId"] = Value::String("remote".into());
        imported["tables"]["timetable_courses"] =
            json!([course("Incoming", 2, "2026-01-02T00:00:00.000Z", None)]);
        let error = store.import_data(&imported.to_string()).unwrap_err();
        assert!(error.contains("merge-conflict"));
        assert!(error.contains("title"));
        assert_eq!(
            store.load_workspace().unwrap()["courses"][0]["title"],
            "Current"
        );
        let _ = fs::remove_file(store.path());
    }
}
