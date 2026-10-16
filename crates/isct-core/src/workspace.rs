//! Authoritative workspace snapshots and user intents.
//!
//! A mutation checks the revision and applies every row change in one SQLite
//! IMMEDIATE transaction.  The returned snapshot is assembled and validated
//! before that transaction commits.

mod calendar;
mod catalog;
mod courses;
mod lms_merge;

pub(crate) use calendar::set_assignment_mark;
use calendar::*;
pub use catalog::catalog_course;
pub(crate) use catalog::planned_catalog_course;
use catalog::*;
use courses::*;
pub use lms_merge::lms_merge_preview;
use lms_merge::*;

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
        let (workspace, school_records, revision) = self.store.workspace_write(
            &mutation.expected_revision,
            |write, current, school_records| {
                apply_intent(write, current, school_records, mutation.intent, &mut undo)
            },
        )?;
        // Only a committed add or undo changes what Undo reverses.
        *self
            .catalog_undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = undo;
        snapshot(workspace, school_records, revision).map_err(WorkspaceError::from)
    }
}

/// Saves one personal plan and returns its id. Shared by the `SavePlan`
/// intent and agent tools, so both meet one set of plan rules.
pub(crate) fn save_plan(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    mut draft: Value,
    existing_id: Option<String>,
) -> Result<String, DataError> {
    crate::plans::canonicalize_plan_draft(&mut draft)
        .map_err(|error| DataError::InvalidInput(error.to_string()))?;
    let id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
    save_simple_record(
        write,
        current,
        "plans",
        "plans",
        draft,
        Some(id.clone()),
        |_| Ok(()),
    )?;
    Ok(id)
}

pub(crate) fn delete_plan(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    plan_id: &str,
) -> Result<(), DataError> {
    delete_visible(write, current, "plans", "plans", plan_id)
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
            // Older calendar editors also predate configurable teaching
            // periods and must not erase a newer profile's complete table.
            if academic_profile.get("teachingPeriods").is_none() {
                if let Some(periods) = current.pointer("/academicProfile/teachingPeriods") {
                    academic_profile["teachingPeriods"] = periods.clone();
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
        WorkspaceIntent::SavePlan { draft, existing_id } => {
            save_plan(write, current, draft, existing_id).map(|_| ())
        }
        WorkspaceIntent::DeletePlan { plan_id } => delete_plan(write, current, &plan_id),
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

pub(crate) fn workspace_content_locale(workspace: &Value) -> &'static str {
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
#[path = "tests/workspace.rs"]
mod tests;
