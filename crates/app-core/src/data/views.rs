use super::*;
use crate::schedule::{CatalogEntry, ExamTimetable, SchoolReads};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use serde::Deserialize;
use std::{collections::BTreeMap, sync::PoisonError};

/// The records dataset_state feeds: freshness and page availability.
const SYNC_RECORDS: [&str; 2] = ["freshness", "pageAvailability"];

/// The table the exam timetable is read from. A view that reuses the
/// timetable does not read it, yet carries it (`SchoolReads`).
const EXAMS: &str = "exams";

/// The exam timetable as of a commit. No user intent changes it
/// (`WorkspaceWrite` writes user tables only), so it is read, normalized and
/// validated once per change to its table and every view shares it.
pub(crate) struct SharedExams {
    as_of: u64,
    timetable: Arc<ExamTimetable>,
}

impl SharedExams {
    /// `cached` when its table has not changed since it was read, otherwise
    /// read on `connection`, which sees at least commit `as_of`.
    fn read(
        connection: &Connection,
        cached: Option<&Arc<Self>>,
        versions: &TableVersions,
        as_of: u64,
    ) -> Result<Arc<Self>, DataError> {
        if let Some(cached) = cached.filter(|cached| versions.latest([EXAMS]) <= cached.as_of) {
            return Ok(cached.clone());
        }
        let mut rows = Value::Array(read_visible_rows(connection, EXAMS)?);
        normalize_optional_nulls(&mut rows);
        Vec::<crate::contracts::Exam>::deserialize(&rows)
            .map_err(|error| format!("workspace validation failed: {error}"))?;
        let Value::Array(rows) = rows else {
            return Err(DataError::storage("exams did not read as rows"));
        };
        Ok(Arc::new(Self {
            as_of,
            timetable: Arc::new(ExamTimetable::new(rows)),
        }))
    }
}

/// Every workspace field, with the school records queries evaluate with it:
/// what an intent reads.
fn every_field() -> BTreeSet<&'static str> {
    COLLECTIONS
        .iter()
        .map(|(field, _)| *field)
        .chain([
            "courses",
            "memberships",
            "registrations",
            "grades",
            "lmsCourses",
        ])
        .collect()
}

/// The overlay row (a course override, an assignment mark) a record reads:
/// the latest written live row that `names` it. Several rows can name one
/// record: each device creates its own, and cloud sync brings the others'
/// in without their key columns, so a write here keys on the row it made
/// and never finds them. Every write copies the overlay this returns, so
/// the latest row holds the latest state; the first one by id was stale and
/// hid every later mark.
fn latest_overlay<'a>(rows: &'a [Value], names: impl Fn(&Value) -> bool) -> Option<&'a Value> {
    let written = |row: &'a Value| {
        (
            row.get("updatedAt")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            row.get("id").and_then(Value::as_str).unwrap_or_default(),
        )
    };
    rows.iter()
        .filter(|row| row.get("deletedAt").is_none() && names(row))
        .max_by_key(|row| written(row))
}

pub(super) struct ViewReader {
    /// A read connection of its own, because its authorizer records what a
    /// view reads.
    connection: Connection,
    /// The tables statements compiled on `connection` read, since `tracking`.
    read: Arc<Mutex<BTreeSet<String>>>,
    exams: Option<Arc<SharedExams>>,
    /// The latest partial workspaces queries read (`DataStore::view`), most
    /// recent last.
    views: Vec<CachedView>,
}

/// A view as of commit `as_of` on `day`. It holds while none of `tables`, the
/// tables it read, has changed since.
struct CachedView {
    fields: BTreeSet<&'static str>,
    tables: BTreeSet<String>,
    as_of: u64,
    day: chrono::NaiveDate,
    workspace: Arc<Value>,
    school: Arc<SchoolReads>,
}

impl CachedView {
    fn holds(&self, versions: &TableVersions) -> bool {
        versions.latest(self.tables.iter().map(String::as_str)) <= self.as_of
    }
}

/// Distinct field sets kept; one per query group in use.
const CACHED_VIEWS: usize = 8;

impl ViewReader {
    pub(super) fn new(mut connection: Connection) -> Result<Self, DataError> {
        let read: Arc<Mutex<BTreeSet<String>>> = Arc::default();
        let sink = read.clone();
        // SQLite names what a statement reads as it compiles it, so what a
        // view depends on cannot be left out of a list by hand. A table left
        // out would show old rows; one too many only reads again.
        connection.authorizer(Some(move |context: AuthContext<'_>| {
            if let AuthAction::Read { table_name, .. } = context.action {
                if !table_name.starts_with("sqlite_") {
                    sink.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(table_name.to_string());
                }
            }
            Authorization::Allow
        }))?;
        Ok(Self {
            connection,
            read,
            exams: None,
            views: Vec::new(),
        })
    }

    /// Starts recording what statements read. Cached statements are dropped:
    /// one already compiled would not be reported again.
    fn tracking(&mut self) {
        self.connection.flush_prepared_statement_cache();
        self.read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The tables read since `tracking`.
    fn tracked(&self) -> BTreeSet<String> {
        std::mem::take(&mut *self.read.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl DataStore {
    /// The workspace fields `fields` names, with settings, terms, the
    /// current term and the calendar (`view`), for a reader that needs no
    /// more: `&[]` is the settings alone.
    pub fn load_fields(&self, fields: &[&'static str]) -> Result<Arc<Value>, DataError> {
        Ok(self.view(&fields.iter().copied().collect())?.0)
    }

    /// The user-intent revision a mutation is checked against, without the
    /// workspace.
    pub fn intent_revision(&self) -> Result<String, DataError> {
        intent_revision(&*self.open()?)
    }

    /// Every workspace field, owned (`load_reads`).
    pub fn load_workspace(&self) -> Result<Value, DataError> {
        Ok(Value::clone(&self.load_reads()?.0))
    }

    /// Every workspace field and the school records read models evaluate
    /// with it, from one read, shared with the view cache.
    pub fn load_reads(&self) -> Result<(Arc<Value>, Arc<SchoolReads>), DataError> {
        self.view(&every_field())
            .map(|(workspace, school, _)| (workspace, school))
    }

    /// The workspace fields `fields` names (`view_from`), with the school
    /// records queries evaluate it with, from one read. Cached per field set
    /// until a table it read changes or the day does. The view's revision
    /// names the latest change to those tables, for caches of what is
    /// computed from it.
    pub(crate) fn view(
        &self,
        fields: &BTreeSet<&'static str>,
    ) -> Result<(Arc<Value>, Arc<SchoolReads>, String), DataError> {
        let mut reader = self
            .view_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let today = crate::schedule::japan_today(Utc::now());
        // Before the rows. A commit that landed but has not moved its tables'
        // versions yet stamps its rows as older than they are, which reads
        // them once more; the other way round would keep old rows for good.
        let as_of = self.commit_seq.get();
        let versions = &self.table_versions;
        let revision = |tables: &BTreeSet<String>| {
            format!(
                "{}:{today}",
                versions.latest(tables.iter().map(String::as_str))
            )
        };
        if let Some(index) = reader
            .views
            .iter()
            .position(|view| view.fields == *fields && view.day == today && view.holds(versions))
        {
            let view = reader.views.remove(index);
            let found = (
                view.workspace.clone(),
                view.school.clone(),
                revision(&view.tables),
            );
            reader.views.push(view);
            return Ok(found);
        }
        reader.tracking();
        let cached_exams = reader.exams.clone();
        let transaction = reader.connection.transaction()?;
        let shared = SharedExams::read(&transaction, cached_exams.as_ref(), versions, as_of)?;
        let (workspace, school) = Self::view_with(&transaction, fields, &shared)?;
        transaction.commit()?;
        let mut tables = reader.tracked();
        tables.insert(EXAMS.to_string());
        reader.exams = Some(shared);
        let workspace = Arc::new(workspace);
        let school = Arc::new(school);
        let revision = revision(&tables);
        reader.views.retain(|view| view.fields != *fields);
        if reader.views.len() >= CACHED_VIEWS {
            reader.views.remove(0);
        }
        reader.views.push(CachedView {
            fields: fields.clone(),
            tables,
            as_of,
            day: today,
            workspace: workspace.clone(),
            school: school.clone(),
        });
        Ok((workspace, school, revision))
    }

    /// The view of `fields` as `connection` sees it, with the school records
    /// queries evaluate it with and the exam timetable `exams` read on it.
    fn view_with(
        connection: &Connection,
        fields: &BTreeSet<&'static str>,
        exams: &SharedExams,
    ) -> Result<(Value, SchoolReads), DataError> {
        let (mut workspace, catalog) = Self::view_from(connection, Some(fields))?;
        Self::school_fields_into(connection, fields, &mut workspace)?;
        Ok((
            workspace,
            SchoolReads {
                catalog,
                exams: exams.timetable.clone(),
            },
        ))
    }

    /// The school records among `fields` that queries read beside the
    /// workspace fields.
    fn school_fields_into(
        connection: &Connection,
        fields: &BTreeSet<&'static str>,
        workspace: &mut Value,
    ) -> Result<(), DataError> {
        for (field, table) in [
            ("registrations", "registrations"),
            ("grades", "grades"),
            ("lmsCourses", "lms_courses"),
        ] {
            if fields.contains(field) {
                workspace[field] = Value::Array(read_visible_rows(connection, table)?);
            }
        }
        if SYNC_RECORDS.iter().any(|field| fields.contains(field)) {
            let sync_records = Self::sync_records_from(connection)?;
            for field in SYNC_RECORDS {
                if fields.contains(field) {
                    workspace[field] = sync_records[field].clone();
                }
            }
        }
        Ok(())
    }

    /// Every workspace field (`every_field`) as `connection` sees it, for a
    /// write that reads the workspace under its lock.
    #[cfg(feature = "agent")]
    pub(super) fn whole_view_from(connection: &Connection) -> Result<Value, DataError> {
        let (mut workspace, _) = Self::view_from(connection, None)?;
        Self::school_fields_into(connection, &every_field(), &mut workspace)?;
        Ok(workspace)
    }

    /// Applies one user intent after checking the revision it was made
    /// against, and returns its result with the revision the next intent is
    /// made against and the tables the commit changed; that revision reaches
    /// the observer. `operation` reads every workspace field (`load_reads`),
    /// and the school records read with it, as of the write lock. Before the
    /// commit, the fields the written tables feed are read back and
    /// validated, so an intent cannot commit rows no view can read.
    pub(crate) fn workspace_write<T>(
        &self,
        expected_revision: &str,
        operation: impl FnOnce(&WorkspaceWrite<'_, '_>, &Value, &SchoolReads) -> Result<T, DataError>,
    ) -> Result<(T, String, BTreeSet<String>), DataError> {
        // Lock order: view reader, then the write connection.
        let mut reader = self
            .view_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let today = crate::schedule::japan_today(Utc::now());
        let every = every_field();
        let mut writer = self.connections.writer();
        let ((result, revision, shared), tables) = commit_then(
            &mut writer,
            |transaction| {
                // Under the write lock every commit has moved its tables'
                // versions (`committed_tables`), so this is the state the
                // transaction starts from.
                let as_of = self.commit_seq.get();
                let versions = &self.table_versions;
                let actual = intent_revision(transaction)?;
                if actual != expected_revision {
                    return Err(DataError::conflict(format!(
                        "workspace revision conflict: expected {expected_revision}, actual {actual}"
                    )));
                }
                // A user intent writes no exam row, so the timetable read
                // here stays that of the commit.
                let shared =
                    SharedExams::read(transaction, reader.exams.as_ref(), versions, as_of)?;
                // The whole view as `view` cached it, or read here.
                let cached = reader
                    .views
                    .iter()
                    .find(|view| view.fields == every && view.day == today && view.holds(versions))
                    .map(|view| (view.workspace.clone(), view.school.clone()));
                let (current, school) = match cached {
                    Some(current) => current,
                    None => {
                        let (workspace, school) = Self::view_with(transaction, &every, &shared)?;
                        (Arc::new(workspace), Arc::new(school))
                    }
                };
                let write = WorkspaceWrite {
                    transaction,
                    actor: WorkspaceChangeSource::UserIntent.actor(),
                };
                let result = operation(&write, &current, &school)?;
                drop((current, school));
                Ok((result, shared))
            },
            |transaction, (result, shared), tables| {
                // Read after the courses the intent fed were merged again.
                let fed = crate::workspace_changes::loaded_fields(
                    crate::workspace_changes::changes_for(tables),
                );
                let (written, _) = Self::view_from(transaction, Some(&fed))?;
                crate::contracts::Workspace::deserialize(&written)
                    .map_err(|error| format!("workspace validation failed: {error}"))?;
                Ok((result, intent_revision(transaction)?, shared))
            },
        )?;
        reader.exams = Some(shared);
        self.committed_tables(&tables);
        drop(writer);
        drop(reader);
        self.observer.committed(
            WorkspaceChangeSource::UserIntent,
            Some(revision.clone()),
            &tables,
        );
        Ok((result, revision, tables))
    }

    /// The workspace fields `fields` names (every one when `None`), each read
    /// exactly as the whole workspace reads it; settings, terms, the current
    /// term and the academic calendar are always read. A field left out
    /// keeps its empty default.
    pub(super) fn view_from(
        connection: &Connection,
        fields: Option<&BTreeSet<&str>>,
    ) -> Result<(Value, BTreeMap<String, CatalogEntry>), DataError> {
        let wants = |field: &str| {
            field == "academicCalendar" || fields.is_none_or(|fields| fields.contains(field))
        };
        let mut workspace = empty_workspace();
        for (field, table) in COLLECTIONS {
            if wants(field) {
                workspace[*field] = Value::Array(read_visible_rows(connection, table)?);
            }
        }
        // Courses as their sources' writes merged them (`course_merge`).
        if wants("courses") {
            workspace["courses"] = Value::Array(course_merge::read(connection, "course")?);
        }
        if wants("memberships") {
            if let Some(memberships) = workspace
                .get_mut("memberships")
                .and_then(Value::as_array_mut)
            {
                memberships.extend(read_rows(connection, "manual_memberships")?);
                memberships.extend(course_merge::read(connection, "membership")?);
            }
        }
        let marks = if wants("assignments") {
            read_rows(connection, "assignment_marks")?
        } else {
            Vec::new()
        };
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
                let Some(mark) = latest_overlay(&marks, |mark| {
                    mark.get("sourceKey").and_then(Value::as_str) == Some(source_key)
                        || mark.get("assignmentId").and_then(Value::as_str) == Some(id)
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
        Self::settings_into(connection, &mut workspace)?;
        if let Some(terms) = workspace.get_mut("terms").and_then(Value::as_array_mut) {
            for term in course_merge::read(connection, "term")? {
                if !terms.contains(&term) {
                    terms.push(term);
                }
            }
        }
        school_projection::apply_academic_identity(connection, &mut workspace)?;
        // Rows of an LMS course name the course its LMS key names.
        let links = course_keys::lms_links(connection)?;
        for field in ["assignments", "materials", "memberships", "planItems"] {
            for row in workspace[field].as_array_mut().into_iter().flatten() {
                if let Some(id) = row["lmsCourseId"].as_u64().and_then(|id| links.get(&id)) {
                    row["courseId"] = id.clone().into();
                }
            }
        }
        let catalog = school_projection::catalog_entries(connection, &workspace)?;
        let overrides = if wants("courses") {
            read_rows(connection, "course_overrides")?
        } else {
            Vec::new()
        };
        if let Some(courses) = workspace.get_mut("courses").and_then(Value::as_array_mut) {
            for course in courses {
                let Some(id) = course.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let source_key = course
                    .get("sourceKey")
                    .and_then(Value::as_str)
                    .unwrap_or(id);
                if let Some(overlay) = latest_overlay(&overrides, |row| {
                    row.get("sourceKey").and_then(Value::as_str) == Some(source_key)
                        || row.get("courseId").and_then(Value::as_str) == Some(id)
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
                course[field] = adpt_isct::lms::decode_entities(&crate::schedule::localized(
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
        let today_jst = crate::schedule::japan_today(Utc::now());
        workspace["currentTerm"] = Value::String(
            current_term_for(&workspace, today_jst)
                .ok_or_else(|| "current academic term is not configured".to_string())?,
        );
        normalize_optional_nulls(&mut workspace);
        normalize_meeting_clocks(&mut workspace)?;
        Ok((workspace, catalog))
    }

    /// The settings rows the workspace carries, with the preferences merged
    /// into `settings`.
    pub(super) fn settings_into(
        connection: &Connection,
        workspace: &mut Value,
    ) -> Result<(), DataError> {
        for key in [
            "deviceId",
            "selectedTerm",
            "terms",
            "settings",
            "academicProfile",
            "classGroups",
            "importStatus",
            "recentSeen",
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
            workspace["settings"]["cloud"] = super::settings::retire_providers(&value);
        }
        Ok(())
    }

    /// The records dataset_state feeds, `SYNC_RECORDS`.
    fn sync_records_from(connection: &Connection) -> Result<Value, DataError> {
        let freshness = crate::sync_state::list(connection, None)?;
        Ok(json!({
            "freshness": Self::freshness_from(&freshness),
            "pageAvailability": Self::page_availability_from(&freshness)?,
        }))
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
