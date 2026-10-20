use super::*;

pub(crate) type SnapshotParts = (Value, Value, String);

/// `PRAGMA data_version` of user.db and cache.db on the snapshot connection.
type Versions = (i64, i64);

pub(super) struct SnapshotReader {
    /// A read connection of its own: data versions only compare between
    /// reads of one connection. Every commit lands on another connection
    /// (the store's write connection, or another store on the same files),
    /// so every commit changes them.
    connection: Connection,
    cached: Option<(Versions, chrono::NaiveDate, SnapshotParts, String)>,
}

impl SnapshotReader {
    pub(super) fn new(connection: Connection) -> Self {
        Self {
            connection,
            cached: None,
        }
    }

    fn versions(&self) -> Result<Versions, DataError> {
        let version = |schema: &str| {
            self.connection
                .query_row(&format!("PRAGMA {schema}.data_version"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(DataError::from)
        };
        Ok((version("main")?, version("cache")?))
    }

    fn baseline(
        &self,
        versions: Versions,
        today: chrono::NaiveDate,
    ) -> Option<(SnapshotParts, String)> {
        self.cached
            .as_ref()
            .filter(|(v, day, _, _)| *v == versions && *day == today)
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
        let today = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        // Observed before the read: a commit landing during the read must
        // invalidate this snapshot rather than be stamped onto old data.
        let versions = reader.versions()?;
        if let Some(parts) = reader.baseline(versions, today) {
            if std::env::var_os("ISCT_TRACE_READS").is_some() {
                eprintln!("[workspace-snapshot] cache=hit data_version={versions:?}");
            }
            return Ok(parts);
        }
        if std::env::var_os("ISCT_TRACE_READS").is_some() {
            eprintln!("[workspace-snapshot] cache=miss data_version={versions:?}");
        }
        let transaction = reader.connection.transaction()?;
        let parts = (
            Self::load_workspace_from(&transaction)?,
            Self::school_records_from(&transaction)?,
            intent_revision(&transaction)?,
        );
        transaction.commit()?;
        let hash = content_revision(&parts.0, &parts.1)?;
        reader.cached = Some((versions, today, parts.clone(), hash.clone()));
        Ok((parts, hash))
    }

    /// Applies one user intent after checking the revision it was made
    /// against. `operation` returns its result with the snapshot parts and
    /// content hash as committed; the parts' revision reaches the observer.
    pub(crate) fn workspace_write<T>(
        &self,
        expected_revision: &str,
        operation: impl FnOnce(
            &WorkspaceWrite<'_, '_>,
            &Value,
            &Value,
        ) -> Result<(T, SnapshotParts, String), DataError>,
    ) -> Result<T, DataError> {
        // Lock order: snapshot reader, then the write connection.
        let mut reader = self
            .snapshot_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let today = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        let prior = reader.cached.clone();
        let mut writer = self.connections.writer();
        let ((result, parts, hash), tables) = commit_on(&mut writer, |transaction| {
            // Under the write lock no other commit can land, so these are the
            // versions of the state this transaction starts from.
            let versions = reader.versions()?;
            let actual = intent_revision(transaction)?;
            if actual != expected_revision {
                return Err(DataError::conflict(format!(
                    "workspace revision conflict: expected {expected_revision}, actual {actual}"
                )));
            }
            let (current, school_records, _) = match prior {
                Some((v, day, parts, _)) if v == versions && day == today => parts,
                _ => (
                    Self::load_workspace_from(transaction)?,
                    Self::school_records_from(transaction)?,
                    actual,
                ),
            };
            let write = WorkspaceWrite {
                transaction,
                actor: WorkspaceChangeSource::UserIntent.actor(),
            };
            operation(&write, &current, &school_records)
        })?;
        let revision = parts.2.clone();
        // The committed parts are the snapshot for the versions after this
        // commit. They are read before the write lock is released, so no
        // commit of this store can land in between; only another process
        // writing the same files could, and the app opens one store per
        // data folder. Without versions the next read builds it afresh.
        reader.cached = reader
            .versions()
            .ok()
            .map(|versions| (versions, today, parts, hash));
        drop(writer);
        drop(reader);
        self.observer
            .committed(WorkspaceChangeSource::UserIntent, Some(revision), &tables);
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
        let freshness = crate::sync_state::list(connection, None)?;
        records.insert("freshness".into(), Self::freshness_from(&freshness));
        records.insert(
            "pageAvailability".into(),
            Self::page_availability_from(&freshness)?,
        );
        Ok(Value::Object(records))
    }

    /// `dataset/key` → when the key was last read in full and how its latest
    /// attempt went, for shells that show data age.
    fn freshness_from(states: &[crate::sync_state::KeyState]) -> Value {
        let time = |at: Option<DateTime<Utc>>| at.map(|at| at.to_rfc3339());
        Value::Object(
            states
                .iter()
                .map(|state| {
                    (
                        format!("{}/{}", state.dataset, state.key),
                        json!({
                            "fetchedAt": time(state.fetched_at),
                            "outcome": state.outcome,
                            "message": state.message,
                            "failures": state.failures,
                            "nextEligible": time(state.next_eligible),
                            "lastError": state.last_error,
                        }),
                    )
                })
                .collect(),
        )
    }

    /// The latest complete read of each 教務Web page, keyed `kyomu:<kind>`.
    fn page_availability_from(states: &[crate::sync_state::KeyState]) -> Result<Value, DataError> {
        let mut availability = Map::new();
        for state in states
            .iter()
            .filter(|state| state.dataset == crate::sync_state::Dataset::KyomuPage.name())
        {
            let Some(checked_at) = state.fetched_at else {
                continue;
            };
            let unavailable = state.outcome == Some(crate::sync_state::Outcome::Unavailable);
            availability.insert(
                format!("kyomu:{}", state.key),
                serde_json::to_value(SchoolPageAvailability {
                    state: if unavailable {
                        SchoolPageAvailabilityState::Unavailable
                    } else {
                        SchoolPageAvailabilityState::Available
                    },
                    checked_at: checked_at.to_rfc3339(),
                    message: if unavailable {
                        state.message.clone().unwrap_or_default()
                    } else {
                        String::new()
                    },
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
