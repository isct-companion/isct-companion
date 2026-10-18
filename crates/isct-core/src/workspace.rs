//! Authoritative workspace snapshots and user intents.
//!
//! A mutation checks the revision and applies every row change in one SQLite
//! IMMEDIATE transaction.  The returned snapshot is assembled and validated
//! before that transaction commits.

mod calendar;
mod catalog;
mod courses;

use calendar::*;
pub use catalog::catalog_course;
use catalog::*;
pub(crate) use catalog::{catalog_instructor_names, meetings_from_grid, planned_catalog_course};
use courses::*;

use crate::data::{DataError, DataStore, WorkspaceWrite};
use crate::schedule::ClassGroup;
use crate::study_profile::StudyEnrollment;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
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
    /// Replaces an exam's personal date, time, room and visibility. A field
    /// that is empty or equals the published value follows the source.
    EditExam {
        exam_key: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        date: Option<String>,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        start_time: Option<String>,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        end_time: Option<String>,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        room: Option<String>,
        #[serde(default)]
        hidden: bool,
    },
    /// Links an exam to a course of its quarter, or to none. Choosing the
    /// course matching would pick returns the exam to automatic matching.
    LinkExam {
        exam_key: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        course_id: Option<String>,
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
        let ((workspace, school_records, revision), content_revision) =
            self.store.workspace_snapshot_with_revision()?;
        Ok(WorkspaceSnapshot {
            workspace,
            school_records,
            revision,
            content_revision,
        })
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
                let selection = match &mutation.intent {
                    WorkspaceIntent::SelectTerm { term_id } => Some(term_id.clone()),
                    _ => None,
                };
                apply_intent(write, current, mutation.intent, &mut undo)?;
                let parts = if let Some(term) = selection {
                    let mut workspace = current.clone();
                    workspace["selectedTerm"] = term.into();
                    (workspace, school_records.clone(), write.revision()?)
                } else {
                    write.snapshot_parts()?
                };
                let result = snapshot(parts.0.clone(), parts.1.clone(), parts.2.clone())?;
                let hash = result.content_revision.clone();
                Ok((result, parts, hash))
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
    let content_revision = crate::data::content_revision(&workspace, &school_records)?;
    Ok(WorkspaceSnapshot {
        revision,
        content_revision,
        workspace,
        school_records,
    })
}

fn apply_intent(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
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
            academic_profile,
            exceptions,
        } => {
            require_object(&academic_profile, "academic profile")?;
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
        WorkspaceIntent::SaveScheduleOverride { draft, existing_id } => {
            if draft["targetKind"] == "exam" {
                return Err(DataError::InvalidInput(
                    "Use editExam or linkExam to change an exam.".into(),
                ));
            }
            save_simple_record(
                write,
                current,
                "scheduleOverrides",
                "schedule_overrides",
                draft,
                existing_id,
                |_| Ok(()),
            )
        }
        WorkspaceIntent::EditExam {
            exam_key,
            date,
            start_time,
            end_time,
            room,
            hidden,
        } => {
            let row = crate::schedule::corrected_exam_override(
                current,
                &exam_key,
                crate::schedule::ExamCorrection {
                    date,
                    start_time,
                    end_time,
                    room,
                    hidden,
                },
            )
            .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_exam_override(write, current, &exam_key, row)
        }
        WorkspaceIntent::LinkExam {
            exam_key,
            course_id,
        } => {
            let row = crate::schedule::linked_exam_override(
                current,
                &exam_key,
                course_id.as_deref().filter(|id| !id.is_empty()),
            )
            .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_exam_override(write, current, &exam_key, row)
        }
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

/// One override per exam: write the new content, or remove the row once
/// nothing personal remains.
fn save_exam_override(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    key: &str,
    row: Option<Value>,
) -> Result<(), DataError> {
    let existing = rows(current, "scheduleOverrides")?
        .find(|row| {
            row.get("deletedAt").is_none_or(Value::is_null)
                && row.get("targetKey").and_then(Value::as_str) == Some(key)
        })
        .and_then(|row| object_string(row, "id"));
    match (row, existing) {
        (Some(row), existing) => save_simple_record(
            write,
            current,
            "scheduleOverrides",
            "schedule_overrides",
            row,
            existing,
            |_| Ok(()),
        ),
        (None, Some(id)) => delete_visible(
            write,
            current,
            "scheduleOverrides",
            "schedule_overrides",
            &id,
        ),
        (None, None) => Ok(()),
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
