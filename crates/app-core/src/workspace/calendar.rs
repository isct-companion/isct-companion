use super::*;

pub(super) fn save_simple_record<F>(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    field: &str,
    table: &str,
    draft: Value,
    existing_id: Option<String>,
    enrich: F,
) -> Result<(), DataError>
where
    F: FnOnce(&mut Map<String, Value>) -> Result<(), DataError>,
{
    let mut row = draft;
    let object = object_mut(&mut row, "record draft")?;
    let existing = existing_id
        .as_deref()
        .and_then(|id| visible_row(current, field, id));
    object.insert(
        "id".into(),
        Value::String(existing_id.unwrap_or_else(|| Uuid::now_v7().to_string())),
    );
    // Run and fetch bookkeeping belongs to Core; a form built from an older
    // snapshot must not roll it back.
    for field in crate::data::bookkeeping_fields(table) {
        match existing.and_then(|row| row.get(*field)) {
            Some(value) => object.insert((*field).into(), value.clone()),
            None => object.remove(*field),
        };
    }
    enrich(object)?;
    write
        .put(table, row, existing.and_then(revision))
        .map(|_| ())
}

pub(super) fn save_calendar_exception(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    draft: Value,
    existing_id: Option<String>,
) -> Result<(), DataError> {
    let date = string(&draft, "date")?.to_string();
    chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
        .map_err(|_| DataError::from("calendar exception date is invalid"))?;
    let label = string(&draft, "label")?.trim().to_string();
    if label.is_empty() {
        return Err("calendar exception label is required".into());
    }
    let kind = string(&draft, "type")?.to_string();
    if !matches!(
        kind.as_str(),
        "no-class" | "holiday" | "break" | "replacement"
    ) {
        return Err("calendar exception type is invalid".into());
    }
    if kind == "replacement"
        && !draft
            .get("replacementDay")
            .and_then(Value::as_u64)
            .is_some_and(|day| (1..=7).contains(&day))
    {
        return Err("replacement day must be within 1..7".into());
    }
    if rows(current, "exceptions")?.any(|row| {
        row.get("date").and_then(Value::as_str) == Some(date.as_str())
            && row.get("deletedAt").is_none_or(Value::is_null)
            && row.get("id").and_then(Value::as_str) != existing_id.as_deref()
    }) {
        return Err("that date already has a calendar exception".into());
    }
    let mut draft = draft;
    let object = object_mut(&mut draft, "calendar exception")?;
    object.insert("label".into(), Value::String(label));
    if kind != "replacement" {
        object.remove("replacementDay");
    }
    save_simple_record(
        write,
        current,
        "exceptions",
        "calendar_exceptions",
        draft,
        existing_id,
        |_| Ok(()),
    )
}

pub(super) fn replace_records(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    field: &str,
    table: &str,
    replacements: Vec<Value>,
) -> Result<(), DataError> {
    let existing: Vec<_> = rows(current, field)?.collect();
    let mut retained = HashSet::new();
    for mut replacement in replacements {
        let object = object_mut(&mut replacement, "record")?;
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        if !retained.insert(id.clone()) {
            return Err(format!("duplicate {field} id {id}").into());
        }
        object.insert("id".into(), Value::String(id.clone()));
        let expected = existing
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(&id))
            .and_then(|row| revision(row));
        write.put(table, replacement, expected)?;
    }
    for row in existing {
        let id = string(row, "id")?;
        if !retained.contains(id) {
            write.delete(table, id, revision(row))?;
        }
    }
    Ok(())
}

pub(super) fn record_scheduled_task_run(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    task_id: &str,
    occurrence_at: String,
    run_at: String,
    result: String,
) -> Result<(), DataError> {
    chrono::DateTime::parse_from_rfc3339(&occurrence_at)
        .map_err(|_| DataError::from("task occurrence must be RFC 3339"))?;
    chrono::DateTime::parse_from_rfc3339(&run_at)
        .map_err(|_| DataError::from("task run time must be RFC 3339"))?;
    let task = visible_row(current, "scheduledTasks", task_id)
        .ok_or_else(|| "scheduled task no longer exists".to_string())?;
    let mut updated = task.clone();
    let object = object_mut(&mut updated, "scheduled task")?;
    object.insert("lastOccurrenceAt".into(), Value::String(occurrence_at));
    object.insert("lastRunAt".into(), Value::String(run_at));
    object.insert("lastResult".into(), Value::String(result));
    write
        .put("scheduled_tasks", updated, revision(task))
        .map(|_| ())
}

pub(super) fn set_subscription(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    subscription_kind: String,
    enabled: Option<bool>,
    url: Option<String>,
    reset_remote: bool,
) -> Result<(), DataError> {
    if !matches!(
        subscription_kind.as_str(),
        "calendar" | "departments" | "important-dates" | "exams"
    ) {
        return Err("subscription kind is invalid".into());
    }
    if url.as_ref().is_some_and(|url| !url.starts_with("https://")) {
        return Err("subscription URL must use HTTPS".into());
    }
    let existing = rows(current, "subscriptions")?.find(|row| {
        row.get("kind").and_then(Value::as_str) == Some(&subscription_kind)
            && row.get("deletedAt").is_none_or(Value::is_null)
    });
    let mut row = existing.cloned().unwrap_or_else(|| {
        json!({
            "id":format!("subscription:public:{subscription_kind}"),
            "kind":subscription_kind,
            "url":"https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json",
            "enabled":true
        })
    });
    let object = object_mut(&mut row, "subscription")?;
    if let Some(enabled) = enabled {
        object.insert("enabled".into(), Value::Bool(enabled));
    }
    if let Some(url) = url {
        object.insert("url".into(), Value::String(url));
    }
    if reset_remote {
        for key in ["indexHash", "dataUrl", "remoteHash", "year", "lastError"] {
            object.remove(key);
        }
    }
    write
        .put("subscriptions", row, existing.and_then(revision))
        .map(|_| ())
}

pub(super) fn delete_visible(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    field: &str,
    table: &str,
    id: &str,
) -> Result<(), DataError> {
    let row =
        visible_row(current, field, id).ok_or_else(|| format!("{field} record was not found"))?;
    write.delete(table, id, revision(row)).map(|_| ())
}

pub(super) fn save_intensive(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    draft: Value,
) -> Result<(), DataError> {
    if visible_row(current, "courses", course_id).is_none() {
        return Err("course was not found".into());
    }
    let existing = rows(current, "intensiveSchedules")?
        .find(|row| row.get("courseId").and_then(Value::as_str) == Some(course_id));
    let existing_id = existing.and_then(|row| object_string(row, "id"));
    save_simple_record(
        write,
        current,
        "intensiveSchedules",
        "intensive_schedules",
        draft,
        existing_id,
        |row| {
            row.insert("courseId".into(), Value::String(course_id.into()));
            Ok(())
        },
    )
}

/// Hides, pins or completes one assignment; agent tools share this rule.
pub(crate) fn set_assignment_mark(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    assignment_id: &str,
    hidden: Option<bool>,
    pinned: Option<bool>,
    completed_at: Option<String>,
) -> Result<(), DataError> {
    let assignment = visible_row(current, "assignments", assignment_id)
        .ok_or_else(|| "assignment was not found".to_string())?;
    let source_key = assignment
        .get("sourceKey")
        .and_then(Value::as_str)
        .unwrap_or(assignment_id);
    write.put("assignment_marks", json!({
        "sourceKey":source_key,
        "assignmentId":assignment_id,
        "hidden":hidden.unwrap_or_else(|| assignment.get("hidden").and_then(Value::as_bool).unwrap_or(false)),
        "pinned":pinned.unwrap_or_else(|| assignment.get("pinned").and_then(Value::as_bool).unwrap_or(false)),
        "completedAt":completed_at.or_else(|| assignment.get("completedAt").and_then(Value::as_str).map(str::to_string))
    }), None).map(|_| ())
}

pub(super) fn record_lesson(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    occurrence: String,
    section_id: u64,
) -> Result<(), DataError> {
    chrono::DateTime::parse_from_rfc3339(&occurrence)
        .map_err(|_| DataError::from("lesson occurrence must be RFC 3339"))?;
    let course = visible_row(current, "courses", course_id)
        .ok_or_else(|| "course was not found".to_string())?;
    let mut log = course
        .pointer("/overrides/lessonLog")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    log.retain(|mark| mark.get("occurrence").and_then(Value::as_str) != Some(&occurrence));
    log.push(json!({"occurrence":occurrence,"sectionId":section_id}));
    if log.len() > 60 {
        log.drain(..log.len() - 60);
    }
    put_course_override(write, course, [("lessonLog", Value::Array(log))])
}
