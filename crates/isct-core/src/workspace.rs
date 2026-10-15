//! Authoritative workspace snapshots and user intents.
//!
//! A mutation checks the revision and applies every row change in one SQLite
//! IMMEDIATE transaction.  The returned snapshot is assembled and validated
//! before that transaction commits.

use crate::data::{DataError, DataStore, WorkspaceWrite};
use crate::schedule::ClassGroup;
use crate::study_profile::StudyEnrollment;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;
#[cfg(feature = "ts")]
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    /// Baseline for `WorkspaceMutation::expected_revision`. It covers only
    /// user-owned state, so school and reference ingests do not cause
    /// conflicts.
    pub revision: String,
    /// Changes whenever any part of the snapshot changes; read-model caches
    /// key on it.
    pub content_revision: String,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub workspace: Value,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub school_records: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMutation {
    pub expected_revision: String,
    pub intent: WorkspaceIntent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WorkspaceIntent {
    /// The complete settings object from a settings form. Absent optional
    /// keys are cleared; Core keeps the fields it owns (poll clock, agent
    /// write history) and validates every value.
    ReplaceSettings {
        #[cfg_attr(feature = "ts", ts(type = "Settings"))]
        settings: Value,
    },
    SetAdmissionYear {
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        admission_year: Option<i32>,
    },
    /// Saves the guide cohort and the complete, ordered degree-stage history.
    /// `admissionYear` remains a reference-data cohort and never implies a
    /// degree stage.
    SetStudyProfile {
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        admission_year: Option<i32>,
        study_enrollments: Vec<StudyEnrollment>,
    },
    EditAcademicCalendar {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        academic_profile: Value,
        #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
        exceptions: Vec<Value>,
    },
    SelectTerm {
        term_id: String,
    },
    AddTerm {
        term_id: String,
    },
    RemoveTerm {
        term_id: String,
    },
    SaveCourse {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        term_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    DeleteCourseBatch {
        course_ids: Vec<String>,
    },
    EditCourseBatch {
        course_ids: Vec<String>,
        changes: CourseBatchChanges,
    },
    SetCourseOnTimetable {
        course_id: String,
        on_timetable: bool,
    },
    AddCatalogCourses {
        term_id: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
        offerings: Vec<Value>,
    },
    UndoCatalogCourses {
        term_id: String,
        offering_ids: Vec<String>,
    },
    ApplyLmsMerge {
        terms: Vec<String>,
        add: Vec<String>,
        keep: Vec<String>,
        retime: Vec<String>,
    },
    SaveEvent {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        term_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    DeleteEvent {
        event_id: String,
    },
    SavePlan {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    DeletePlan {
        plan_id: String,
    },
    SaveCalendarException {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    DeleteCalendarException {
        exception_id: String,
    },
    SaveSubscription {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    SetSubscription {
        subscription_kind: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        enabled: Option<bool>,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        url: Option<String>,
        #[serde(default)]
        reset_remote: bool,
    },
    DeleteSubscription {
        subscription_id: String,
    },
    SaveScheduleOverride {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    DeleteScheduleOverride {
        override_id: String,
    },
    SaveScheduledTask {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        existing_id: Option<String>,
    },
    DeleteScheduledTask {
        task_id: String,
    },
    RecordScheduledTaskRun {
        task_id: String,
        occurrence_at: String,
        run_at: String,
        result: String,
    },
    SaveIntensiveSchedule {
        course_id: String,
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        draft: Value,
    },
    SetAssignmentMark {
        assignment_id: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        hidden: Option<bool>,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        pinned: Option<bool>,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        completed_at: Option<String>,
    },
    RecordLesson {
        course_id: String,
        occurrence: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        section_id: u64,
    },
    MarkTimetableReviewed,
    /// Chooses (or clears) the 物理学演習・実験 group of one semester,
    /// such as "2026-3Q4Q".
    SetClassGroup {
        semester: String,
        group: Option<ClassGroup>,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseBatchChanges {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub language: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub instructor: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub credits: Option<f64>,
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    InvalidIntent(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Storage(String),
}

impl From<DataError> for WorkspaceError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::InvalidInput(message) | DataError::Compatibility(message) => {
                Self::InvalidIntent(message)
            }
            DataError::Conflict(message) => Self::Conflict(message),
            DataError::Storage(message) => Self::Storage(message),
        }
    }
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        WorkspaceSnapshot::decl(),
        WorkspaceMutation::decl(),
        WorkspaceIntent::decl(),
        CourseBatchChanges::decl(),
    ]
}

#[derive(Clone)]
pub struct WorkspaceService {
    store: DataStore,
    events: Option<Arc<dyn crate::ports::EventSink>>,
    /// What the last `AddCatalogCourses` placed and replaced, so
    /// `UndoCatalogCourses` reverses exactly that add.  Like the finder's undo
    /// toast it lasts for this session only.
    catalog_undo: Arc<Mutex<Option<CatalogAddReceipt>>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct CatalogAddReceipt {
    term_id: String,
    /// Sorted offering ids the add was asked for.
    offering_ids: Vec<String>,
    /// Plan items the add put on the timetable (new or brought back).
    placed: Vec<String>,
    /// Plan items the add took off because their course clashed.
    replaced: Vec<String>,
}

impl WorkspaceService {
    pub fn new(store: DataStore) -> Self {
        Self {
            store,
            events: None,
            catalog_undo: Arc::default(),
        }
    }

    pub fn with_events(store: DataStore, events: Arc<dyn crate::ports::EventSink>) -> Self {
        Self {
            store,
            events: Some(events),
            catalog_undo: Arc::default(),
        }
    }

    pub fn snapshot(&self) -> Result<WorkspaceSnapshot, WorkspaceError> {
        let (workspace, school_records, revision) = self.store.workspace_snapshot_parts()?;
        snapshot(workspace, school_records, revision).map_err(WorkspaceError::from)
    }

    pub fn mutate(&self, mutation: WorkspaceMutation) -> Result<WorkspaceSnapshot, WorkspaceError> {
        let mut undo = self
            .catalog_undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let result = self.store.workspace_write(
            &mutation.expected_revision,
            |write, current, school_records| {
                apply_intent(write, current, school_records, mutation.intent, &mut undo)?;
                let (workspace, school_records, revision) = write.snapshot_parts()?;
                snapshot(workspace, school_records, revision)
            },
        )?;
        // Only a committed add or undo changes what Undo reverses.
        *self
            .catalog_undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = undo;
        if let Some(events) = &self.events {
            let _ = events.emit(crate::events::CoreEvent::WorkspaceChanged {
                source: crate::events::WorkspaceChangeSource::UserIntent,
                revision: Some(result.revision.clone()),
            });
        }
        Ok(result)
    }
}

fn snapshot(
    workspace: Value,
    school_records: Value,
    revision: String,
) -> Result<WorkspaceSnapshot, DataError> {
    let bytes = serde_json::to_vec(&(&workspace, &school_records)).map_err(DataError::from)?;
    Ok(WorkspaceSnapshot {
        revision,
        content_revision: format!("{:x}", Sha256::digest(bytes)),
        workspace,
        school_records,
    })
}

fn apply_intent(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    school_records: &Value,
    intent: WorkspaceIntent,
    catalog_undo: &mut Option<CatalogAddReceipt>,
) -> Result<(), DataError> {
    match intent {
        WorkspaceIntent::ReplaceSettings { mut settings } => {
            require_object(&settings, "settings")?;
            for (parent, field) in [
                ("/subscriptionRefresh", "lastCheckedAt"),
                ("/agentAccess", "lastWrites"),
            ] {
                let owned = current
                    .pointer(&format!("/settings{parent}/{field}"))
                    .cloned();
                if let Some(parent) = settings.pointer_mut(parent).and_then(Value::as_object_mut) {
                    match owned {
                        Some(value) => parent.insert(field.into(), value),
                        None => parent.remove(field),
                    };
                }
            }
            write.save_settings(&settings)
        }
        WorkspaceIntent::SetAdmissionYear { admission_year } => {
            let mut profile = current
                .get("academicProfile")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let profile = object_mut(&mut profile, "academic profile")?;
            match admission_year {
                Some(year) => {
                    profile.insert("admissionYear".into(), Value::from(year));
                }
                None => {
                    profile.remove("admissionYear");
                }
            }
            write.save_setting("academicProfile", &Value::Object(profile.clone()))
        }
        WorkspaceIntent::SetStudyProfile {
            admission_year,
            mut study_enrollments,
        } => {
            crate::study_profile::canonicalize_study_enrollments(&mut study_enrollments)
                .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            let mut profile = current
                .get("academicProfile")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let profile = object_mut(&mut profile, "academic profile")?;
            match admission_year {
                Some(year) => {
                    profile.insert("admissionYear".into(), Value::from(year));
                }
                None => {
                    profile.remove("admissionYear");
                }
            }
            profile.insert(
                "studyEnrollments".into(),
                serde_json::to_value(study_enrollments)?,
            );
            write.save_setting("academicProfile", &Value::Object(profile.clone()))
        }
        WorkspaceIntent::EditAcademicCalendar {
            mut academic_profile,
            exceptions,
        } => {
            require_object(&academic_profile, "academic profile")?;
            // Older shells do not know the degree-stage history. A calendar
            // edit from one must not erase identity saved by a newer shell.
            if academic_profile.get("studyEnrollments").is_none() {
                if let Some(history) = current.pointer("/academicProfile/studyEnrollments") {
                    academic_profile["studyEnrollments"] = history.clone();
                }
            }
            write.save_setting("academicProfile", &academic_profile)?;
            replace_records(
                write,
                current,
                "exceptions",
                "calendar_exceptions",
                exceptions,
            )
        }
        WorkspaceIntent::SelectTerm { term_id } => {
            validate_term(&term_id)?;
            write.save_setting("selectedTerm", &Value::String(term_id))
        }
        WorkspaceIntent::AddTerm { term_id } => edit_terms(write, current, term_id, true),
        WorkspaceIntent::RemoveTerm { term_id } => edit_terms(write, current, term_id, false),
        WorkspaceIntent::SaveCourse {
            draft,
            term_id,
            existing_id,
        } => save_course(write, current, draft, term_id, existing_id),
        WorkspaceIntent::DeleteCourseBatch { course_ids } => {
            delete_course_batch(write, current, course_ids)
        }
        WorkspaceIntent::EditCourseBatch {
            course_ids,
            changes,
        } => edit_course_batch(write, current, course_ids, changes),
        WorkspaceIntent::SetCourseOnTimetable {
            course_id,
            on_timetable,
        } => set_course_on_timetable(write, current, &course_id, on_timetable),
        WorkspaceIntent::AddCatalogCourses { term_id, offerings } => {
            *catalog_undo = Some(add_catalog_courses(write, current, &term_id, offerings)?);
            Ok(())
        }
        WorkspaceIntent::UndoCatalogCourses {
            term_id,
            offering_ids,
        } => {
            let receipt = catalog_undo.take();
            undo_catalog_courses(write, current, &term_id, offering_ids, receipt)
        }
        WorkspaceIntent::ApplyLmsMerge {
            terms,
            add,
            keep,
            retime,
        } => apply_lms_merge(write, current, school_records, terms, add, keep, retime),
        WorkspaceIntent::SaveEvent {
            draft,
            term_id,
            existing_id,
        } => save_simple_record(
            write,
            current,
            "events",
            "personal_events",
            draft,
            existing_id,
            |row| {
                validate_term(&term_id)?;
                row.insert("kind".into(), Value::String("event".into()));
                row.insert("termId".into(), Value::String(term_id));
                Ok(())
            },
        ),
        WorkspaceIntent::DeleteEvent { event_id } => {
            delete_visible(write, current, "events", "personal_events", &event_id)
        }
        WorkspaceIntent::SavePlan {
            mut draft,
            existing_id,
        } => {
            crate::plans::canonicalize_plan_draft(&mut draft)
                .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_simple_record(write, current, "plans", "plans", draft, existing_id, |_| {
                Ok(())
            })
        }
        WorkspaceIntent::DeletePlan { plan_id } => {
            delete_visible(write, current, "plans", "plans", &plan_id)
        }
        WorkspaceIntent::SaveCalendarException { draft, existing_id } => {
            save_calendar_exception(write, current, draft, existing_id)
        }
        WorkspaceIntent::DeleteCalendarException { exception_id } => delete_visible(
            write,
            current,
            "exceptions",
            "calendar_exceptions",
            &exception_id,
        ),
        WorkspaceIntent::SaveSubscription { draft, existing_id } => save_simple_record(
            write,
            current,
            "subscriptions",
            "subscriptions",
            draft,
            existing_id,
            |_| Ok(()),
        ),
        WorkspaceIntent::SetSubscription {
            subscription_kind,
            enabled,
            url,
            reset_remote,
        } => set_subscription(
            write,
            current,
            subscription_kind,
            enabled,
            url,
            reset_remote,
        ),
        WorkspaceIntent::DeleteSubscription { subscription_id } => delete_visible(
            write,
            current,
            "subscriptions",
            "subscriptions",
            &subscription_id,
        ),
        WorkspaceIntent::SaveScheduleOverride { draft, existing_id } => save_simple_record(
            write,
            current,
            "scheduleOverrides",
            "schedule_overrides",
            draft,
            existing_id,
            |_| Ok(()),
        ),
        WorkspaceIntent::DeleteScheduleOverride { override_id } => delete_visible(
            write,
            current,
            "scheduleOverrides",
            "schedule_overrides",
            &override_id,
        ),
        WorkspaceIntent::SaveScheduledTask { draft, existing_id } => save_simple_record(
            write,
            current,
            "scheduledTasks",
            "scheduled_tasks",
            draft,
            existing_id,
            |_| Ok(()),
        ),
        WorkspaceIntent::DeleteScheduledTask { task_id } => delete_visible(
            write,
            current,
            "scheduledTasks",
            "scheduled_tasks",
            &task_id,
        ),
        WorkspaceIntent::RecordScheduledTaskRun {
            task_id,
            occurrence_at,
            run_at,
            result,
        } => record_scheduled_task_run(write, current, &task_id, occurrence_at, run_at, result),
        WorkspaceIntent::SaveIntensiveSchedule { course_id, draft } => {
            save_intensive(write, current, &course_id, draft)
        }
        WorkspaceIntent::SetAssignmentMark {
            assignment_id,
            hidden,
            pinned,
            completed_at,
        } => set_assignment_mark(write, current, &assignment_id, hidden, pinned, completed_at),
        WorkspaceIntent::RecordLesson {
            course_id,
            occurrence,
            section_id,
        } => record_lesson(write, current, &course_id, occurrence, section_id),
        WorkspaceIntent::MarkTimetableReviewed => {
            let mut status = current
                .get("importStatus")
                .cloned()
                .unwrap_or_else(|| json!({}));
            object_mut(&mut status, "import status")?.insert(
                "timetableReviewedAt".into(),
                Value::String(
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                ),
            );
            write.save_setting("importStatus", &status)
        }
        WorkspaceIntent::SetClassGroup { semester, group } => {
            crate::schedule::validate_semester(&semester)
                .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            let mut groups = current
                .get("classGroups")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let groups_object = object_mut(&mut groups, "class groups")?;
            match group {
                Some(group) => {
                    groups_object.insert(semester, serde_json::to_value(group)?);
                }
                None => {
                    groups_object.remove(&semester);
                }
            }
            write.save_setting("classGroups", &groups)
        }
    }
}

fn validate_term(term: &str) -> Result<(), DataError> {
    let bytes = term.as_bytes();
    let valid = bytes.len() == 7
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && matches!(&bytes[5..], b"1Q" | b"2Q" | b"3Q" | b"4Q");
    if valid {
        Ok(())
    } else {
        Err("term must use YYYY-1Q..4Q".into())
    }
}

fn edit_terms(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: String,
    add: bool,
) -> Result<(), DataError> {
    validate_term(&term_id)?;
    let mut terms = current
        .get("terms")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    terms.retain(|term| term.as_str() != Some(&term_id));
    if add {
        terms.push(Value::String(term_id.clone()));
        terms.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
        write.save_setting("selectedTerm", &Value::String(term_id))?;
    } else if current.get("selectedTerm").and_then(Value::as_str) == Some(&term_id) {
        return Err("the selected term cannot be removed".into());
    }
    write.save_setting("terms", &Value::Array(terms))
}

fn save_course(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    draft: Value,
    term_id: String,
    existing_id: Option<String>,
) -> Result<(), DataError> {
    validate_term(&term_id)?;
    let draft = require_object(&draft, "course draft")?;
    let existing = existing_id
        .as_deref()
        .and_then(|id| visible_row(current, "courses", id));
    let existing_rev = existing
        .and_then(|row| row.get("rev"))
        .and_then(Value::as_u64);
    if let Some(existing) = existing
        .filter(|row| row.pointer("/source/provider").and_then(Value::as_str) != Some("manual"))
    {
        put_course_override(
            write,
            existing,
            ["title", "instructor", "color"]
                .map(|key| (key, draft.get(key).cloned().unwrap_or(Value::Null))),
        )?;
        return set_course_on_timetable(
            write,
            current,
            string(existing, "id")?,
            draft
                .get("onTimetable")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        );
    }
    let mut course = existing.cloned().unwrap_or_else(|| {
        json!({
            "id": Uuid::now_v7().to_string(), "kind":"course", "rev":0,
            "updatedAt":"", "updatedBy":"", "termId":term_id,
            "code":"", "title":"", "instructor":"", "credits":0,
            "creditSplit":{"lecture":0,"exercise":0,"lab":0},
            "instructionLanguage":"other", "color":"#526e91", "meetings":[],
            "source":{"provider":"manual"}
        })
    });
    let object = object_mut(&mut course, "course")?;
    for key in [
        "code",
        "title",
        "instructor",
        "credits",
        "instructionLanguage",
        "color",
        "meetings",
        "url",
    ] {
        if let Some(value) = draft.get(key) {
            object.insert(key.into(), value.clone());
        }
    }
    object.insert("termId".into(), Value::String(term_id.clone()));
    let credits = object.get("credits").and_then(Value::as_f64).unwrap_or(0.0);
    object.insert(
        "creditSplit".into(),
        json!({"lecture":credits,"exercise":0,"lab":0}),
    );
    let course_id = string(&course, "id")?.to_string();
    write.put("manual_courses", course, existing_rev)?;
    let on_timetable = draft
        .get("onTimetable")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if visible_row(current, "courses", &course_id).is_some() {
        set_course_on_timetable(write, current, &course_id, on_timetable)?;
    } else if on_timetable {
        write.put(
            "manual_memberships",
            json!({"id":Uuid::now_v7().to_string(),"kind":"membership","rev":0,"updatedAt":"","updatedBy":"","termId":term_id,"courseId":course_id,"status":"declared","evidence":{"provider":"manual"}}),
            None,
        )?;
    }
    Ok(())
}

/// Deletes the selected courses, whatever added them, with every quarter
/// they are placed in.
fn delete_course_batch(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: Vec<String>,
) -> Result<(), DataError> {
    let ids: HashSet<_> = course_ids.into_iter().collect();
    for course in rows(current, "courses")?.filter(|row| {
        is_live(row)
            && row
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ids.contains(id))
    }) {
        let id = string(course, "id")?;
        // Courses the school cache lists are not user rows; delete reports
        // false for them and their placements still go.
        write.delete("manual_courses", id, None)?;
        remove_placements(write, current, id, None)?;
    }
    Ok(())
}

/// Tombstones the live memberships and plan items that put a course on the
/// timetable: in `term`, or in every quarter it runs in.
fn remove_placements(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    term: Option<&str>,
) -> Result<(), DataError> {
    for (field, table) in [
        ("memberships", "manual_memberships"),
        ("planItems", "plan_items"),
    ] {
        for row in rows(current, field)?.filter(|row| {
            is_live(row)
                && row.get("courseId").and_then(Value::as_str) == Some(course_id)
                && term.is_none_or(|term| row.get("termId").and_then(Value::as_str) == Some(term))
        }) {
            write.delete(table, string(row, "id")?, None)?;
        }
    }
    Ok(())
}

fn is_live(row: &Value) -> bool {
    row.get("deletedAt").is_none_or(Value::is_null)
}

fn edit_course_batch(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: Vec<String>,
    changes: CourseBatchChanges,
) -> Result<(), DataError> {
    if changes
        .credits
        .is_some_and(|credits| !credits.is_finite() || !(0.0..=20.0).contains(&credits))
    {
        return Err("credits must be between 0 and 20".into());
    }
    let language = changes
        .language
        .as_deref()
        .map(str::trim)
        .filter(|language| !language.is_empty());
    if language.is_some_and(|language| !matches!(language, "ja" | "en" | "ja-en" | "other")) {
        return Err("instruction language is invalid".into());
    }
    let instructor = changes
        .instructor
        .as_deref()
        .map(str::trim)
        .filter(|instructor| !instructor.is_empty());
    let ids: HashSet<_> = course_ids.into_iter().collect();
    for course in rows(current, "courses")?.filter(|row| {
        row.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(id) && row.get("deletedAt").is_none_or(Value::is_null))
    }) {
        let id = string(course, "id")?;
        let manual = course.pointer("/source/provider").and_then(Value::as_str) == Some("manual");
        // Catalog and LMS courses keep their source instructor; the person's
        // name for it is an overlay like the course editor writes.
        if let Some(instructor) = instructor.filter(|_| !manual) {
            put_course_override(write, course, [("instructor", Value::from(instructor))])?;
        }
        if language.is_none() && changes.credits.is_none() && (instructor.is_none() || !manual) {
            continue;
        }
        if !write.has_user_row("manual_courses", id)? {
            return Err(
                "credits and language of a school timetable course cannot be edited".into(),
            );
        }
        let mut updated = course.clone();
        let object = object_mut(&mut updated, "course")?;
        // The read model embeds the overlay; the stored row never contains it.
        object.remove("overrides");
        if let Some(language) = language {
            object.insert("instructionLanguage".into(), Value::from(language));
        }
        if let Some(instructor) = instructor.filter(|_| manual) {
            object.insert("instructor".into(), Value::from(instructor));
        }
        if let Some(credits) = changes.credits {
            object.insert("credits".into(), Value::from(credits));
        }
        write.put("manual_courses", updated, revision(course))?;
    }
    Ok(())
}

/// One overlay row holds every personal change to a non-manual course. Each
/// edit replaces only its own fields and keeps the others.
fn put_course_override<const N: usize>(
    write: &WorkspaceWrite<'_, '_>,
    course: &Value,
    changes: [(&str, Value); N],
) -> Result<(), DataError> {
    let id = string(course, "id")?;
    let mut overlay = course
        .get("overrides")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, value) in changes {
        overlay.insert(key.into(), value);
    }
    overlay.insert(
        "sourceKey".into(),
        Value::from(
            course
                .get("sourceKey")
                .and_then(Value::as_str)
                .unwrap_or(id),
        ),
    );
    overlay.insert("courseId".into(), Value::from(id));
    write
        .put("course_overrides", Value::Object(overlay), None)
        .map(|_| ())
}

fn set_course_on_timetable(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    on: bool,
) -> Result<(), DataError> {
    let course = visible_row(current, "courses", course_id)
        .ok_or_else(|| "course was not found".to_string())?;
    if !on {
        // Off takes the course out of every quarter it runs in (3Q–4Q
        // alike), whoever placed it there.
        return remove_placements(write, current, course_id, None);
    }
    let placed = rows(current, "memberships")?
        .chain(rows(current, "planItems")?)
        .any(|row| is_live(row) && row.get("courseId").and_then(Value::as_str) == Some(course_id));
    if !placed {
        let term = string(course, "termId")?;
        write.put("manual_memberships", json!({"id":Uuid::now_v7().to_string(),"kind":"membership","rev":0,"updatedAt":"","updatedBy":"","termId":term,"courseId":course_id,"status":"declared","evidence":{"provider":"manual"}}), None)?;
    }
    Ok(())
}

/// Adds catalog offerings to the timetable of `term_id`, the quarter on
/// screen.  An offering running over several quarters (3Q–4Q) that is added
/// in one of them is planned into each of them; added outside its quarters
/// it stays in `term_id`.  A local draft (plan item) in `term_id` whose
/// periods clash leaves the timetable; school-registered courses stay.
fn catalog_plan_terms(offering: &Value, term_id: &str) -> Result<Vec<String>, DataError> {
    validate_term(term_id)?;
    let (plan_year, quarter) = (&term_id[..4], &term_id[5..]);
    let quarters = offering
        .get("quarters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if !quarters.contains(&quarter) {
        return Ok(vec![term_id.to_string()]);
    }
    let terms = quarters
        .iter()
        .map(|quarter| format!("{plan_year}-{quarter}"))
        .filter(|term| validate_term(term).is_ok())
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err(DataError::InvalidInput(
            "catalog offering has no valid teaching quarter".into(),
        ));
    }
    Ok(terms)
}

fn add_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    offerings: Vec<Value>,
) -> Result<CatalogAddReceipt, DataError> {
    validate_term(term_id)?;
    let offerings = offerings
        .iter()
        .map(|submitted| {
            let id = string(submitted, "id")?;
            write.catalog_row(id)?.ok_or_else(|| {
                DataError::InvalidInput(format!("catalog offering {id} is no longer available"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for offering in &offerings {
        for plan_term in catalog_plan_terms(offering, term_id)? {
            let eligibility =
                crate::study_profile::catalog_course_eligibility(current, &plan_term, offering);
            if let Some(message) =
                crate::study_profile::ineligibility_message(offering, eligibility)
            {
                return Err(DataError::InvalidInput(message));
            }
        }
    }
    let locale = workspace_content_locale(current);
    let plan_year = &term_id[..4];
    let mut offering_ids = offerings
        .iter()
        .map(|offering| string(offering, "id").map(str::to_string))
        .collect::<Result<Vec<_>, _>>()?;
    offering_ids.sort();
    offering_ids.dedup();
    let mut receipt = CatalogAddReceipt {
        term_id: term_id.to_string(),
        offering_ids,
        ..CatalogAddReceipt::default()
    };
    // Working copies, so later offerings see what earlier ones changed.
    let mut courses: Vec<Value> = rows(current, "courses")?.cloned().collect();
    let mut plans: Vec<Value> = rows(current, "planItems")?.cloned().collect();
    let members: Vec<&Value> = rows(current, "memberships")?
        .filter(|row| is_live(row))
        .collect();
    for offering in offerings {
        let offering_id = string(&offering, "id")?;
        let year = offering
            .get("year")
            .and_then(Value::as_i64)
            .or_else(|| plan_year.parse().ok())
            .ok_or_else(|| "catalog offering is missing academic year".to_string())?;
        let course_id = offering_course_id(
            i32::try_from(year).map_err(|_| "academic year is invalid")?,
            offering_id,
        )?;
        let placed_here: HashSet<&str> = members
            .iter()
            .copied()
            .chain(plans.iter().filter(|row| is_live(row)))
            .filter(|row| row.get("termId").and_then(Value::as_str) == Some(term_id))
            .filter_map(|row| row.get("courseId").and_then(Value::as_str))
            .collect();
        let on_timetable: Vec<Value> = courses
            .iter()
            .filter(|course| {
                is_live(course)
                    && course
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| placed_here.contains(id))
            })
            .cloned()
            .collect();
        for victim in crate::course_query::clashing_courses(&offering, &on_timetable) {
            let Some(plan) = plans.iter_mut().find(|row| {
                is_live(row)
                    && row.get("courseId") == victim.get("id")
                    && row.get("termId").and_then(Value::as_str) == Some(term_id)
            }) else {
                continue;
            };
            let id = string(plan, "id")?.to_string();
            write.delete("plan_items", &id, revision(plan))?;
            plan["deletedAt"] = Value::String("replaced".into());
            receipt.replaced.push(id);
        }
        let live_course = courses.iter().find(|course| {
            is_live(course)
                && (course.get("id").and_then(Value::as_str) == Some(&course_id)
                    || course.get("offeringId").and_then(Value::as_str) == Some(offering_id))
        });
        // A course already in the workspace stays as it is (and in its own
        // quarter); only its plan items change.
        let course_id = match live_course {
            Some(course) => string(course, "id")?.to_string(),
            None => {
                let credits = offering
                    .get("credits")
                    .cloned()
                    .unwrap_or_else(|| json!({"lecture":0,"exercise":0,"lab":0}));
                let total = ["lecture", "exercise", "lab"]
                    .into_iter()
                    .filter_map(|key| credits.get(key).and_then(Value::as_f64))
                    .sum::<f64>();
                let instructor = offering
                    .get("instructors")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|value| localized(Some(value), locale))
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join(" / ");
                let tombstone = courses
                    .iter()
                    .find(|course| course.get("id").and_then(Value::as_str) == Some(&course_id));
                let course = write.put(
                    "manual_courses",
                    json!({
                        "id":course_id, "kind":"course", "termId":term_id, "offeringId":offering_id,
                        "names":{"title":offering.get("title"),"instructor":offering.get("instructors").and_then(Value::as_array).and_then(|items| items.first()).cloned().unwrap_or_else(|| json!({}))},
                        "code":offering.get("code").and_then(Value::as_str).unwrap_or_default(),
                        "title":localized(offering.get("title"), locale), "instructor":instructor, "credits":total,
                        "creditSplit":credits,
                        "instructionLanguage":normalize_language(offering.get("instructionLanguage").and_then(Value::as_str)),
                        "color":"#556b8d", "meetings":catalog_meetings(&offering, locale)?,
                        "url":offering.get("syllabusUrl"),
                        "source":{"provider":"syllabus","externalId":offering_id}
                    }),
                    tombstone.and_then(revision),
                )?;
                let id = string(&course, "id")?.to_string();
                courses.retain(|row| row.get("id").and_then(Value::as_str) != Some(&id));
                courses.push(course);
                id
            }
        };
        let plan_terms = catalog_plan_terms(&offering, term_id)?;
        for plan_term in plan_terms {
            let mine = |row: &&Value| {
                row.get("courseId").and_then(Value::as_str) == Some(&course_id)
                    && row.get("termId").and_then(Value::as_str) == Some(&plan_term)
            };
            if plans.iter().filter(mine).any(is_live) {
                continue;
            }
            // Bring back the latest removed draft before writing a new one.
            let removed = plans
                .iter()
                .filter(mine)
                .max_by(|a, b| {
                    a.get("deletedAt")
                        .and_then(Value::as_str)
                        .cmp(&b.get("deletedAt").and_then(Value::as_str))
                })
                .cloned();
            let plan = match removed {
                Some(row) => {
                    let rev = revision(&row);
                    write.put("plan_items", row, rev)?
                }
                None => write.put(
                    "plan_items",
                    json!({"kind":"plan-item","termId":plan_term,"courseId":course_id,"offeringId":offering_id,"state":"planned"}),
                    None,
                )?,
            };
            let id = string(&plan, "id")?.to_string();
            plans.retain(|row| row.get("id").and_then(Value::as_str) != Some(&id));
            plans.push(plan);
            receipt.placed.push(id);
        }
    }
    Ok(receipt)
}

/// Applies the person's LMS review choices to the diff Core recomputes from
/// the same catalog rows the preview used: adds the chosen LMS courses as
/// LMS memberships, refreshes LMS-sourced courses, retimes the chosen common
/// courses, drops the unchecked local ones from that quarter and links LMS
/// assignments and materials to the timetable course they belong to.
fn apply_lms_merge(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    school_records: &Value,
    terms: Vec<String>,
    add: Vec<String>,
    keep: Vec<String>,
    retime: Vec<String>,
) -> Result<(), DataError> {
    for term in &terms {
        validate_term(term)?;
    }
    let add: HashSet<_> = add.into_iter().collect();
    let keep: HashSet<_> = keep.into_iter().collect();
    let retime: HashSet<_> = retime.into_iter().collect();
    let offerings = write
        .catalog_rows()?
        .into_iter()
        .filter_map(crate::course_query::normalize_offering)
        .collect::<Vec<_>>();
    let locale = workspace_content_locale(current);
    let diff = lms_diff(current, school_records, &terms, &offerings, locale)?;
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let lms_id_of = |entry: &Value| {
        entry
            .pointer("/lms/lmsId")
            .and_then(Value::as_u64)
            .ok_or_else(|| DataError::from("LMS course is missing lmsId"))
    };
    let mut linked = HashMap::<u64, String>::new();
    for pair in &diff.common {
        linked.insert(
            lms_id_of(&pair["lms"])?,
            string(&pair["local"], "id")?.to_string(),
        );
    }
    let stored_courses: Vec<&Value> = rows(current, "courses")?.collect();
    // Course rows this merge changes, written once each at the end.
    let mut edited = HashMap::<String, (Value, Option<u64>)>::new();
    let mut created = HashSet::<String>::new();
    let memberships: Vec<&Value> = rows(current, "memberships")?.collect();
    let mut live_members: HashSet<(String, String)> = memberships
        .iter()
        .filter(|row| is_live(row))
        .filter_map(|row| {
            Some((
                row.get("courseId")?.as_str()?.to_string(),
                row.get("termId")?.as_str()?.to_string(),
            ))
        })
        .collect();

    for entry in diff.lms_only.iter().filter(|entry| {
        entry
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| add.contains(key))
    }) {
        let term = string(entry, "termId")?;
        let lms = &entry["lms"];
        let lms_id = lms_id_of(entry)?;
        let offering = entry.get("offering").filter(|value| value.is_object());
        let year: i32 = term[..4]
            .parse()
            .map_err(|_| DataError::from("term year is invalid"))?;
        let offering_id = lms.get("offeringId").and_then(Value::as_str);
        let mut id = match offering_id {
            Some(offering_id) => offering_course_id(year, offering_id)?,
            None => format!("{year}-lms-{lms_id}"),
        };
        match stored_courses
            .iter()
            .find(|course| course.get("id").and_then(Value::as_str) == Some(&id))
        {
            _ if created.contains(&id) => {}
            Some(course) if is_live(course) => {}
            Some(course) => {
                write.put("manual_courses", stored_course(course), revision(course))?;
            }
            None => {
                let syllabus = offering
                    .map(|offering| catalog_meetings(offering, locale))
                    .transpose()?
                    .unwrap_or_else(|| json!([]));
                let meetings = match entry.get("gridMeetings").filter(|grid| !grid.is_null()) {
                    Some(grid) => meetings_from_grid(grid, &syllabus)?,
                    None => syllabus,
                };
                let credits = offering
                    .and_then(|offering| offering.get("credits"))
                    .cloned()
                    .unwrap_or_else(|| json!({"lecture":0,"exercise":0,"lab":0}));
                let instructors = offering
                    .and_then(|offering| offering.get("instructors"))
                    .and_then(Value::as_array);
                let mut course = json!({
                    "id":id, "kind":"course", "termId":term,
                    "names":{
                        "title":offering.and_then(|offering| offering.get("title")).or_else(|| lms.get("title")),
                        "instructor":instructors.and_then(|items| items.first()).cloned().unwrap_or_else(|| json!({}))
                    },
                    "code":entry.get("code"), "title":entry.get("title"),
                    "instructor":instructors.into_iter().flatten()
                        .map(|value| localized(Some(value), locale))
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>().join(" / "),
                    "credits":credit_total(&credits), "creditSplit":credits,
                    "instructionLanguage":normalize_language(offering.and_then(|offering| offering.get("instructionLanguage")).and_then(Value::as_str)),
                    "color":"#526e91", "meetings":meetings,
                    "source":{"provider":"lms","externalId":lms_id.to_string(),"fetchedAt":now}
                });
                // The LMS page is reached through the LMS record; `url` is the syllabus.
                let url = offering
                    .and_then(|offering| offering.get("syllabusUrl"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        offering_id
                            .map(|id| format!("https://syllabus.s.isct.ac.jp/courses/-/-/-/{id}"))
                    });
                let object = object_mut(&mut course, "course")?;
                if let Some(url) = url {
                    object.insert("url".into(), Value::String(url));
                }
                if let Some(offering_id) = offering_id {
                    object.insert("offeringId".into(), Value::String(offering_id.into()));
                }
                let course = write.put("manual_courses", course, None)?;
                id = string(&course, "id")?.to_string();
                created.insert(id.clone());
            }
        }
        // A 3Q–4Q course pulled in 3Q also joins 4Q; quarters inside the
        // reviewed scope are left to their own entry, which the person may
        // have unchecked.
        let mut member_terms = vec![term.to_string()];
        member_terms.extend(
            lms_terms_with_offering(lms, offering)
                .into_iter()
                .filter(|other| !terms.contains(other)),
        );
        for member_term in member_terms {
            if live_members.contains(&(id.clone(), member_term.clone())) {
                continue;
            }
            let membership_id = format!("lms-membership:{lms_id}:{member_term}");
            match memberships
                .iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(&membership_id))
            {
                Some(row) if is_live(row) => {}
                Some(row) => {
                    write.put("manual_memberships", (*row).clone(), revision(row))?;
                }
                None => {
                    write.put(
                        "manual_memberships",
                        json!({
                            "id":membership_id, "kind":"membership", "termId":member_term,
                            "courseId":id, "status":"lms-member",
                            "evidence":{"provider":"lms","externalId":lms_id.to_string(),"fetchedAt":now}
                        }),
                        None,
                    )?;
                }
            }
            live_members.insert((id.clone(), member_term));
        }
        linked.entry(lms_id).or_insert(id);
    }

    // The latest edited copy of a live course, else its stored row.
    let draft = |edited: &HashMap<String, (Value, Option<u64>)>, id: &str| {
        edited.get(id).cloned().or_else(|| {
            stored_courses
                .iter()
                .find(|course| {
                    is_live(course) && course.get("id").and_then(Value::as_str) == Some(id)
                })
                .map(|course| (stored_course(course), revision(course)))
        })
    };
    // Courses added from LMS follow its current name and take the syllabus
    // URL and credits once the syllabus gives them.
    for pair in &diff.common {
        let local = &pair["local"];
        if local.pointer("/source/provider").and_then(Value::as_str) != Some("lms") {
            continue;
        }
        let entry = &pair["lms"];
        let offering = entry.get("offering").filter(|value| value.is_object());
        let names = offering
            .and_then(|offering| offering.get("title"))
            .or_else(|| entry.pointer("/lms/title"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let title = entry.get("title").cloned().unwrap_or_else(|| json!(""));
        let mut changed = false;
        let renamed = local
            .pointer("/overrides/title")
            .and_then(Value::as_str)
            .is_some_and(|title| !title.is_empty());
        let id = string(local, "id")?;
        let Some((mut course, rev)) = draft(&edited, id) else {
            continue;
        };
        if !renamed
            && (course.pointer("/names/title").unwrap_or(&json!({})) != &names
                || course.get("title") != Some(&title))
        {
            let instructor = course
                .pointer("/names/instructor")
                .cloned()
                .unwrap_or_else(|| json!({}));
            course["names"] = json!({"title":names,"instructor":instructor});
            course["title"] = title;
            changed = true;
        }
        if let Some(url) = offering
            .and_then(|offering| offering.get("syllabusUrl"))
            .and_then(Value::as_str)
        {
            if !course
                .get("url")
                .and_then(Value::as_str)
                .is_some_and(|url| url.starts_with("https://syllabus.s.isct.ac.jp/"))
            {
                course["url"] = Value::String(url.into());
                changed = true;
            }
        }
        if let Some(credits) = offering.and_then(|offering| offering.get("credits")) {
            let total = credit_total(credits);
            if total > 0.0 && course.get("credits").and_then(Value::as_f64) == Some(0.0) {
                course["credits"] = json!(total);
                course["creditSplit"] = credits.clone();
                changed = true;
            }
        }
        if changed {
            edited.insert(id.to_string(), (course, rev));
        }
    }
    for pair in &diff.common {
        let id = string(&pair["local"], "id")?;
        let grid = pair
            .pointer("/lms/gridMeetings")
            .filter(|grid| !grid.is_null());
        let (Some(grid), true) = (
            grid,
            pair.get("timeDiffers").and_then(Value::as_bool) == Some(true)
                && retime.contains(&format!("local:{id}")),
        ) else {
            continue;
        };
        if let Some((mut course, rev)) = draft(&edited, id) {
            course["meetings"] =
                meetings_from_grid(grid, course.get("meetings").unwrap_or(&json!([])))?;
            edited.insert(id.to_string(), (course, rev));
        }
    }
    let mut edited = edited.into_iter().collect::<Vec<_>>();
    edited.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, (course, rev)) in edited {
        write.put("manual_courses", course, rev)?;
    }

    for entry in &diff.local_only {
        if entry
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| keep.contains(key))
        {
            continue;
        }
        let term = string(entry, "termId")?;
        remove_placements(write, current, string(&entry["course"], "id")?, Some(term))?;
    }

    let mut linked = linked.into_iter().collect::<Vec<_>>();
    linked.sort();
    for (lms_id, course_id) in linked {
        write.relink_lms_records(lms_id, &course_id)?;
    }
    let mut status = current
        .get("importStatus")
        .cloned()
        .unwrap_or_else(|| json!({}));
    object_mut(&mut status, "import status")?.insert(
        "timetableReviewedAt".into(),
        Value::String(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    );
    write.save_setting("importStatus", &status)
}

/// A course row as stored, without the overlay the snapshot adds.
fn stored_course(course: &Value) -> Value {
    let mut course = course.clone();
    if let Some(object) = course.as_object_mut() {
        object.remove("overrides");
    }
    course
}

fn credit_total(credits: &Value) -> f64 {
    ["lecture", "exercise", "lab"]
        .into_iter()
        .filter_map(|key| credits.get(key).and_then(Value::as_f64))
        .sum()
}

/// The course id of a catalog offering: `{year}-{offeringId}`.
fn offering_course_id(year: i32, offering_id: &str) -> Result<String, DataError> {
    if offering_id.len() == 9 && offering_id.bytes().all(|byte| byte.is_ascii_digit()) {
        crate::data::canonical_course_id(year, offering_id)
    } else {
        Ok(format!("{year}-{offering_id}"))
    }
}

fn lms_terms(lms: &Value) -> Vec<String> {
    let mut terms: HashSet<String> = lms
        .get("timetables")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|map| {
            map.keys()
                .filter(|term| validate_term(term).is_ok())
                .cloned()
        })
        .collect();
    if let Some(year) = lms.get("year").and_then(Value::as_i64) {
        for quarter in lms
            .get("quarters")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let term = format!("{year}-{quarter}");
            if validate_term(&term).is_ok() {
                terms.insert(term);
            }
        }
    }
    terms.into_iter().collect()
}

fn base_code(value: &str) -> String {
    let mut value = value.trim().to_ascii_uppercase();
    if value.ends_with(']') {
        if let Some(index) = value.rfind('[') {
            value.truncate(index);
        }
    }
    if let Some((head, tail)) = value.rsplit_once('-') {
        if tail.bytes().all(|byte| byte.is_ascii_digit()) {
            value = head.to_string();
        }
    }
    value.trim().to_string()
}

/// Builds the review model for an LMS timetable merge and reports syllabus
/// details that the shell must fetch before asking again.  The returned
/// `conflicts` value is an object keyed by entry id; the React adapter may turn
/// it into a `Map` for the existing dialog.
pub fn lms_merge_preview(
    workspace: &Value,
    school_records: &Value,
    scope: &str,
    selected_term: &str,
    offerings: &[Value],
) -> Result<Value, WorkspaceError> {
    validate_term(selected_term).map_err(WorkspaceError::from)?;
    if scope != "term" && scope != "all" {
        return Err(WorkspaceError::InvalidIntent(
            "LMS merge scope must be term or all".into(),
        ));
    }
    let offerings = offerings
        .iter()
        .filter_map(|offering| crate::course_query::normalize_offering(offering.clone()))
        .collect::<Vec<_>>();
    let offering_by_id = |id: &str| {
        offerings
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
    };
    let lms_courses = school_records
        .get("lmsCourses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let selected_year = selected_term
        .get(..4)
        .and_then(|year| year.parse::<i64>().ok());
    let in_scope = |row: &Value| {
        let terms = lms_terms_with_offering(
            row,
            row.get("offeringId")
                .and_then(Value::as_str)
                .and_then(offering_by_id),
        );
        scope == "all"
            || terms.iter().any(|term| term == selected_term)
            || (terms.is_empty() && row.get("year").and_then(Value::as_i64) == selected_year)
    };
    let detail_ids = lms_courses
        .iter()
        .filter(|row| in_scope(row))
        .filter_map(|row| row.get("offeringId").and_then(Value::as_str))
        .filter(|id| {
            offering_by_id(id)
                .and_then(|row| row.get("detailed"))
                .and_then(Value::as_bool)
                != Some(true)
        })
        .map(str::to_string)
        .collect::<HashSet<_>>();

    let mut terms = if scope == "term" {
        vec![selected_term.to_string()]
    } else {
        let mut values: HashSet<String> = lms_courses
            .iter()
            .flat_map(|row| {
                lms_terms_with_offering(
                    row,
                    row.get("offeringId")
                        .and_then(Value::as_str)
                        .and_then(offering_by_id),
                )
            })
            .collect();
        values.extend(crate::schedule::placement_terms(workspace).map(str::to_string));
        values.extend(
            workspace
                .get("terms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
        if let Some(term) = workspace.get("currentTerm").and_then(Value::as_str) {
            values.insert(term.to_string());
        }
        values.into_iter().collect()
    };
    terms.sort();
    let diff = lms_diff(
        workspace,
        school_records,
        &terms,
        &offerings,
        workspace_content_locale(workspace),
    )
    .map_err(WorkspaceError::from)?;
    let (common, local_only, lms_only) = (diff.common, diff.local_only, diff.lms_only);
    let mut conflicts = Map::new();
    let mut group = 0_u64;
    for remote in &lms_only {
        for local in &local_only {
            let same_code = !remote
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .is_empty()
                && base_code(
                    remote
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ) == base_code(
                    local
                        .pointer("/course/code")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            let reason = if same_code && remote.get("termId") != local.get("termId") {
                Some("quarter")
            } else if remote.get("termId") == local.get("termId")
                && !value_string_set(remote.get("slots").unwrap_or(&Value::Null)).is_disjoint(
                    &value_string_set(local.get("slots").unwrap_or(&Value::Null)),
                )
            {
                Some("slot")
            } else {
                None
            };
            if let Some(reason) = reason {
                let remote_key = string(remote, "key").map_err(WorkspaceError::from)?;
                let local_key = string(local, "key").map_err(WorkspaceError::from)?;
                let id = conflicts
                    .get(remote_key)
                    .or_else(|| conflicts.get(local_key))
                    .and_then(|value| value.get("group"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        let id = group;
                        group += 1;
                        id
                    });
                conflicts.insert(remote_key.into(), json!({"group":id,"reason":reason}));
                conflicts.insert(local_key.into(), json!({"group":id,"reason":reason}));
            }
        }
    }
    let has_differences = !local_only.is_empty()
        || !lms_only.is_empty()
        || common
            .iter()
            .any(|pair| pair.get("timeDiffers").and_then(Value::as_bool) == Some(true));
    let mut detail_ids = detail_ids.into_iter().collect::<Vec<_>>();
    detail_ids.sort();
    Ok(
        json!({"terms":terms,"detailIds":detail_ids,"common":common,"localOnly":local_only,"lmsOnly":lms_only,"conflicts":conflicts,"hasDifferences":has_differences}),
    )
}

/// The LMS review diff of `terms`: matched (`common`), local-only and
/// LMS-only entries.  Shared by the preview and the applied merge so both see
/// the same entries.
struct LmsDiff {
    common: Vec<Value>,
    local_only: Vec<Value>,
    lms_only: Vec<Value>,
}

fn lms_diff(
    workspace: &Value,
    school_records: &Value,
    terms: &[String],
    offerings: &[Value],
    locale: &str,
) -> Result<LmsDiff, DataError> {
    let lms_courses = school_records
        .get("lmsCourses")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let term_scope: HashSet<_> = terms.iter().cloned().collect();
    let offering_by_id = |id: &str| {
        offerings
            .iter()
            .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
    };
    let mut remote = Vec::<Value>::new();
    for row in lms_courses
        .iter()
        .filter(|row| row.get("hidden").and_then(Value::as_bool) != Some(true))
    {
        let offering = row
            .get("offeringId")
            .and_then(Value::as_str)
            .and_then(offering_by_id);
        for term in lms_terms_with_offering(row, offering)
            .into_iter()
            .filter(|term| term_scope.contains(term))
        {
            let grid = row.pointer(&format!("/timetables/{term}"));
            let slots = grid
                .map(grid_slots)
                .unwrap_or_else(|| offering.map(offering_slots).unwrap_or_default());
            let shortname = row
                .get("shortname")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let title = localized(
                offering
                    .and_then(|row| row.get("title"))
                    .or_else(|| row.get("title")),
                locale,
            );
            let title = crate::school_parser::lms::decode_entities(if title.is_empty() {
                shortname
            } else {
                &title
            });
            let code = offering
                .and_then(|row| row.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    shortname
                        .strip_suffix(']')
                        .and_then(|value| value.rsplit_once('[').map(|(head, _)| head.trim()))
                        .unwrap_or(shortname)
                        .to_string()
                });
            remote.push(json!({
                "key":format!("lms:{}:{term}", row.get("lmsId").and_then(Value::as_u64).unwrap_or_default()),
                "termId":term, "lms":row, "offering":offering, "title":title,
                "code":code, "slots":slots, "gridMeetings":grid
            }));
        }
    }
    let mut common = Vec::new();
    let mut local_only = Vec::new();
    let mut matched = HashSet::new();
    for term in terms {
        // The timetable of the quarter: a 3Q–4Q course is compared in each.
        for course in &crate::schedule::scheduled_courses(workspace, term) {
            let found = remote.iter().find(|entry| {
                entry.get("termId").and_then(Value::as_str) == Some(term)
                    && !matched
                        .contains(entry.get("key").and_then(Value::as_str).unwrap_or_default())
                    && ((course.get("offeringId").and_then(Value::as_str).is_some()
                        && course.get("offeringId") == entry.pointer("/lms/offeringId"))
                        || base_code(
                            course
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ) == base_code(
                            entry
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ))
            });
            if let Some(entry) = found {
                matched.insert(string(entry, "key")?.to_string());
                let time_differs = entry
                    .get("gridMeetings")
                    .is_some_and(|grid| !grid.is_null())
                    && course_slots(course)
                        != value_string_set(entry.get("slots").unwrap_or(&Value::Null));
                common.push(
                    json!({"termId":term,"local":course,"lms":entry,"timeDiffers":time_differs}),
                );
            } else {
                local_only.push(json!({"key":format!("local:{}",string(course,"id")?),"termId":term,"course":course,"slots":course_slots(course)}));
            }
        }
    }
    let lms_only = remote
        .into_iter()
        .filter(|entry| {
            !matched.contains(entry.get("key").and_then(Value::as_str).unwrap_or_default())
        })
        .collect::<Vec<_>>();
    Ok(LmsDiff {
        common,
        local_only,
        lms_only,
    })
}

fn lms_terms_with_offering(lms: &Value, offering: Option<&Value>) -> Vec<String> {
    let mut terms: HashSet<String> = lms_terms(lms).into_iter().collect();
    let year = lms.get("year").and_then(Value::as_i64).or_else(|| {
        offering
            .and_then(|row| row.get("year"))
            .and_then(Value::as_i64)
    });
    if let Some(year) = year {
        let quarters = offering
            .and_then(|row| row.get("quarters"))
            .and_then(Value::as_array)
            .filter(|quarters| !quarters.is_empty())
            .or_else(|| lms.get("quarters").and_then(Value::as_array));
        for quarter in quarters.into_iter().flatten().filter_map(Value::as_str) {
            let term = format!("{year}-{quarter}");
            if validate_term(&term).is_ok() {
                terms.insert(term);
            }
        }
    }
    terms.into_iter().collect()
}

fn grid_slots(grid: &Value) -> Value {
    Value::Array(
        grid.as_array()
            .into_iter()
            .flatten()
            .flat_map(|meeting| {
                let day = meeting
                    .get("day")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                let bounds = meeting.get("periods").and_then(Value::as_array);
                let start = bounds
                    .and_then(|value| value.first())
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                let end = bounds
                    .and_then(|value| value.get(1))
                    .and_then(Value::as_u64)
                    .unwrap_or(start);
                (start..=end).map(move |period| Value::String(format!("{day}:{period}")))
            })
            .collect(),
    )
}

fn offering_slots(offering: &Value) -> Value {
    let schedule = offering
        .get("schedule")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Value::Array(
        schedule
            .iter()
            .filter(|row| row.get("kind").and_then(Value::as_str) == Some("weekly"))
            .flat_map(|row| {
                let day = row.get("day").and_then(Value::as_u64).unwrap_or_default();
                let bounds = row.get("periods").and_then(Value::as_array);
                let start = bounds
                    .and_then(|value| value.first())
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                let end = bounds
                    .and_then(|value| value.get(1))
                    .and_then(Value::as_u64)
                    .unwrap_or(start);
                (start..=end).map(move |period| Value::String(format!("{day}:{period}")))
            })
            .collect(),
    )
}

fn course_slots(course: &Value) -> HashSet<String> {
    course
        .get("meetings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|meeting| {
            let days = meeting
                .get("days")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect::<Vec<_>>();
            meeting
                .get("periods")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .flat_map(move |period| {
                    days.clone()
                        .into_iter()
                        .map(move |day| format!("{day}:{period}"))
                })
        })
        .collect()
}

fn value_string_set(value: &Value) -> HashSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Meetings from the LMS timetable grid, with the syllabus room for the same
/// weekday when known.
fn meetings_from_grid(grid: &Value, syllabus: &Value) -> Result<Value, DataError> {
    let mut meetings = Vec::new();
    for row in grid.as_array().into_iter().flatten() {
        let day = row
            .get("day")
            .and_then(Value::as_u64)
            .ok_or_else(|| "LMS timetable slot is missing its day".to_string())?;
        let bounds = row.get("periods").and_then(Value::as_array);
        let start = bounds
            .and_then(|bounds| bounds.first())
            .and_then(Value::as_u64)
            .ok_or_else(|| "LMS timetable slot is missing its periods".to_string())?;
        let end = bounds
            .and_then(|bounds| bounds.get(1))
            .and_then(Value::as_u64)
            .unwrap_or(start);
        let period = |value: u64| {
            u8::try_from(value)
                .ok()
                .filter(|value| (1..=10).contains(value))
                .ok_or_else(|| DataError::from("LMS timetable periods must be within 1..10"))
        };
        let (from, to) = (period(start)?, period(end)?);
        if from > to {
            return Err("LMS timetable periods are out of order".into());
        }
        let (start_time, end_time) = crate::schedule::clock_for_periods([from, to])
            .map_err(|error| DataError::from(error.to_string()))?;
        let twin = syllabus.as_array().into_iter().flatten().find(|meeting| {
            meeting
                .get("days")
                .and_then(Value::as_array)
                .is_some_and(|days| days.iter().any(|value| value.as_u64() == Some(day)))
        });
        let mut meeting = json!({
            "days":[day], "periods":(from..=to).collect::<Vec<_>>(),
            "startTime":start_time, "endTime":end_time,
            "room":twin.and_then(|twin| twin.get("room")).cloned().unwrap_or_else(|| json!(""))
        });
        if let Some(names) = twin
            .and_then(|twin| twin.get("roomNames"))
            .filter(|names| !names.is_null())
        {
            meeting["roomNames"] = names.clone();
        }
        meetings.push(meeting);
    }
    Ok(Value::Array(meetings))
}

/// Reverses the last `AddCatalogCourses` of this session: removes only the
/// plan items it placed and brings back the ones it replaced.
fn undo_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    mut offering_ids: Vec<String>,
    receipt: Option<CatalogAddReceipt>,
) -> Result<(), DataError> {
    validate_term(term_id)?;
    offering_ids.sort();
    offering_ids.dedup();
    let receipt = receipt
        .filter(|receipt| receipt.term_id == term_id && receipt.offering_ids == offering_ids)
        .ok_or_else(|| "this add can no longer be undone".to_string())?;
    for plan in rows(current, "planItems")? {
        let Some(id) = plan.get("id").and_then(Value::as_str) else {
            continue;
        };
        if is_live(plan) && receipt.placed.iter().any(|placed| placed == id) {
            write.delete("plan_items", id, revision(plan))?;
        } else if !is_live(plan) && receipt.replaced.iter().any(|replaced| replaced == id) {
            write.put("plan_items", plan.clone(), revision(plan))?;
        }
    }
    Ok(())
}

/// Converts a catalog offering to the shell's syllabus-course shape.  Keeping
/// this conversion beside `AddCatalogCourses` prevents sync and add flows from
/// disagreeing about localized names, rooms, credits, or period clocks.
pub fn catalog_course(
    offering: &Value,
    locale: &str,
    content: Option<&Value>,
) -> Result<Value, WorkspaceError> {
    let offering_id = string(offering, "id").map_err(WorkspaceError::from)?;
    let year = offering
        .get("year")
        .and_then(Value::as_i64)
        .ok_or_else(|| WorkspaceError::InvalidIntent("catalog offering is missing year".into()))?;
    let quarters = offering
        .get("quarters")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            WorkspaceError::InvalidIntent("catalog offering is missing quarters".into())
        })?;
    let quarter = quarters.first().and_then(Value::as_str).unwrap_or("1Q");
    let credits = offering.get("credits").and_then(Value::as_object);
    let total_credits = credits.map(|credits| {
        ["lecture", "exercise", "lab"]
            .into_iter()
            .filter_map(|key| credits.get(key).and_then(Value::as_f64))
            .sum::<f64>()
    });
    let instructors = offering
        .get("instructors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|name| localized(Some(name), locale))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    let mut result = json!({
        "externalId":offering_id,
        "year":year,
        "quarter":quarter,
        "quarters":quarters,
        "code":string(offering, "code").map_err(WorkspaceError::from)?,
        "title":localized(offering.get("title"), locale),
        "instructor":instructors,
        "language":offering.get("instructionLanguage"),
        "meetings":catalog_meetings(offering, locale).map_err(WorkspaceError::from)?,
        "url":string(offering, "syllabusUrl").map_err(WorkspaceError::from)?,
        "updated":offering.get("sourceUpdated")
    });
    if let Some(credits) = total_credits {
        result["credits"] = json!(credits);
    }
    if let Some(content) = content {
        result["content"] = content.clone();
    }
    Ok(result)
}

fn catalog_meetings(offering: &Value, locale: &str) -> Result<Value, DataError> {
    let mut meetings = Vec::new();
    for entry in offering
        .get("schedule")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if entry.get("kind").and_then(Value::as_str) != Some("weekly") {
            continue;
        }
        let day = entry
            .get("day")
            .and_then(Value::as_u64)
            .ok_or_else(|| "weekly schedule is missing day".to_string())?;
        let bounds = entry
            .get("periods")
            .and_then(Value::as_array)
            .ok_or_else(|| "weekly schedule is missing periods".to_string())?;
        let start = bounds
            .first()
            .and_then(Value::as_u64)
            .ok_or_else(|| "weekly schedule start is invalid".to_string())?;
        let end = bounds
            .get(1)
            .and_then(Value::as_u64)
            .ok_or_else(|| "weekly schedule end is invalid".to_string())?;
        if start == 0 || end > 10 || start > end {
            return Err("weekly periods must be within 1..10".into());
        }
        let periods: Vec<_> = (start..=end).map(Value::from).collect();
        let bounds = [
            u8::try_from(start).map_err(|_| DataError::from("period is outside 1..10"))?,
            u8::try_from(end).map_err(|_| DataError::from("period is outside 1..10"))?,
        ];
        let (start_time, end_time) = crate::schedule::clock_for_periods(bounds)
            .map_err(|error| DataError::from(error.to_string()))?;
        let room_names = entry.get("room").cloned();
        let room = localized(room_names.as_ref(), locale);
        meetings.push(json!({"days":[day],"periods":periods,"startTime":start_time,"endTime":end_time,"room":room,"roomNames":room_names}));
    }
    Ok(Value::Array(meetings))
}

fn localized(value: Option<&Value>, locale: &str) -> String {
    value
        .and_then(|value| {
            value
                .get(locale)
                .or_else(|| value.get("en"))
                .or_else(|| value.get("ja"))
        })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn workspace_content_locale(workspace: &Value) -> &'static str {
    match workspace
        .pointer("/settings/contentLanguage")
        .and_then(Value::as_str)
    {
        Some("ja") => "ja",
        Some("en") => "en",
        _ if workspace
            .pointer("/settings/locale")
            .and_then(Value::as_str)
            == Some("ja") =>
        {
            "ja"
        }
        _ => "en",
    }
}

fn normalize_language(value: Option<&str>) -> &'static str {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "ja" | "japanese" | "日本語" => "ja",
        "en" | "english" | "英語" => "en",
        "ja-en" => "ja-en",
        _ => "other",
    }
}

fn save_simple_record<F>(
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

fn save_calendar_exception(
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

fn replace_records(
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

fn record_scheduled_task_run(
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

fn set_subscription(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    subscription_kind: String,
    enabled: Option<bool>,
    url: Option<String>,
    reset_remote: bool,
) -> Result<(), DataError> {
    if !crate::reference_refresh::REFERENCE_KINDS.contains(&subscription_kind.as_str()) {
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

fn delete_visible(
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

fn save_intensive(
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

fn set_assignment_mark(
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

fn record_lesson(
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

fn rows<'a>(
    workspace: &'a Value,
    field: &str,
) -> Result<impl Iterator<Item = &'a Value>, DataError> {
    workspace
        .get(field)
        .and_then(Value::as_array)
        .map(|rows| rows.iter())
        .ok_or_else(|| format!("workspace {field} must be an array").into())
}

fn visible_row<'a>(workspace: &'a Value, field: &str, id: &str) -> Option<&'a Value> {
    workspace.get(field)?.as_array()?.iter().find(|row| {
        row.get("id").and_then(Value::as_str) == Some(id)
            && row.get("deletedAt").is_none_or(Value::is_null)
    })
}

fn revision(value: &Value) -> Option<u64> {
    value.get("rev").and_then(Value::as_u64)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, DataError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("record is missing {key}").into())
}
fn object_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}
fn require_object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>, DataError> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object").into())
}
fn object_mut<'a>(
    value: &'a mut Value,
    label: &str,
) -> Result<&'a mut Map<String, Value>, DataError> {
    value
        .as_object_mut()
        .ok_or_else(|| format!("{label} must be an object").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{EventError, EventSink};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MutationEvents {
        count: AtomicUsize,
        fail: bool,
    }

    impl EventSink for MutationEvents {
        fn emit(&self, event: crate::events::CoreEvent) -> Result<(), EventError> {
            assert!(matches!(
                event,
                crate::events::CoreEvent::WorkspaceChanged {
                    source: crate::events::WorkspaceChangeSource::UserIntent,
                    revision: Some(_),
                }
            ));
            self.count.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                Err(EventError::Delivery("synthetic delivery failure".into()))
            } else {
                Ok(())
            }
        }
    }

    fn service(name: &str) -> WorkspaceService {
        let path =
            std::env::temp_dir().join(format!("isct-workspace-{name}-{}.db", Uuid::new_v4()));
        WorkspaceService::new(DataStore::initialize(path).unwrap().0)
    }

    #[test]
    fn committed_mutations_emit_once_and_delivery_does_not_change_the_result() {
        let path =
            std::env::temp_dir().join(format!("isct-workspace-events-{}.db", Uuid::new_v4()));
        let store = DataStore::initialize(path).unwrap().0;
        let events = Arc::new(MutationEvents {
            count: AtomicUsize::new(0),
            fail: false,
        });
        let service = WorkspaceService::with_events(store, events.clone());
        let initial = service.snapshot().unwrap();
        let committed = service
            .mutate(WorkspaceMutation {
                expected_revision: initial.revision,
                intent: WorkspaceIntent::SetAdmissionYear {
                    admission_year: Some(2026),
                },
            })
            .unwrap();
        assert_eq!(events.count.load(Ordering::Relaxed), 1);

        let failed = service.mutate(WorkspaceMutation {
            expected_revision: "stale".into(),
            intent: WorkspaceIntent::SetAdmissionYear {
                admission_year: Some(2025),
            },
        });
        assert!(matches!(failed, Err(WorkspaceError::Conflict(_))));
        assert_eq!(events.count.load(Ordering::Relaxed), 1);

        let failing_events = Arc::new(MutationEvents {
            count: AtomicUsize::new(0),
            fail: true,
        });
        let service = WorkspaceService::with_events(service.store.clone(), failing_events.clone());
        assert!(service
            .mutate(WorkspaceMutation {
                expected_revision: committed.revision,
                intent: WorkspaceIntent::SetAdmissionYear {
                    admission_year: Some(2027),
                },
            })
            .is_ok());
        assert_eq!(failing_events.count.load(Ordering::Relaxed), 1);
    }

    fn settings_with(snapshot: &WorkspaceSnapshot, edit: impl FnOnce(&mut Value)) -> Value {
        let mut settings = snapshot.workspace["settings"].clone();
        edit(&mut settings);
        settings
    }

    #[test]
    fn stale_revision_cannot_overwrite_a_newer_settings_change() {
        let service = service("conflict");
        let original = service.snapshot().unwrap();
        let changed = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::ReplaceSettings {
                    settings: settings_with(&original, |s| s["appearance"] = json!("dark")),
                },
            })
            .unwrap();
        assert_ne!(changed.revision, original.revision);
        assert_eq!(changed.workspace["settings"]["appearance"], "dark");

        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::ReplaceSettings {
                    settings: settings_with(&original, |s| s["density"] = json!("compact")),
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::Conflict(_)));
        assert_eq!(
            service.snapshot().unwrap().workspace["settings"]["density"],
            changed.workspace["settings"]["density"]
        );
    }

    #[test]
    fn settings_replacement_clears_keys_and_core_validates_every_value() {
        let service = service("settings");
        let original = service.snapshot().unwrap();
        let chosen = mutate(
            &service,
            WorkspaceIntent::ReplaceSettings {
                settings: settings_with(&original, |s| {
                    s["departmentId"] = json!("cs");
                    s["textSize"] = json!("large");
                }),
            },
        );
        assert_eq!(chosen.workspace["settings"]["departmentId"], "cs");
        service.store.record_reference_poll().unwrap();
        let polled = service.snapshot().unwrap();
        let checked = polled.workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"].clone();
        assert!(checked.is_string());

        // A form built from the earlier snapshot clears the department and
        // cannot roll the Core-owned poll clock back.
        let cleared = mutate(
            &service,
            WorkspaceIntent::ReplaceSettings {
                settings: settings_with(&chosen, |s| {
                    s.as_object_mut().unwrap().remove("departmentId");
                }),
            },
        );
        assert!(cleared.workspace["settings"].get("departmentId").is_none());
        assert_eq!(cleared.workspace["settings"]["textSize"], "large");
        assert_eq!(
            cleared.workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"],
            checked
        );

        let before = service.snapshot().unwrap();
        for (pointer, value) in [
            ("/locale", json!("xx")),
            ("/appearance", json!("blue")),
            ("/textSize", json!("huge")),
            ("/contentLanguage", json!("fr")),
            ("/density", json!(3)),
            ("/cloud/provider", json!("dropbox")),
        ] {
            let error = service
                .mutate(WorkspaceMutation {
                    expected_revision: before.revision.clone(),
                    intent: WorkspaceIntent::ReplaceSettings {
                        settings: settings_with(&before, |s| {
                            let (parent, key) = pointer.rsplit_once('/').unwrap();
                            s.pointer_mut(parent).unwrap()[key] = value;
                        }),
                    },
                })
                .unwrap_err();
            assert!(
                matches!(error, WorkspaceError::InvalidIntent(_)),
                "{pointer}"
            );
        }
        for intent in [
            WorkspaceIntent::SetAdmissionYear {
                admission_year: Some(1999),
            },
            WorkspaceIntent::EditAcademicCalendar {
                academic_profile: json!({"name":"Mine","admissionYear":1990,"gradePath":[],"years":[]}),
                exceptions: vec![],
            },
        ] {
            let error = service
                .mutate(WorkspaceMutation {
                    expected_revision: before.revision.clone(),
                    intent,
                })
                .unwrap_err();
            assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        }
        assert!(matches!(
            service.store.put_workspace_setting(
                "devicePreferences",
                json!({"locale":"xx"}),
                "test"
            ),
            Err(DataError::InvalidInput(_))
        ));
        assert_eq!(service.snapshot().unwrap().revision, before.revision);
        let year = mutate(
            &service,
            WorkspaceIntent::SetAdmissionYear {
                admission_year: Some(2024),
            },
        );
        assert_eq!(year.workspace["academicProfile"]["admissionYear"], 2024);
    }

    #[test]
    fn study_profile_is_backward_compatible_and_preserves_calendar_fields() {
        let service = service("study-profile");
        let legacy = json!({
            "name":"My program",
            "admissionYear":2024,
            "gradePath":[{"grade":"B2","academicYear":2025}],
            "years":[{"academicYear":2026,"quarterStarts":{
                "1Q":"2026-04-01","2Q":"2026-06-01","3Q":"2026-10-01","4Q":"2026-12-01"
            }}]
        });
        service
            .store
            .put_workspace_setting("academicProfile", legacy.clone(), "test")
            .unwrap();
        let old = service.snapshot().unwrap();
        assert!(old.workspace["academicProfile"]
            .get("studyEnrollments")
            .is_none());

        let saved = run(
            &service,
            WorkspaceIntent::SetStudyProfile {
                admission_year: Some(2024),
                study_enrollments: vec![
                    StudyEnrollment {
                        stage: crate::study_profile::StudyStage::Undergraduate,
                        entry_kind: crate::study_profile::StudyEntryKind::Regular,
                        entry_grade: Some(1),
                        admitted_term: "2024-1Q".into(),
                    },
                    StudyEnrollment {
                        stage: crate::study_profile::StudyStage::Master,
                        entry_kind: crate::study_profile::StudyEntryKind::Regular,
                        entry_grade: Some(1),
                        admitted_term: "2028-1Q".into(),
                    },
                ],
            },
        )
        .unwrap();
        assert_eq!(saved["academicProfile"]["name"], legacy["name"]);
        assert_eq!(saved["academicProfile"]["gradePath"], legacy["gradePath"]);
        assert_eq!(saved["academicProfile"]["years"], legacy["years"]);
        assert_eq!(
            saved["academicProfile"]["studyEnrollments"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let calendar = run(
            &service,
            WorkspaceIntent::EditAcademicCalendar {
                academic_profile: json!({
                    "name":"Renamed program", "admissionYear":2024,
                    "gradePath":legacy["gradePath"], "years":legacy["years"]
                }),
                exceptions: vec![],
            },
        )
        .unwrap();
        assert_eq!(
            calendar["academicProfile"]["studyEnrollments"],
            saved["academicProfile"]["studyEnrollments"],
            "an older calendar editor must preserve the stage history"
        );
    }

    #[test]
    fn invalid_study_history_is_rejected_atomically() {
        let service = service("invalid-study-profile");
        let before = service.snapshot().unwrap();
        for study_enrollments in [
            vec![StudyEnrollment {
                stage: crate::study_profile::StudyStage::Undergraduate,
                entry_kind: crate::study_profile::StudyEntryKind::Regular,
                entry_grade: Some(1),
                admitted_term: "2026-spring".into(),
            }],
            vec![
                StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Undergraduate,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-1Q".into(),
                },
                StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Master,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-1Q".into(),
                },
            ],
        ] {
            let error = service
                .mutate(WorkspaceMutation {
                    expected_revision: before.revision.clone(),
                    intent: WorkspaceIntent::SetStudyProfile {
                        admission_year: Some(2026),
                        study_enrollments,
                    },
                })
                .unwrap_err();
            assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
            let after = service.snapshot().unwrap();
            assert_eq!(after.revision, before.revision);
            assert_eq!(
                after.workspace["academicProfile"],
                before.workspace["academicProfile"]
            );
        }
    }

    #[test]
    fn invalid_record_rolls_back_the_whole_intent() {
        let service = service("rollback");
        let original = service.snapshot().unwrap();
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::SaveEvent {
                    draft: json!({"title":"Missing required event fields"}),
                    term_id: "2026-3Q".into(),
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        let after = service.snapshot().unwrap();
        assert_eq!(after.revision, original.revision);
        assert!(after.workspace["events"].as_array().unwrap().is_empty());
    }

    #[test]
    fn save_plan_canonicalizes_civil_time_and_rejects_invalid_drafts_atomically() {
        let service = service("plan-validation");
        let original = service.snapshot().unwrap();
        let saved = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::SavePlan {
                    draft: json!({
                        "title":" Study ", "start":"2026-10-14T09:00", "duration":60,
                        "rrule":"freq=weekly;count=2", "exdates":["2026-10-21T18:00:00+09:00"]
                    }),
                    existing_id: None,
                },
            })
            .unwrap();
        assert_eq!(saved.workspace["plans"][0]["title"], "Study");
        assert_eq!(
            saved.workspace["plans"][0]["start"],
            "2026-10-14T09:00:00+09:00"
        );
        assert_eq!(
            saved.workspace["plans"][0]["exdates"],
            json!(["2026-10-21"])
        );

        let before_invalid = service.snapshot().unwrap();
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: before_invalid.revision.clone(),
                intent: WorkspaceIntent::SavePlan {
                    draft: json!({
                        "title":"Broken", "start":"not-a-date", "duration":0, "exdates":[]
                    }),
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        let after_invalid = service.snapshot().unwrap();
        assert_eq!(after_invalid.revision, before_invalid.revision);
        assert_eq!(
            after_invalid.workspace["plans"].as_array().unwrap().len(),
            1
        );

        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: after_invalid.revision.clone(),
                intent: WorkspaceIntent::SavePlan {
                    draft: json!({
                        "title":"Broken recurrence", "start":"2026-10-14T09:00",
                        "duration":60, "rrule":"FREQ=DAILY;INTERVAL=garbage", "exdates":[]
                    }),
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        let after_rule = service.snapshot().unwrap();
        assert_eq!(after_rule.revision, after_invalid.revision);
        assert_eq!(after_rule.workspace["plans"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn schedule_override_periods_must_name_a_class_slot() {
        let service = service("override-periods");
        let original = service.snapshot().unwrap();
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::SaveScheduleOverride {
                    draft: json!({"targetKey":"course-weekly-0-2026-10-14","targetKind":"course","periods":[5,3],"hidden":false}),
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        assert_eq!(service.snapshot().unwrap().revision, original.revision);
    }

    #[test]
    fn course_batch_validation_happens_before_any_row_is_changed() {
        let service = service("course-batch");
        let original = service.snapshot().unwrap();
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::EditCourseBatch {
                    course_ids: vec!["missing".into()],
                    changes: CourseBatchChanges {
                        credits: Some(21.0),
                        ..CourseBatchChanges::default()
                    },
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        assert_eq!(service.snapshot().unwrap().revision, original.revision);
    }

    fn mutate(service: &WorkspaceService, intent: WorkspaceIntent) -> WorkspaceSnapshot {
        if let WorkspaceIntent::AddCatalogCourses { offerings, .. } = &intent {
            service.store.catalog_upsert_offerings(offerings).unwrap();
        }
        let expected_revision = service.snapshot().unwrap().revision;
        service
            .mutate(WorkspaceMutation {
                expected_revision,
                intent,
            })
            .unwrap()
    }

    fn course<'a>(snapshot: &'a WorkspaceSnapshot, id: &str) -> &'a Value {
        snapshot.workspace["courses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|course| course["id"] == id)
            .unwrap()
    }

    #[test]
    fn course_edits_merge_into_one_overlay_and_batch_credits_reach_every_course() {
        let service = service("course-overlay");
        let added = mutate(
            &service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: "2026-3Q".into(),
                offerings: vec![json!({
                    "id":"202600002", "year":2026, "quarters":["3Q"],
                    "code":"CSC.T364", "title":{"en":"Systems"},
                    "instructors":[{"en":"Teacher One"}],
                    "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600002",
                    "credits":{"lecture":2,"exercise":0,"lab":0},
                    "instructionLanguage":"ja", "format":{"kinds":["lecture"]}, "media":{},
                    "academicUnit":{}, "intensive":false, "fetchedAt":{},
                    "schedule":[{"kind":"weekly","day":1,"periods":[1,2]}]
                })],
            },
        );
        let catalog_id = added.workspace["courses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|course| course["offeringId"] == "202600002")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let manual = mutate(
            &service,
            WorkspaceIntent::SaveCourse {
                draft: json!({"code":"X.1","title":"Own","instructor":"Me","credits":1,"meetings":[]}),
                term_id: "2026-3Q".into(),
                existing_id: None,
            },
        );
        let manual_id = manual.workspace["courses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|course| course["title"] == "Own")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();

        mutate(
            &service,
            WorkspaceIntent::SaveCourse {
                draft: json!({"title":"Renamed","instructor":"Teacher A","color":"#123456"}),
                term_id: "2026-3Q".into(),
                existing_id: Some(catalog_id.clone()),
            },
        );
        let marked = mutate(
            &service,
            WorkspaceIntent::RecordLesson {
                course_id: catalog_id.clone(),
                occurrence: "2026-10-05T08:50:00+09:00".into(),
                section_id: 3,
            },
        );
        let overrides = &course(&marked, &catalog_id)["overrides"];
        assert_eq!(overrides["title"], "Renamed");
        assert_eq!(overrides["color"], "#123456");
        assert_eq!(overrides["lessonLog"][0]["sectionId"], 3);

        let edited = mutate(
            &service,
            WorkspaceIntent::EditCourseBatch {
                course_ids: vec![catalog_id.clone(), manual_id.clone()],
                changes: CourseBatchChanges {
                    credits: Some(4.0),
                    ..CourseBatchChanges::default()
                },
            },
        );
        let catalog = course(&edited, &catalog_id);
        assert_eq!(catalog["credits"], 4.0);
        assert_eq!(catalog["overrides"]["title"], "Renamed");
        assert_eq!(catalog["overrides"]["instructor"], "Teacher A");
        assert_eq!(catalog["overrides"]["lessonLog"][0]["sectionId"], 3);
        assert_eq!(course(&edited, &manual_id)["credits"], 4.0);
        assert_eq!(course(&edited, &manual_id)["instructor"], "Me");

        let renamed = mutate(
            &service,
            WorkspaceIntent::EditCourseBatch {
                course_ids: vec![catalog_id.clone(), manual_id.clone()],
                changes: CourseBatchChanges {
                    instructor: Some(" Teacher B ".into()),
                    language: Some("en".into()),
                    ..CourseBatchChanges::default()
                },
            },
        );
        let catalog = course(&renamed, &catalog_id);
        assert_eq!(catalog["overrides"]["instructor"], "Teacher B");
        assert_eq!(catalog["overrides"]["color"], "#123456");
        assert_eq!(catalog["instructionLanguage"], "en");
        assert_eq!(catalog["credits"], 4.0);
        assert_eq!(course(&renamed, &manual_id)["instructor"], "Teacher B");
        assert_eq!(course(&renamed, &manual_id)["instructionLanguage"], "en");
    }

    #[test]
    fn catalog_course_preserves_localized_fields_credits_rooms_and_long_period_ranges() {
        let service = service("catalog-course");
        service
            .store
            .catalog_upsert_offerings(&[json!({
                "id":"202600001", "year":2026, "quarters":["3Q"],
                "code":"CSC.T363", "title":{"ja":"演習","en":"Exercise"},
                "instructors":[{"ja":"教員一","en":"Teacher One"},{"ja":"教員二","en":"Teacher Two"}],
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600001",
                "credits":{"lecture":0,"exercise":1.5,"lab":0.5},
                "instructionLanguage":"ja", "format":{"kinds":["exercise"]}, "media":{},
                "academicUnit":{}, "intensive":false, "fetchedAt":{},
                "content":{"summary":{"ja":"内容"}},
                "schedule":[{"kind":"weekly","day":2,"periods":[5,8],"room":{"ja":"西講義棟","en":"West Hall"}}]
            })])
            .unwrap();
        let original = service.snapshot().unwrap();
        let next = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::AddCatalogCourses {
                    term_id: "2026-3Q".into(),
                    offerings: vec![json!({
                        "id":"202600001", "year":2026, "quarters":["3Q"],
                        "code":"CSC.T363", "title":{"ja":"演習","en":"Exercise"},
                        "instructors":[{"ja":"教員一","en":"Teacher One"},{"ja":"教員二","en":"Teacher Two"}],
                        "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600001",
                        "credits":{"lecture":0,"exercise":1.5,"lab":0.5},
                        "instructionLanguage":"ja",
                        "content":{"summary":{"ja":"内容"}},
                        "schedule":[{"kind":"weekly","day":2,"periods":[5,8],"room":{"ja":"西講義棟","en":"West Hall"}}]
                    })],
                },
            })
            .unwrap();
        let course = next.workspace["courses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|course| course["offeringId"] == "202600001")
            .unwrap();
        assert_eq!(course["title"], "Exercise");
        assert_eq!(course["instructor"], "Teacher One / Teacher Two");
        assert_eq!(course["credits"], 2.0);
        assert_eq!(course["creditSplit"]["exercise"], 1.5);
        assert_eq!(course["meetings"][0]["startTime"], "13:30");
        assert_eq!(course["meetings"][0]["endTime"], "17:05");
        assert_eq!(course["meetings"][0]["room"], "West Hall");
        assert_eq!(course["meetings"][0]["roomNames"]["ja"], "西講義棟");

        let converted = catalog_course(
            &json!({
                "id":"202600001", "year":2026, "quarters":["3Q"],
                "code":"CSC.T363", "title":{"ja":"演習","en":"Exercise"},
                "instructors":[{"ja":"教員一","en":"Teacher One"},{"ja":"教員二","en":"Teacher Two"}],
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202600001",
                "credits":{"lecture":0,"exercise":1.5,"lab":0.5},
                "instructionLanguage":"ja", "sourceUpdated":"2026-10-01",
                "schedule":[{"kind":"weekly","day":2,"periods":[5,8],"room":{"ja":"西講義棟","en":"West Hall"}}]
            }),
            "ja",
            Some(&json!({"summary":{"ja":"内容"}})),
        )
        .unwrap();
        assert_eq!(converted["title"], "演習");
        assert_eq!(converted["instructor"], "教員一 / 教員二");
        assert_eq!(converted["credits"], 2.0);
        assert_eq!(converted["meetings"][0]["endTime"], "17:05");
        assert_eq!(converted["content"]["summary"]["ja"], "内容");
    }

    #[test]
    fn subscription_toggle_and_reset_are_one_revision_checked_intent() {
        let service = service("subscription");
        let original = service.snapshot().unwrap();
        let created = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::SetSubscription {
                    subscription_kind: "calendar".into(),
                    enabled: Some(false),
                    url: Some("https://example.test/index.json".into()),
                    reset_remote: true,
                },
            })
            .unwrap();
        let row = &created.workspace["subscriptions"][0];
        assert_eq!(row["id"], "subscription:public:calendar");
        assert_eq!(row["enabled"], false);
        assert_eq!(row["url"], "https://example.test/index.json");
        assert!(row.get("remoteHash").is_none());

        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: created.revision,
                intent: WorkspaceIntent::SetSubscription {
                    subscription_kind: "calendar".into(),
                    enabled: None,
                    url: Some("http://insecure.test/index.json".into()),
                    reset_remote: false,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
    }

    #[test]
    fn scheduled_task_run_is_committed_with_the_workspace_revision() {
        let service = service("scheduled-task-run");
        let original = service.snapshot().unwrap();
        let task_id = original.workspace["scheduledTasks"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let next = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::RecordScheduledTaskRun {
                    task_id: task_id.clone(),
                    occurrence_at: "2026-09-29T12:00:00.000Z".into(),
                    run_at: "2026-09-29T12:00:01.000Z".into(),
                    result: "Export completed".into(),
                },
            })
            .unwrap();
        // Run bookkeeping is Core-owned: it changes the content, not the
        // baseline user edits are checked against.
        assert_ne!(next.content_revision, original.content_revision);
        assert_eq!(next.revision, original.revision);
        let task = next.workspace["scheduledTasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == task_id)
            .unwrap();
        assert_eq!(task["lastOccurrenceAt"], "2026-09-29T12:00:00.000Z");
        assert_eq!(task["lastRunAt"], "2026-09-29T12:00:01.000Z");
        assert_eq!(task["lastResult"], "Export completed");
        assert_eq!(
            service.snapshot().unwrap().content_revision,
            next.content_revision
        );
    }

    #[test]
    fn background_ingest_and_bookkeeping_do_not_conflict_with_user_edits() {
        let service = service("ingest-revision");
        let before = service.snapshot().unwrap();
        // A school ingest, a reference poll and a scheduler run land between
        // the person's snapshot and their edit.
        service
            .store
            .catalog_upsert_offerings(&[json!({
                "id":"202600009", "year":2026, "quarters":["3Q"], "code":"X.9",
                "title":{"en":"Ingested"}, "instructors":[], "instructionLanguage":"en",
                "format":{"kinds":["lecture"]}, "media":{},
                "credits":{"lecture":1.0,"exercise":0.0,"lab":0.0}, "schedule":[],
                "academicUnit":{"en":"Unit"}, "intensive":false,
                "syllabusUrl":"https://example.invalid/202600009", "fetchedAt":{}
            })])
            .unwrap();
        service.store.record_reference_poll().unwrap();
        let mut task = service.store.user_rows("scheduled_tasks").unwrap()[0].clone();
        let rev = task["rev"].as_u64();
        task["lastRunAt"] = json!("2026-10-14T00:00:00Z");
        task["lastResult"] = json!("Done");
        service
            .store
            .put_user_record_checked("scheduled_tasks", task, "scheduler", rev)
            .unwrap();
        let ingested = service.snapshot().unwrap();
        assert_ne!(ingested.content_revision, before.content_revision);
        assert_eq!(ingested.revision, before.revision);

        let edited = service
            .mutate(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::SelectTerm {
                    term_id: "2026-4Q".into(),
                },
            })
            .unwrap();
        assert_ne!(edited.revision, before.revision);
        // A toggle built from the older snapshot keeps the newer run record.
        let stale_task = before.workspace["scheduledTasks"][0].clone();
        let toggled = mutate(
            &service,
            WorkspaceIntent::SaveScheduledTask {
                existing_id: stale_task["id"].as_str().map(str::to_owned),
                draft: json!({
                    "name": stale_task["name"], "action": stale_task["action"],
                    "schedule": stale_task["schedule"],
                    "enabled": !stale_task["enabled"].as_bool().unwrap()
                }),
            },
        );
        let saved = toggled.workspace["scheduledTasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == stale_task["id"])
            .unwrap();
        assert_eq!(saved["enabled"], !stale_task["enabled"].as_bool().unwrap());
        assert_eq!(saved["lastResult"], "Done");
        // A user edit still conflicts with another user edit.
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: edited.revision,
                intent: WorkspaceIntent::SelectTerm {
                    term_id: "2026-3Q".into(),
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::Conflict(_)));
    }

    #[test]
    fn calendar_exception_metadata_and_unique_date_are_core_owned() {
        let service = service("calendar-exception");
        let original = service.snapshot().unwrap();
        let created = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::SaveCalendarException {
                    draft: json!({"date":"2026-10-13","type":"replacement","replacementDay":2,"label":" Tuesday classes "}),
                    existing_id: None,
                },
            })
            .unwrap();
        let row = &created.workspace["exceptions"][0];
        assert!(row["id"].as_str().is_some_and(|id| !id.is_empty()));
        assert_eq!(row["rev"], 1);
        assert_eq!(row["label"], "Tuesday classes");

        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: created.revision.clone(),
                intent: WorkspaceIntent::SaveCalendarException {
                    draft: json!({"date":"2026-10-13","type":"holiday","label":"Duplicate"}),
                    existing_id: None,
                },
            })
            .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        assert_eq!(service.snapshot().unwrap().revision, created.revision);
    }

    #[test]
    fn lms_preview_discovers_details_terms_and_slot_conflicts() {
        let mut workspace = service("lms-preview").snapshot().unwrap().workspace;
        workspace["currentTerm"] = json!("2026-3Q");
        workspace["terms"] = json!(["2026-3Q"]);
        workspace["courses"] = json!([{
            "id":"local-1","termId":"2026-3Q","code":"CSC.T363","title":"Local",
            "meetings":[{"days":[2],"periods":[5,6]}]
        }]);
        workspace["planItems"] = json!([{"courseId":"local-1","termId":"2026-3Q"}]);
        let school = json!({"lmsCourses":[{
            "lmsId":42,"year":2026,"quarters":["3Q"],"shortname":"MAT.A100[2026]",
            "title":{"en":"Remote"},"offeringId":"offering-1",
            "timetables":{"2026-3Q":[{"day":2,"periods":[5,6]}]}
        }]});
        let preview = lms_merge_preview(&workspace, &school, "all", "2026-3Q", &[]).unwrap();
        assert_eq!(preview["terms"], json!(["2026-3Q"]));
        assert_eq!(preview["detailIds"], json!(["offering-1"]));
        assert_eq!(preview["lmsOnly"][0]["code"], "MAT.A100");
        assert_eq!(preview["conflicts"]["local:local-1"]["reason"], "slot");
        assert_eq!(preview["hasDifferences"], true);

        let cached = lms_merge_preview(
            &workspace,
            &school,
            "all",
            "2026-3Q",
            &[json!({
                "id":"offering-1", "year":2026, "quarters":["3Q"],
                "code":"MAT.A100", "title":{"en":"Remote"}, "instructors":[],
                "credits":{"lecture":2,"exercise":0,"lab":0},
                "instructionLanguage":"en", "format":{"kinds":["lecture"]},
                "schedule":[{"kind":"weekly","day":2,"periods":[5,6]}],
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/offering-1"
            })],
        )
        .unwrap();
        assert_eq!(cached["detailIds"], json!([]));
        assert_eq!(cached["lmsOnly"][0]["offering"]["detailed"], true);
    }

    fn run(service: &WorkspaceService, intent: WorkspaceIntent) -> Result<Value, WorkspaceError> {
        if let WorkspaceIntent::AddCatalogCourses { offerings, .. } = &intent {
            service.store.catalog_upsert_offerings(offerings).unwrap();
        }
        let expected_revision = service.snapshot()?.revision;
        service
            .mutate(WorkspaceMutation {
                expected_revision,
                intent,
            })
            .map(|snapshot| snapshot.workspace)
    }

    fn offering(id: &str, code: &str, quarters: &[&str]) -> Value {
        json!({
            "id":id, "year":2026, "quarters":quarters, "code":code,
            "title":{"en":code}, "instructors":[], "credits":{"lecture":2,"exercise":0,"lab":0},
            "instructionLanguage":"en", "format":{"kinds":["lecture"]}, "media":{},
            "academicUnit":{}, "intensive":false, "fetchedAt":{},
            "schedule":[{"kind":"weekly","day":1,"periods":[1,2],"room":{"en":"W1"}}],
            "syllabusUrl":format!("https://syllabus.s.isct.ac.jp/courses/{id}")
        })
    }

    #[test]
    fn catalog_add_enforces_first_year_eligibility_atomically_and_preserves_existing_data() {
        let service = service("catalog-first-year");
        run(
            &service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: "2026-3Q".into(),
                offerings: vec![offering("202630300", "CSC.T301", &["3Q"])],
            },
        )
        .unwrap();
        run(
            &service,
            WorkspaceIntent::SetStudyProfile {
                admission_year: Some(2026),
                study_enrollments: vec![StudyEnrollment {
                    stage: crate::study_profile::StudyStage::Undergraduate,
                    entry_kind: crate::study_profile::StudyEntryKind::Regular,
                    entry_grade: Some(1),
                    admitted_term: "2026-1Q".into(),
                }],
            },
        )
        .unwrap();
        let before = service.snapshot().unwrap();
        let error = run(
            &service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: "2026-3Q".into(),
                offerings: vec![
                    offering("202630100", "CSC.T101", &["3Q"]),
                    offering("202630200", "LAH.T209-01", &["3Q"]),
                ],
            },
        )
        .unwrap_err();
        assert!(matches!(error, WorkspaceError::InvalidIntent(_)));
        let rejected = service.snapshot().unwrap();
        assert_eq!(rejected.revision, before.revision);
        assert!(rows(&rejected.workspace, "courses")
            .unwrap()
            .any(|course| is_live(course) && course["code"] == "CSC.T301"));
        assert!(!rows(&rejected.workspace, "courses")
            .unwrap()
            .any(|course| course["code"] == "CSC.T101"));

        let first_year = run(
            &service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: "2026-3Q".into(),
                offerings: vec![offering("202630100", "CSC.T101", &["3Q"])],
            },
        )
        .unwrap();
        assert!(rows(&first_year, "courses")
            .unwrap()
            .any(|course| is_live(course) && course["code"] == "CSC.T101"));

        let second_year = run(
            &service,
            WorkspaceIntent::AddCatalogCourses {
                term_id: "2027-3Q".into(),
                offerings: vec![offering("202630200", "CSC.T201", &["3Q"])],
            },
        )
        .unwrap();
        assert!(rows(&second_year, "courses")
            .unwrap()
            .any(|course| is_live(course) && course["code"] == "CSC.T201"));
    }

    #[test]
    fn catalog_add_checks_authoritative_content_in_every_placed_quarter() {
        let service = service("catalog-authority");
        run(
            &service,
            WorkspaceIntent::SetStudyProfile {
                admission_year: Some(2026),
                study_enrollments: vec![
                    StudyEnrollment {
                        stage: crate::study_profile::StudyStage::Undergraduate,
                        entry_kind: crate::study_profile::StudyEntryKind::Regular,
                        entry_grade: Some(1),
                        admitted_term: "2026-1Q".into(),
                    },
                    StudyEnrollment {
                        stage: crate::study_profile::StudyStage::Master,
                        entry_kind: crate::study_profile::StudyEntryKind::Regular,
                        entry_grade: Some(1),
                        admitted_term: "2026-4Q".into(),
                    },
                ],
            },
        )
        .unwrap();
        let canonical = offering("202630209", "LAH.T209-01", &["3Q", "4Q"]);
        service
            .store
            .catalog_upsert_offerings(std::slice::from_ref(&canonical))
            .unwrap();
        let before = service.snapshot().unwrap();

        // The requested 4Q is a master's term, but the same offering would
        // also be placed into undergraduate 3Q and must be rejected there.
        let mut submitted = canonical;
        submitted["code"] = json!("CSC.T101");
        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: before.revision.clone(),
                intent: WorkspaceIntent::AddCatalogCourses {
                    term_id: "2026-4Q".into(),
                    offerings: vec![submitted],
                },
            })
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "LAH.T209-01 is not available to a regular first-year undergraduate student."
        );
        let after = service.snapshot().unwrap();
        assert_eq!(after.revision, before.revision);
        assert!(!rows(&after.workspace, "courses").unwrap().any(is_live));
    }

    /// Live plan items as `(id, courseId, termId)`.
    fn live_plans(workspace: &Value) -> Vec<(String, String, String)> {
        let mut plans: Vec<_> = rows(workspace, "planItems")
            .unwrap()
            .filter(|row| is_live(row))
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_string(),
                    row["courseId"].as_str().unwrap().to_string(),
                    row["termId"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        plans.sort_by(|a, b| (&a.1, &a.2).cmp(&(&b.1, &b.2)));
        plans
    }

    fn placed_terms(workspace: &Value, course_id: &str) -> Vec<String> {
        let mut terms: Vec<_> = rows(workspace, "memberships")
            .unwrap()
            .chain(rows(workspace, "planItems").unwrap())
            .filter(|row| is_live(row) && row["courseId"] == course_id)
            .map(|row| row["termId"].as_str().unwrap().to_string())
            .collect();
        terms.sort();
        terms
    }

    #[test]
    fn a_catalog_add_plans_every_quarter_and_undo_reverses_exactly_that_add() {
        let service = service("catalog-quarters");
        let add = |term: &str, offering: Value| {
            service
                .store
                .catalog_upsert_offerings(std::slice::from_ref(&offering))
                .unwrap();
            run(
                &service,
                WorkspaceIntent::AddCatalogCourses {
                    term_id: term.into(),
                    offerings: vec![offering],
                },
            )
            .unwrap()
        };
        // A 3Q–4Q offering added outside its quarters stays in the quarter on screen.
        let outside = add("2026-2Q", offering("202630030", "CCC.C300", &["3Q", "4Q"]));
        assert_eq!(placed_terms(&outside, "2026-202630030"), vec!["2026-2Q"]);

        add("2026-3Q", offering("202630010", "AAA.A100", &["3Q"]));
        add("2026-4Q", offering("202630020", "BBB.B200", &["4Q"]));
        let before = live_plans(&service.snapshot().unwrap().workspace);
        let replaced_id = before
            .iter()
            .find(|plan| plan.1 == "2026-202630010")
            .unwrap()
            .0
            .clone();
        // Added in 3Q, the 3Q–4Q offering goes into both quarters; the clashing
        // draft in 3Q leaves the timetable.
        let added = add("2026-3Q", offering("202630020", "BBB.B200", &["3Q", "4Q"]));
        assert_eq!(
            placed_terms(&added, "2026-202630020"),
            vec!["2026-3Q", "2026-4Q"]
        );
        assert!(placed_terms(&added, "2026-202630010").is_empty());
        let course = rows(&added, "courses")
            .unwrap()
            .find(|row| row["id"] == "2026-202630020")
            .unwrap();
        assert_eq!(
            course["termId"], "2026-4Q",
            "an existing course keeps its quarter"
        );

        // Undo removes the 3Q plan item the add placed, keeps the earlier 4Q
        // one, and brings back the replaced draft itself.
        let undone = run(
            &service,
            WorkspaceIntent::UndoCatalogCourses {
                term_id: "2026-3Q".into(),
                offering_ids: vec!["202630020".into()],
            },
        )
        .unwrap();
        assert_eq!(live_plans(&undone), before);
        assert!(live_plans(&undone).iter().any(|plan| plan.0 == replaced_id));
        assert!(matches!(
            run(
                &service,
                WorkspaceIntent::UndoCatalogCourses {
                    term_id: "2026-3Q".into(),
                    offering_ids: vec!["202630020".into()],
                },
            ),
            Err(WorkspaceError::InvalidIntent(_))
        ));

        // Off the timetable means off every quarter.
        add("2026-3Q", offering("202630020", "BBB.B200", &["3Q", "4Q"]));
        let off = run(
            &service,
            WorkspaceIntent::SetCourseOnTimetable {
                course_id: "2026-202630020".into(),
                on_timetable: false,
            },
        )
        .unwrap();
        assert!(placed_terms(&off, "2026-202630020").is_empty());

        // Deleting a catalog course removes it and its placements.
        let deleted = run(
            &service,
            WorkspaceIntent::DeleteCourseBatch {
                course_ids: vec!["2026-202630010".into()],
            },
        )
        .unwrap();
        assert!(placed_terms(&deleted, "2026-202630010").is_empty());
        assert!(rows(&deleted, "courses")
            .unwrap()
            .any(|row| row["id"] == "2026-202630010" && !is_live(row)));
    }

    fn insert_school_row(service: &WorkspaceService, table: &str, value: Value) {
        let connection = rusqlite::Connection::open(service.store.path()).unwrap();
        connection
            .execute(
                &format!(
                    "INSERT INTO {table} (id,value,hash,updated_at,updated_by,rev)
                     VALUES (?1,?2,'seed','2026-10-01T00:00:00Z','lms',1)"
                ),
                rusqlite::params![value["id"].as_str().unwrap(), value.to_string()],
            )
            .unwrap();
    }

    #[test]
    fn lms_merge_adds_memberships_joins_quarters_retimes_refreshes_and_relinks() {
        let service = service("lms-apply");
        service
            .store
            .catalog_upsert_offerings(&[json!({
                "id":"202630001", "year":2026, "quarters":["3Q","4Q"], "code":"MTH.B301",
                "title":{"en":"Long Course"}, "instructors":[{"en":"Ada"}],
                "instructionLanguage":"en", "format":{"kinds":["lecture"]}, "media":{},
                "credits":{"lecture":2.0,"exercise":0.0,"lab":0.0},
                "schedule":[{"kind":"weekly","day":1,"periods":[1,2],"room":{"en":"W1"}}],
                "academicUnit":{"en":"Science"}, "intensive":false,
                "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/2026/1/202630001", "fetchedAt":{}
            })])
            .unwrap();
        let lms = |id: u64, offering: Option<&str>, shortname: &str, title: &str, grid: Value| {
            json!({
                "id":format!("lms-course:{id}"), "kind":"lms-course", "lmsId":id,
                "offeringId":offering, "shortname":shortname, "title":{"en":title},
                "year":2026, "quarters":["3Q"], "timetables":{"2026-3Q":grid},
                "rev":1, "updatedAt":"2026-10-01T00:00:00Z", "updatedBy":"lms"
            })
        };
        insert_school_row(
            &service,
            "lms_courses",
            lms(
                501,
                Some("202630001"),
                "MTH.B301[2026]",
                "Long Course",
                json!([{"day":1,"periods":[1,2]}]),
            ),
        );
        insert_school_row(
            &service,
            "lms_courses",
            lms(
                502,
                None,
                "LAH.T209-01[2026]",
                "Humanities",
                json!([{"day":2,"periods":[3,4]}]),
            ),
        );
        insert_school_row(
            &service,
            "lms_courses",
            lms(
                503,
                None,
                "SEM.S100[2026]",
                "Seminar &amp; Lab",
                json!([{"day":4,"periods":[1,2]}]),
            ),
        );
        insert_school_row(
            &service,
            "lms_courses",
            lms(
                504,
                Some("202630002"),
                "PHY.P200[2026]",
                "Physics &amp; More",
                json!([{"day":3,"periods":[1,2]}]),
            ),
        );
        insert_school_row(
            &service,
            "assignments",
            json!({
                "id":"lms-assignment:9", "kind":"assignment", "courseId":"2026-lms-502", "lmsCourseId":502,
                "title":"Essay", "details":"", "dueAt":null, "source":{"provider":"lms"},
                "rev":1, "updatedAt":"2026-10-01T00:00:00Z", "updatedBy":"lms"
            }),
        );
        let save = |code: &str, day: u64| {
            run(
                &service,
                WorkspaceIntent::SaveCourse {
                    draft: json!({"title":code,"code":code,"meetings":[{"days":[day],"periods":[5,6],"startTime":"14:20","endTime":"16:00","room":"S-1"}]}),
                    term_id: "2026-3Q".into(),
                    existing_id: None,
                },
            )
            .unwrap();
            let workspace = service.snapshot().unwrap().workspace;
            let id = rows(&workspace, "courses")
                .unwrap()
                .find(|row| row["code"] == code)
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string();
            id
        };
        let humanities = save("LAH.T209", 2);
        let dropped = save("XYZ.A100", 5);

        let snapshot = service.snapshot().unwrap();
        let preview = lms_merge_preview(
            &snapshot.workspace,
            &snapshot.school_records,
            "term",
            "2026-3Q",
            &service.store.catalog_search("").unwrap(),
        )
        .unwrap();
        let titles: Vec<_> = preview["lmsOnly"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["title"].as_str().unwrap().to_string())
            .collect();
        assert!(titles.contains(&"Seminar & Lab".to_string()), "{titles:?}");

        let applied = run(
            &service,
            WorkspaceIntent::ApplyLmsMerge {
                terms: vec!["2026-3Q".into()],
                add: vec!["lms:501:2026-3Q".into(), "lms:504:2026-3Q".into()],
                keep: vec![],
                retime: vec![format!("local:{humanities}")],
            },
        )
        .unwrap();
        // LMS courses join as LMS memberships, also in their quarter outside the review.
        let membership = rows(&applied, "memberships")
            .unwrap()
            .find(|row| row["id"] == "lms-membership:501:2026-4Q")
            .unwrap();
        assert_eq!(membership["status"], "lms-member");
        assert_eq!(membership["evidence"]["provider"], "lms");
        assert_eq!(
            placed_terms(&applied, "2026-202630001"),
            vec!["2026-3Q", "2026-4Q"]
        );
        assert!(rows(&applied, "planItems")
            .unwrap()
            .all(|row| row["courseId"] != "2026-202630001"));
        let course = |workspace: &Value, id: &str| {
            rows(workspace, "courses")
                .unwrap()
                .find(|row| row["id"] == id)
                .unwrap()
                .clone()
        };
        let long = course(&applied, "2026-202630001");
        assert_eq!(long["source"]["provider"], "lms");
        assert_eq!(long["meetings"][0]["room"], "W1");
        assert_eq!(course(&applied, "2026-202630002")["credits"], 0.0);
        assert_eq!(
            course(&applied, "2026-202630002")["title"],
            "Physics & More"
        );
        // The matched local course takes the LMS time and keeps its room.
        let retimed = course(&applied, &humanities);
        assert_eq!(retimed["meetings"][0]["periods"], json!([3, 4]));
        assert_eq!(retimed["meetings"][0]["room"], "S-1");
        // The unchecked local course leaves the timetable.
        assert!(placed_terms(&applied, &dropped).is_empty());
        // LMS work follows the course it was matched with.
        let essay = rows(&applied, "assignments")
            .unwrap()
            .find(|row| row["id"] == "lms-assignment:9")
            .unwrap();
        assert_eq!(essay["courseId"], humanities.as_str());

        // Once the syllabus gives the details, the LMS course takes them.
        service
            .store
            .catalog_upsert_offerings(&[json!({
                "id":"202630002", "year":2026, "quarters":["3Q"], "code":"PHY.P200",
                "title":{"en":"Physics"}, "instructors":[], "instructionLanguage":"en",
                "format":{"kinds":["lecture"]}, "media":{}, "credits":{"lecture":1.0,"exercise":0.0,"lab":0.0},
                "schedule":[{"kind":"weekly","day":3,"periods":[1,2]}], "academicUnit":{"en":"Science"},
                "intensive":false, "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/2026/1/202630002", "fetchedAt":{}
            })])
            .unwrap();
        let refreshed = run(
            &service,
            WorkspaceIntent::ApplyLmsMerge {
                terms: vec!["2026-3Q".into()],
                add: vec![],
                keep: vec![],
                retime: vec![],
            },
        )
        .unwrap();
        let physics = course(&refreshed, "2026-202630002");
        assert_eq!(physics["credits"], 1.0);
        assert_eq!(physics["title"], "Physics");
        assert_eq!(physics["names"]["title"]["en"], "Physics");

        // Taking an LMS course off the timetable removes it from every quarter.
        let off = run(
            &service,
            WorkspaceIntent::SetCourseOnTimetable {
                course_id: "2026-202630001".into(),
                on_timetable: false,
            },
        )
        .unwrap();
        assert!(placed_terms(&off, "2026-202630001").is_empty());
    }
}
