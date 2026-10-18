use super::*;

pub(crate) type SnapshotParts = (Value, Value, String);

#[derive(Default)]
pub(super) struct SnapshotReader {
    connection: Option<Connection>,
    cached: Option<(i64, chrono::NaiveDate, SnapshotParts, String)>,
}

impl SnapshotReader {
    fn baseline(&self, version: i64, today: chrono::NaiveDate) -> Option<(SnapshotParts, String)> {
        self.cached
            .as_ref()
            .filter(|(v, day, _, _)| *v == version && *day == today)
            .map(|(_, _, parts, hash)| (parts.clone(), hash.clone()))
    }
}

impl DataStore {
    pub fn load_workspace(&self) -> Result<Value, DataError> {
        self.workspace_snapshot_parts()
            .map(|(workspace, _, _)| workspace)
    }

    /// `(workspace, school records, user-intent revision)` from one read.
    pub(crate) fn workspace_snapshot_parts(&self) -> Result<SnapshotParts, DataError> {
        self.workspace_snapshot_with_revision()
            .map(|(parts, _)| parts)
    }

    pub(crate) fn workspace_snapshot_with_revision(
        &self,
    ) -> Result<(SnapshotParts, String), DataError> {
        let mut reader = self
            .snapshot_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if reader.connection.is_none() {
            reader.connection = Some(self.open()?);
        }
        let today = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        let version: i64 = reader
            .connection
            .as_ref()
            .ok_or_else(|| DataError::storage("Snapshot connection unavailable"))?
            .query_row("PRAGMA data_version", [], |row| row.get(0))?;
        if let Some(parts) = reader.baseline(version, today) {
            if std::env::var_os("ISCT_TRACE_READS").is_some() {
                eprintln!("[workspace-snapshot] cache=hit data_version={version}");
            }
            return Ok(parts);
        }
        if std::env::var_os("ISCT_TRACE_READS").is_some() {
            eprintln!("[workspace-snapshot] cache=miss data_version={version}");
        }
        let transaction = reader
            .connection
            .as_mut()
            .ok_or_else(|| DataError::storage("Snapshot connection unavailable"))?
            .transaction()?;
        let parts = (
            Self::load_workspace_from(&transaction)?,
            Self::school_records_from(&transaction)?,
            intent_revision(&transaction)?,
        );
        transaction.commit()?;
        // data_version belongs to this persistent connection. Every commit on
        // another connection invalidates it, including school and cloud writes.
        // Keep the version observed before the read: a concurrent later commit
        // must invalidate this snapshot rather than be stamped onto old data.
        let hash = content_revision(&parts.0, &parts.1)?;
        reader.cached = Some((version, today, parts.clone(), hash.clone()));
        Ok((parts, hash))
    }

    pub(crate) fn workspace_write<T>(
        &self,
        expected_revision: &str,
        operation: impl FnOnce(
            &WorkspaceWrite<'_, '_>,
            &Value,
            &Value,
        ) -> Result<(T, SnapshotParts, String), DataError>,
    ) -> Result<T, DataError> {
        let mut reader = self
            .snapshot_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if reader.connection.is_none() {
            reader.connection = Some(self.open()?);
        }
        let today = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        let prior = reader.cached.clone();
        let connection = reader
            .connection
            .as_mut()
            .ok_or_else(|| DataError::storage("Snapshot connection unavailable"))?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = transaction.query_row("PRAGMA data_version", [], |row| row.get(0))?;
        let actual = intent_revision(&transaction)?;
        if actual != expected_revision {
            return Err(DataError::conflict(format!(
                "workspace revision conflict: expected {expected_revision}, actual {actual}"
            )));
        }
        let (current, school_records, _) = match prior {
            Some((v, day, parts, _)) if v == version && day == today => parts,
            _ => (
                Self::load_workspace_from(&transaction)?,
                Self::school_records_from(&transaction)?,
                actual,
            ),
        };
        let (result, parts, hash) = operation(
            &WorkspaceWrite {
                transaction: &transaction,
            },
            &current,
            &school_records,
        )?;
        transaction.commit()?;
        // Own-connection commits do not change data_version. This update is
        // atomic with the write; any later external commit invalidates it.
        reader.cached = Some((version, today, parts, hash));
        Ok(result)
    }

    pub(super) fn load_workspace_from(connection: &Connection) -> Result<Value, DataError> {
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
        for key in [
            "deviceId",
            "selectedTerm",
            "terms",
            "settings",
            "academicProfile",
            "classGroups",
            "importStatus",
            "syncStatus",
            "logs",
        ] {
            if let Some(value) = read_setting(connection, key)? {
                workspace[key] = value;
            }
        }
        for key in ["userPreferences", "devicePreferences"] {
            if let Some(mut value) = read_setting(connection, key)? {
                if key == "userPreferences" {
                    if let Some(notifications) = value
                        .get_mut("notifications")
                        .and_then(Value::as_object_mut)
                    {
                        // Before independent exam preferences, exams followed
                        // the class toggle and lead. Preserve that on upgrade.
                        if !notifications.contains_key("exams") {
                            let enabled =
                                notifications.get("classes").cloned().unwrap_or(json!(true));
                            notifications.insert("exams".into(), enabled);
                        }
                        if !notifications.contains_key("examLeadMinutes") {
                            let lead = notifications
                                .get("classLeadMinutes")
                                .cloned()
                                .unwrap_or(json!(10));
                            notifications.insert("examLeadMinutes".into(), lead);
                        }
                    }
                }
                apply_preferences(&mut workspace["settings"], value);
            }
        }
        if let Some(value) = read_setting(connection, "cloud")? {
            workspace["settings"]["cloud"] = value;
        }
        school_projection::project(connection, &mut workspace)?;
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
        let language = crate::schedule::content_language(&workspace).to_string();
        for course in workspace["courses"].as_array_mut().into_iter().flatten() {
            for field in ["title", "instructor"] {
                course[field] =
                    crate::school_parser::lms::decode_entities(&crate::schedule::localized(
                        course.pointer(&format!("/names/{field}")),
                        &language,
                        course[field].as_str().unwrap_or(""),
                    ))
                    .into();
            }
            for meeting in course["meetings"].as_array_mut().into_iter().flatten() {
                meeting["room"] = crate::schedule::localized(
                    meeting.get("roomNames"),
                    &language,
                    meeting["room"].as_str().unwrap_or(""),
                )
                .into();
            }
        }
        let today_jst = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        workspace["currentTerm"] = Value::String(
            current_term_for(&workspace, today_jst)
                .ok_or_else(|| "current academic term is not configured".to_string())?,
        );
        normalize_optional_nulls(&mut workspace);
        normalize_meeting_clocks(&mut workspace)?;
        let _: crate::contracts::Workspace = serde_json::from_value(workspace.clone())
            .map_err(|error| format!("workspace validation failed: {error}"))?;
        Ok(workspace)
    }

    pub(super) fn school_records_from(connection: &Connection) -> Result<Value, DataError> {
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
        records.insert(
            "pageAvailability".into(),
            Self::page_availability_from(connection)?,
        );
        Ok(Value::Object(records))
    }

    pub(super) fn last_import_runs_from(connection: &Connection) -> Result<Value, DataError> {
        let mut statement = connection
            .prepare(
                "SELECT provider, scope, MAX(finished_at) FROM import_runs
                 WHERE state IN ('complete','unavailable') AND finished_at IS NOT NULL
                 GROUP BY provider, scope",
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

    pub(super) fn page_availability_from(connection: &Connection) -> Result<Value, DataError> {
        let mut statement = connection
            .prepare(
                "SELECT provider,scope,state,finished_at,COALESCE(message,'') FROM import_runs
                 WHERE state IN ('complete','unavailable') AND finished_at IS NOT NULL
                 ORDER BY finished_at,rowid",
            )
            .map_err(DataError::from)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(DataError::from)?;
        let mut availability = Map::new();
        for row in rows {
            let (provider, scope, state, checked_at, message) = row.map_err(DataError::from)?;
            let availability_state = if state == "unavailable" {
                SchoolPageAvailabilityState::Unavailable
            } else {
                SchoolPageAvailabilityState::Available
            };
            availability.insert(
                format!("{provider}:{scope}"),
                serde_json::to_value(SchoolPageAvailability {
                    state: availability_state,
                    checked_at,
                    message,
                })
                .map_err(DataError::from)?,
            );
        }
        Ok(Value::Object(availability))
    }
}

pub(crate) fn content_revision(
    workspace: &Value,
    school_records: &Value,
) -> Result<String, DataError> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(&(workspace, school_records))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
