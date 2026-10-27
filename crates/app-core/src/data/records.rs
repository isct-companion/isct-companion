use super::*;

pub(super) fn content_value(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        for key in ["rev", "updatedAt", "updatedBy", "deletedAt"] {
            object.remove(key);
        }
    }
    value
}

pub(super) fn string_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, DataError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DataError::from(format!("record is missing {key}")))
}

pub(super) fn validate_contract<T: DeserializeOwned>(
    table: &str,
    value: &Value,
) -> Result<(), DataError> {
    serde_json::from_value::<T>(value.clone())
        .map(|_| ())
        .map_err(|error| DataError::from(format!("{table} validation failed: {error}")))
}

/// Domain rules every local writer (workspace intents, MCP, scheduler) must
/// meet beyond the wire shape. Synced and imported rows are only shape-checked;
/// read models skip and report a stored record they cannot evaluate.
pub(super) fn validate_local_record(table: &str, value: &Value) -> Result<(), DataError> {
    let invalid = |error: &dyn std::fmt::Display| DataError::from(format!("{table}: {error}"));
    match table {
        "manual_courses" => {
            let course = serde_json::from_value::<crate::contracts::Course>(value.clone())
                .map_err(|error| invalid(&error))?;
            for meeting in course.meetings {
                if meeting.periods.is_empty() {
                    let clock = chrono::NaiveTime::parse_from_str;
                    let valid = clock(&meeting.start_time, "%H:%M")
                        .ok()
                        .zip(clock(&meeting.end_time, "%H:%M").ok())
                        .is_some_and(|(start, end)| start < end);
                    if !valid {
                        return Err(invalid(&"custom meeting times are invalid"));
                    }
                    continue;
                }
                if meeting
                    .periods
                    .windows(2)
                    .any(|pair| pair[1] != pair[0].saturating_add(1))
                {
                    return Err(invalid(
                        &"meeting periods must be a contiguous ascending range",
                    ));
                }
                crate::schedule::validate_period_range([
                    meeting.periods[0],
                    *meeting.periods.last().unwrap_or(&meeting.periods[0]),
                ])
                .map_err(|error| invalid(&error))?;
            }
            Ok(())
        }
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

pub(super) fn validate_user_record(table: &str, value: &Value) -> Result<(), DataError> {
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

pub(super) fn row_version(value: &Value) -> Result<VersionedValue, DataError> {
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

pub(super) fn read_rows(connection: &Connection, table: &str) -> Result<Vec<Value>, DataError> {
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

pub(super) fn read_visible_rows(
    connection: &Connection,
    table: &str,
) -> Result<Vec<Value>, DataError> {
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

/// The internal course id of a school-facing source (`course_keys`): the
/// course its offering id names when there is one, else the one its own key
/// names, else a new one. Every key it names is kept for that course.
pub(crate) fn resolve_course_id(
    transaction: &Transaction<'_>,
    provider: &str,
    external_id: &str,
    academic_year: i32,
    offering_id: Option<&str>,
) -> Result<String, DataError> {
    if !matches!(provider, "lms" | "kyomu" | "manual") || external_id.trim().is_empty() {
        return Err("invalid course identity source".into());
    }
    let offering =
        offering_id.and_then(|offering| course_keys::offering_key(academic_year, offering));
    let source = match provider {
        "lms" => Some(course_keys::lms_key(external_id)),
        "kyomu" => Some(course_keys::kyomu_key(academic_year, external_id)),
        // A course made on a device is known by its offering alone.
        _ => None,
    };
    let keys: Vec<String> = offering.into_iter().chain(source).collect();
    if keys.is_empty() {
        return Err("course identity requires an academic year and 9-digit offering id".into());
    }
    course_keys::resolve(transaction, &keys)
}

/// The internal course id of a catalog offering of `academic_year`.
pub(crate) fn offering_course_id(
    transaction: &Transaction<'_>,
    academic_year: i32,
    offering_id: &str,
) -> Result<String, DataError> {
    let key = course_keys::offering_key(academic_year, offering_id)
        .unwrap_or_else(|| format!("catalog:{academic_year}:{offering_id}"));
    course_keys::resolve(transaction, &[key])
}

type StoredRow = (String, u64, String, String, Option<String>, Option<String>);

pub(super) fn read_stored_row(
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

pub(super) fn upsert_row(
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

pub(super) fn read_setting(connection: &Connection, key: &str) -> Result<Option<Value>, DataError> {
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

pub(super) fn read_settings_export(connection: &Connection) -> Result<Vec<Value>, DataError> {
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

pub(super) fn setting_version(row: &Value) -> Result<VersionedValue, DataError> {
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

pub(super) fn import_setting(transaction: &Transaction<'_>, row: &Value) -> Result<(), DataError> {
    let key = string_field(row, "key")?;
    let incoming = setting_version(row)?;
    if incoming.deleted_at.is_none() {
        validate_setting(key, &incoming.value)?;
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

pub(super) fn save_setting(
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

/// Remove the LMS academic-year suffix while retaining the class section.
pub(crate) fn lms_course_code(shortname: &str) -> String {
    let shortname = shortname.trim();
    shortname
        .strip_suffix(']')
        .and_then(|s| s.rsplit_once('[').map(|(head, _)| head.trim()))
        .unwrap_or(shortname)
        .to_ascii_uppercase()
}
