use super::*;
use crate::schedule::{CatalogEntry, SchoolReads};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// `(workspace, school records, user-intent revision, the school records
/// read models evaluate with the workspace)`, shared: a read hands out the
/// snapshot itself, never a copy of it.
pub(crate) type SnapshotParts = (Arc<Value>, Arc<Value>, String, Arc<SchoolReads>);

/// `PRAGMA data_version` of user.db, cache.db and sync.db on the snapshot
/// connection.
type Versions = (i64, i64, i64);

/// The school records only sync.db feeds: freshness and page availability.
/// A commit there changes nothing else, so they are read again over the
/// cached workspace instead of rebuilding it.
const SYNC_RECORDS: [&str; 2] = ["freshness", "pageAvailability"];

/// The workspace's largest school collection: the exam timetable. No user
/// intent can change it (`WorkspaceWrite` writes user tables only) and
/// nothing in the snapshot rewrites it, so it is read, normalized and
/// validated once per cache.db version, and a user write rebuilds only the
/// rest of the snapshot.
const SHARED_SCHOOL: &[(&str, Validate)] = &[("exams", |rows| {
    Vec::<crate::contracts::Exam>::deserialize(rows).map(drop)
})];

/// Checks one shared collection against its contract.
type Validate = fn(&Value) -> Result<(), serde_json::Error>;

pub(crate) struct SharedSchool {
    /// cache.db's data version on the snapshot connection, when one is known.
    version: Option<i64>,
    /// In `SHARED_SCHOOL` order.
    collections: Vec<Value>,
    /// Feeds the content revision, so the collections are not hashed again.
    digest: [u8; 32],
}

impl SharedSchool {
    /// `cached` when it was read at `version`, otherwise read on `connection`.
    fn read(
        connection: &Connection,
        cached: Option<&Arc<Self>>,
        version: Option<i64>,
    ) -> Result<Arc<Self>, DataError> {
        if let Some(cached) = cached.filter(|cached| version.is_some() && cached.version == version)
        {
            return Ok(cached.clone());
        }
        let mut collections = Vec::with_capacity(SHARED_SCHOOL.len());
        let mut digest = Sha256::new();
        for (field, validate) in SHARED_SCHOOL {
            let table = COLLECTIONS
                .iter()
                .find(|(name, _)| name == field)
                .map(|(_, table)| *table)
                .ok_or_else(|| DataError::storage(format!("No table holds {field}.")))?;
            let mut rows = Value::Array(read_visible_rows(connection, table)?);
            normalize_optional_nulls(&mut rows);
            validate(&rows).map_err(|error| format!("workspace validation failed: {error}"))?;
            serde_json::to_writer(&mut digest, &rows)?;
            collections.push(rows);
        }
        Ok(Arc::new(Self {
            version,
            collections,
            digest: digest.finalize().into(),
        }))
    }
}

pub(super) struct SnapshotReader {
    /// A read connection of its own: data versions only compare between
    /// reads of one connection. Every commit lands on another connection
    /// (the store's write connection, or another store on the same files),
    /// so every commit changes them.
    connection: Connection,
    cached: Option<Cached>,
    shared: Option<Arc<SharedSchool>>,
}

/// The snapshot of `versions` on `day`, with its content revision.
struct Cached {
    versions: Versions,
    day: chrono::NaiveDate,
    parts: SnapshotParts,
    hash: String,
    /// Feeds `hash` with everything but the sync records.
    digest: [u8; 32],
    /// The shared collections its workspace holds.
    school: Arc<SharedSchool>,
}

/// A whole snapshot as one read built it.
pub(super) struct Built {
    parts: SnapshotParts,
    hash: String,
    digest: [u8; 32],
}

/// The content revision of a snapshot whose other content hashes to
/// `digest`.
fn content_hash(digest: &[u8; 32], sync_records: &Value) -> Result<String, DataError> {
    let mut hash = Sha256::new();
    hash.update(digest);
    serde_json::to_writer(&mut hash, sync_records)?;
    Ok(format!("{:x}", hash.finalize()))
}

impl SnapshotReader {
    pub(super) fn new(connection: Connection) -> Self {
        Self {
            connection,
            cached: None,
            shared: None,
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
        Ok((version("main")?, version("cache")?, version("sync")?))
    }

    fn baseline(
        &self,
        versions: Versions,
        today: chrono::NaiveDate,
    ) -> Option<(SnapshotParts, String)> {
        self.cached
            .as_ref()
            .filter(|cached| cached.versions == versions && cached.day == today)
            .map(|cached| (cached.parts.clone(), cached.hash.clone()))
    }

    /// The cached workspace when user.db and cache.db are as it read them:
    /// sync.db feeds no workspace field.
    fn workspace_at(&self, versions: Versions, today: chrono::NaiveDate) -> Option<Arc<Value>> {
        self.cached
            .as_ref()
            .filter(|cached| {
                (cached.versions.0, cached.versions.1) == (versions.0, versions.1)
                    && cached.day == today
            })
            .map(|cached| cached.parts.0.clone())
    }

    /// The cached snapshot with the sync records of `versions` read again,
    /// when only sync.db changed since it was read.
    fn refresh_sync_records(
        &mut self,
        versions: Versions,
        today: chrono::NaiveDate,
    ) -> Result<Option<(SnapshotParts, String)>, DataError> {
        let Self {
            connection, cached, ..
        } = self;
        let Some(cached) = cached.as_mut().filter(|cached| {
            (cached.versions.0, cached.versions.1) == (versions.0, versions.1)
                && cached.day == today
        }) else {
            return Ok(None);
        };
        let transaction = connection.transaction()?;
        let sync_records = DataStore::sync_records_from(&transaction)?;
        transaction.commit()?;
        let mut records = Value::clone(&cached.parts.1);
        for field in SYNC_RECORDS {
            records[field] = sync_records[field].clone();
        }
        cached.hash = content_hash(&cached.digest, &sync_records)?;
        cached.parts.1 = Arc::new(records);
        cached.versions = versions;
        Ok(Some((cached.parts.clone(), cached.hash.clone())))
    }

    /// The cached workspace, taken out of the cache, when it holds `school`'s
    /// collections and nothing else holds it.
    fn take_workspace(&mut self, school: &Arc<SharedSchool>) -> Option<Value> {
        let cached = self
            .cached
            .take_if(|cached| Arc::ptr_eq(&cached.school, school))?;
        Arc::try_unwrap(cached.parts.0).ok()
    }
}

impl DataStore {
    pub fn load_workspace(&self) -> Result<Value, DataError> {
        self.workspace_snapshot()
            .map(|((workspace, ..), _)| Value::clone(&workspace))
    }

    /// The workspace and the school records read models evaluate with it,
    /// from one read, shared with the snapshot cache.
    pub fn load_reads(&self) -> Result<(Arc<Value>, Arc<SchoolReads>), DataError> {
        self.workspace_snapshot()
            .map(|((workspace, _, _, school), _)| (workspace, school))
    }

    /// `(workspace, school records, user-intent revision)` and the content
    /// revision, from one read. Cached until a commit or the day changes.
    pub(crate) fn workspace_snapshot(&self) -> Result<(SnapshotParts, String), DataError> {
        let mut reader = self
            .snapshot_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let today = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        // Observed before the read: a commit landing during the read must
        // invalidate this snapshot rather than be stamped onto old data.
        let versions = reader.versions()?;
        if let Some(snapshot) = reader.baseline(versions, today) {
            if std::env::var_os("ISCT_TRACE_READS").is_some() {
                eprintln!("[workspace-snapshot] cache=hit data_version={versions:?}");
            }
            return Ok(snapshot);
        }
        if let Some(snapshot) = reader.refresh_sync_records(versions, today)? {
            if std::env::var_os("ISCT_TRACE_READS").is_some() {
                eprintln!("[workspace-snapshot] cache=sync-records data_version={versions:?}");
            }
            return Ok(snapshot);
        }
        if std::env::var_os("ISCT_TRACE_READS").is_some() {
            eprintln!("[workspace-snapshot] cache=miss data_version={versions:?}");
        }
        let SnapshotReader {
            connection, shared, ..
        } = &mut *reader;
        let transaction = connection.transaction()?;
        let school = SharedSchool::read(&transaction, shared.as_ref(), Some(versions.1))?;
        let built = Self::snapshot_from(&transaction, &school, None)?;
        transaction.commit()?;
        *shared = Some(school.clone());
        let snapshot = (built.parts.clone(), built.hash.clone());
        reader.cached = Some(Cached {
            versions,
            day: today,
            parts: built.parts,
            hash: built.hash,
            digest: built.digest,
            school,
        });
        Ok(snapshot)
    }

    /// Applies one user intent after checking the revision it was made
    /// against, and returns its result with the snapshot as committed and
    /// the tables the commit changed; the snapshot's revision reaches the
    /// observer. `operation` reads the workspace as of the write lock.
    pub(crate) fn workspace_write<T>(
        &self,
        expected_revision: &str,
        operation: impl FnOnce(&WorkspaceWrite<'_, '_>, &Value) -> Result<T, DataError>,
    ) -> Result<(T, SnapshotParts, String, BTreeSet<String>), DataError> {
        // Lock order: snapshot reader, then the write connection.
        let mut reader = self
            .snapshot_reader
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let today = (Utc::now() + chrono::Duration::hours(9)).date_naive();
        let mut writer = self.connections.writer();
        let ((result, snapshot, school, versions, previous), tables) =
            commit_on(&mut writer, |transaction| {
                // Under the write lock no other commit can land, so these are
                // the versions of the state this transaction starts from.
                let versions = reader.versions()?;
                let actual = intent_revision(transaction)?;
                if actual != expected_revision {
                    return Err(DataError::conflict(format!(
                        "workspace revision conflict: expected {expected_revision}, actual {actual}"
                    )));
                }
                // A user intent writes no cache.db table, so the shared school
                // collections read at this version stay those of the commit.
                let school =
                    SharedSchool::read(transaction, reader.shared.as_ref(), Some(versions.1))?;
                let current = match reader.workspace_at(versions, today) {
                    Some(workspace) => workspace,
                    None => Self::snapshot_from(transaction, &school, None)?.parts.0,
                };
                let write = WorkspaceWrite {
                    transaction,
                    actor: WorkspaceChangeSource::UserIntent.actor(),
                };
                let result = operation(&write, &current)?;
                drop(current);
                // The commit replaces the cached snapshot; its collections
                // move into the new one rather than being copied.
                let mut previous = reader.take_workspace(&school);
                let snapshot = Self::snapshot_from(transaction, &school, previous.as_mut())?;
                Ok((result, snapshot, school, versions, previous))
            })?;
        let revision = snapshot.parts.2.clone();
        // The committed snapshot is the one for the versions after this
        // commit. They are read before the write lock is released, so no
        // commit of this store can land in between; only another process
        // writing the same files could, and the app opens one store per
        // data folder. Without versions the next read builds it afresh.
        let after = reader.versions().ok();
        if after.is_some_and(|after| after.1 == versions.1) {
            reader.shared = Some(school.clone());
        }
        let replaced = std::mem::replace(
            &mut reader.cached,
            after.map(|after| Cached {
                versions: after,
                day: today,
                parts: snapshot.parts.clone(),
                hash: snapshot.hash.clone(),
                digest: snapshot.digest,
                school,
            }),
        );
        drop(writer);
        drop(reader);
        // Freed outside the locks.
        drop((previous, replaced));
        self.committed_tables(&tables);
        self.observer
            .committed(WorkspaceChangeSource::UserIntent, Some(revision), &tables);
        Ok((result, snapshot.parts, snapshot.hash, tables))
    }

    /// The whole snapshot as `connection` sees it, with `school`'s shared
    /// collections, and its content revision. They move out of `previous`,
    /// a workspace built with them that is no longer needed, when given.
    pub(super) fn snapshot_from(
        connection: &Connection,
        school: &SharedSchool,
        mut previous: Option<&mut Value>,
    ) -> Result<Built, DataError> {
        let (mut workspace, catalog) = Self::workspace_without_shared(connection)?;
        let mut school_records = Self::school_records_from(connection)?;
        let sync_records = Self::sync_records_from(connection)?;
        let revision = intent_revision(connection)?;
        let mut digest = Sha256::new();
        digest.update(school.digest);
        serde_json::to_writer(&mut digest, &(&workspace, &school_records, &catalog))?;
        let digest: [u8; 32] = digest.finalize().into();
        let hash = content_hash(&digest, &sync_records)?;
        for field in SYNC_RECORDS {
            school_records[field] = sync_records[field].clone();
        }
        for ((field, _), rows) in SHARED_SCHOOL.iter().zip(&school.collections) {
            workspace[*field] = match previous.as_mut().and_then(|old| old.get_mut(*field)) {
                Some(moved) => moved.take(),
                None => rows.clone(),
            };
        }
        // Schedule read models evaluate these school records with the
        // workspace; carrying them saves every query a copy of it.
        for field in ["registrations", "grades", "lmsCourses"] {
            workspace[field] = school_records[field].clone();
        }
        Ok(Built {
            parts: (
                Arc::new(workspace),
                Arc::new(school_records),
                revision,
                Arc::new(SchoolReads { catalog }),
            ),
            hash,
            digest,
        })
    }

    /// The workspace as `connection` sees it, read in full.
    pub(super) fn load_workspace_from(connection: &Connection) -> Result<Arc<Value>, DataError> {
        let school = SharedSchool::read(connection, None, None)?;
        Ok(Self::snapshot_from(connection, &school, None)?.parts.0)
    }

    /// The workspace with the shared school collections left empty, the
    /// rest normalized and validated, and what its courses show of the
    /// catalog.
    fn workspace_without_shared(
        connection: &Connection,
    ) -> Result<(Value, BTreeMap<String, CatalogEntry>), DataError> {
        let mut workspace = empty_workspace();
        for (field, table) in COLLECTIONS {
            workspace[*field] = if SHARED_SCHOOL.iter().any(|(shared, _)| shared == field) {
                json!([])
            } else {
                Value::Array(read_visible_rows(connection, table)?)
            };
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
            workspace["settings"]["cloud"] = value;
        }
        let catalog = school_projection::project(connection, &mut workspace)?;
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
        crate::contracts::Workspace::deserialize(&workspace)
            .map_err(|error| format!("workspace validation failed: {error}"))?;
        Ok((workspace, catalog))
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
        Ok(Value::Object(records))
    }

    /// The school records sync.db feeds, `SYNC_RECORDS`.
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
