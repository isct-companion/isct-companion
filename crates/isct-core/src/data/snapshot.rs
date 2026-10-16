use super::*;

impl DataStore {
    pub fn load_workspace(&self) -> Result<Value, DataError> {
        self.workspace_snapshot_parts()
            .map(|(workspace, _, _)| workspace)
    }

    /// `(workspace, school records, user-intent revision)` from one read.
    pub(crate) fn workspace_snapshot_parts(&self) -> Result<(Value, Value, String), DataError> {
        let mut connection = self.open()?;
        let transaction = connection.transaction().map_err(DataError::from)?;
        let workspace = Self::load_workspace_from(&transaction)?;
        let school_records = Self::school_records_from(&transaction)?;
        let revision = intent_revision(&transaction)?;
        transaction.commit().map_err(DataError::from)?;
        Ok((workspace, school_records, revision))
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
        let actual = intent_revision(&transaction)?;
        if actual != expected_revision {
            return Err(DataError::conflict(format!(
                "workspace revision conflict: expected {expected_revision}, actual {actual}"
            )));
        }
        let current = Self::load_workspace_from(&transaction)?;
        let school_records = Self::school_records_from(&transaction)?;
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
        kyomu_projection::project(connection, &mut workspace)?;
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
