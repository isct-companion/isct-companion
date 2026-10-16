mod backup_store;
mod catalog_store;
mod defaults;
mod lms_store;
mod records;
mod settings;
mod snapshot;

pub use defaults::initial_workspace;
use defaults::*;
use records::*;
pub(crate) use records::{canonical_course_id, resolve_course_id};
pub(crate) use settings::ADMISSION_YEARS;
use settings::*;

pub mod cloud_merge;
mod error;
pub mod migrations;
pub mod versioned;

pub use error::DataError;

use crate::school_service::{SchoolPageAvailability, SchoolPageAvailabilityState};
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
    ("classSchedules", "class_schedules"),
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
    "class_schedules",
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

/// Core-owned bookkeeping that background work (scheduler, reference
/// refresh) updates inside user-owned rows. Shells never supply it, so it is
/// not part of the user-intent revision.
const BOOKKEEPING_FIELDS: &[(&str, &[&str])] = &[
    (
        "scheduled_tasks",
        &["lastOccurrenceAt", "lastRunAt", "lastResult"],
    ),
    (
        "subscriptions",
        &[
            "fetchedAt",
            "lastError",
            "indexHash",
            "remoteHash",
            "dataUrl",
            "sourceTs",
            "year",
        ],
    ),
];
pub(crate) fn bookkeeping_fields(table: &str) -> &'static [&'static str] {
    BOOKKEEPING_FIELDS
        .iter()
        .find(|(name, _)| *name == table)
        .map_or(&[], |(_, fields)| fields)
}

/// Settings rows a user intent reads as its baseline.
const INTENT_SETTINGS: &[&str] = &[
    "selectedTerm",
    "terms",
    "academicProfile",
    "classGroups",
    "settings",
    "userPreferences",
    "devicePreferences",
    "cloud",
    "importStatus",
];

/// Fingerprint of the user-owned state intents are checked against. School
/// and reference caches and Core bookkeeping are excluded, so a background
/// ingest does not turn a person's edit into a conflict.
fn intent_revision(connection: &Connection) -> Result<String, DataError> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for table in USER_BACKUP_TABLES.iter().chain(&["scheduled_tasks"]) {
        let bookkeeping = bookkeeping_fields(table);
        hasher.update(table.as_bytes());
        for row in read_rows(connection, table)? {
            let mut content = content_value(&row);
            if let Some(object) = content.as_object_mut() {
                // `source` is stamped on a row's first Core write; real edits
                // always change other content as well.
                for field in bookkeeping.iter().chain(&["source"]) {
                    object.remove(*field);
                }
            }
            hasher.update(serde_json::to_vec(&(content, row.get("deletedAt")))?);
        }
    }
    for key in INTENT_SETTINGS {
        let mut value = read_setting(connection, key)?.unwrap_or(Value::Null);
        if let Some(refresh) = value
            .pointer_mut("/subscriptionRefresh")
            .and_then(Value::as_object_mut)
        {
            refresh.remove("lastCheckedAt");
        }
        hasher.update(key.as_bytes());
        hasher.update(serde_json::to_vec(&value)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) const USER_BACKUP_SETTINGS: &[&str] =
    &["userPreferences", "academicProfile", "terms", "classGroups"];
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
    /// `(workspace, school records, user-intent revision)` as written so far.
    pub(crate) fn snapshot_parts(&self) -> Result<(Value, Value, String), DataError> {
        Ok((
            DataStore::load_workspace_from(self.transaction)?,
            DataStore::school_records_from(self.transaction)?,
            intent_revision(self.transaction)?,
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

    pub(crate) fn catalog_row(&self, id: &str) -> Result<Option<Value>, DataError> {
        catalog_row_in(self.transaction, id)
    }

    pub(crate) fn save_setting(&self, key: &str, value: &Value) -> Result<(), DataError> {
        if !matches!(
            key,
            "selectedTerm"
                | "terms"
                | "academicProfile"
                | "classGroups"
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

    /// The catalog rows `Core::lms_merge_preview` compares with, read inside
    /// this transaction so an applied merge sees the reviewed offerings.
    pub(crate) fn catalog_rows(&self) -> Result<Vec<Value>, DataError> {
        catalog_list(self.transaction)
    }

    /// Points the LMS assignments and materials of one LMS course at the
    /// timetable course it was matched with. The content hash is left alone:
    /// it tracks what the LMS sent, so an unchanged pull keeps the link and a
    /// changed one is relinked by the next merge.
    pub(crate) fn relink_lms_records(
        &self,
        lms_course_id: u64,
        course_id: &str,
    ) -> Result<usize, DataError> {
        let timestamp = now();
        let mut changed = 0;
        for table in ["assignments", "lms_items"] {
            changed += self
                .transaction
                .execute(
                    &format!(
                        "UPDATE {table} SET rev=rev+1, updated_at=?3,
                           value=json_set(value,'$.courseId',?2,'$.rev',rev+1,'$.updatedAt',?3)
                         WHERE json_extract(value,'$.lmsCourseId')=?1
                           AND COALESCE(json_extract(value,'$.courseId'),'')<>?2"
                    ),
                    params![lms_course_id as i64, course_id, timestamp],
                )
                .map_err(DataError::from)?;
        }
        Ok(changed)
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
    /// Latest workspace assembled under the agent write transaction's
    /// IMMEDIATE lock. Domain gates must use this snapshot rather than the
    /// preflight read made before the transaction began.
    pub(crate) fn workspace(&self) -> Result<Value, DataError> {
        DataStore::load_workspace_from(self.transaction)
    }

    pub(crate) fn catalog_offering(&self, id: &str) -> Result<Option<Value>, DataError> {
        catalog_offering_in(self.transaction, id)
    }

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

/// Every catalog row: detailed offerings, plus search-only rows not yet
/// replaced by details.
fn catalog_list(connection: &Connection) -> Result<Vec<Value>, DataError> {
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
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(DataError::from)?
        .map(|row| {
            serde_json::from_str(&row.map_err(DataError::from)?)
                .map_err(|_| DataError::storage("catalog row contains invalid JSON"))
        })
        .collect();
    rows
}

fn catalog_offering_in(connection: &Connection, id: &str) -> Result<Option<Value>, DataError> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value FROM offerings
             WHERE id=?1 AND deleted_at IS NULL AND missing_since IS NULL",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|_| DataError::storage("offering contains invalid JSON"))
    })
    .transpose()
}

fn catalog_row_in(connection: &Connection, id: &str) -> Result<Option<Value>, DataError> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT COALESCE(
                (SELECT value FROM offerings
                 WHERE id=?1 AND deleted_at IS NULL AND missing_since IS NULL),
                (SELECT value FROM catalog_search_items WHERE id=?1)
             )",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?
        .flatten();
    raw.map(|raw| {
        serde_json::from_str(&raw)
            .map_err(|_| DataError::storage("catalog row contains invalid JSON"))
    })
    .transpose()
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

#[derive(Clone)]
pub struct DataStore {
    path: PathBuf,
}

impl DataStore {
    pub fn initialize(path: PathBuf) -> Result<(Self, bool), DataError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(DataError::from)?;
        }
        let mut connection = Connection::open(&path).map_err(DataError::from)?;
        let schema_changed = migrations::migrate(&mut connection)?;
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
        Ok((store, schema_changed))
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
                "classGroups",
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
                | "classGroups"
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

fn normalize_meeting_clocks(workspace: &mut Value) -> Result<(), DataError> {
    let teaching_periods = crate::schedule::teaching_periods(workspace)
        .map_err(|error| DataError::InvalidInput(error.to_string()))?;
    let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for meeting in courses
        .iter_mut()
        .filter_map(|course| course.get_mut("meetings"))
        .filter_map(Value::as_array_mut)
        .flatten()
    {
        let Some(periods) = meeting.get("periods").and_then(Value::as_array) else {
            continue;
        };
        // An empty period list is an explicit custom clock. Period-backed
        // rows are projected through the current profile table even when an
        // older generated clock is still present in storage.
        if periods.is_empty() {
            continue;
        }
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
        if let Ok((start, end)) =
            crate::schedule::clock_for_periods(&teaching_periods, [minimum, maximum])
        {
            if let Some(object) = meeting.as_object_mut() {
                object.insert("startTime".into(), Value::String(start.into()));
                object.insert("endTime".into(), Value::String(end.into()));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/data.rs"]
mod tests;
