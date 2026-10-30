//! User intents.
//!
//! A mutation checks the revision and applies every row change in one SQLite
//! IMMEDIATE transaction; the fields its writes feed are read back and
//! validated before that transaction commits, and it answers what changed.

mod calendar;
mod catalog;
mod courses;

pub(crate) use calendar::set_assignment_mark;
use calendar::*;
use catalog::*;
pub(crate) use catalog::{catalog_instructor_names, meetings_from_grid, planned_catalog_course};
use courses::*;

use crate::data::{DataError, DataStore, WorkspaceWrite};
use crate::schedule::{ClassGroup, RecentKind};
use crate::study_profile::StudyEnrollment;
use crate::workspace_changes::{self, WorkspaceChange};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use thiserror::Error;
#[cfg(feature = "ts")]
use ts_rs::TS;
use uuid::Uuid;

/// A committed intent as a shell that caches query answers hears it: what
/// the commit changed, the revision the next intent is made against, and the
/// commit's sequence.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCommit {
    /// As in a `workspace-changed` event of the same commit.
    pub changes: Vec<WorkspaceChange>,
    pub revision: String,
    /// `DataStore::commit_seq` after the commit.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub seq: u64,
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
    /// Copies every weekly event of `from_term_id` into `term_id`.
    CopyEvents {
        from_term_id: String,
        term_id: String,
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
    /// Replaces an exam's personal date, time and room. A field that is
    /// empty or equals the published value follows the source.
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
    },
    /// Marks one calendar session (`Occurrence.key`) as an exam, a quiz,
    /// cancelled (休講) or a normal class. The mark outranks every published
    /// source; marking a session what its sources already make it removes
    /// the mark.
    MarkSession {
        key: String,
        #[cfg_attr(
            feature = "ts",
            ts(type = "\"exam\" | \"quiz\" | \"cancelled\" | \"class\"")
        )]
        mark: String,
    },
    /// Links an exam to a course of its quarter, or to none. Choosing the
    /// course matching would pick returns the exam to automatic matching.
    LinkExam {
        exam_key: String,
        #[serde(default)]
        #[cfg_attr(feature = "ts", ts(optional))]
        course_id: Option<String>,
    },
    /// Deletes an exam from the user's schedule: a marked class session
    /// loses its mark, a published exam is unlinked from its course.
    /// `MarkSession` or `LinkExam` with the previous value restores it.
    RemoveExam {
        exam_key: String,
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
    /// Opening an item of Today's recent list: it stops being new until it
    /// changes again.
    MarkRecentSeen {
        list: RecentKind,
        id: String,
    },
    /// Today's read all: every item new now stops being new until it changes
    /// again, including those past the few Today shows.
    MarkAllRecentSeen,
    /// Chooses (or clears) the 物理学演習・実験 group of one semester,
    /// such as "2026-3Q4Q".
    SetClassGroup {
        semester: String,
        group: Option<ClassGroup>,
    },
    /// The desktop window's close prompt remembered the user's choice.
    SetCloseWindow {
        #[cfg_attr(feature = "ts", ts(type = "\"ask\" | \"quit\" | \"background\""))]
        action: String,
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
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    vec![
        WorkspaceCommit::decl(cfg),
        WorkspaceMutation::decl(cfg),
        WorkspaceIntent::decl(cfg),
        CourseBatchChanges::decl(cfg),
        WorkspaceChange::decl(cfg),
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

    /// The revision the next mutation is made against.
    pub fn revision(&self) -> Result<String, WorkspaceError> {
        Ok(self.store.intent_revision()?)
    }

    /// Applies `mutation` and returns what it changed, for a shell that
    /// queries again what reads those changes.
    pub fn mutate_commit(
        &self,
        mutation: WorkspaceMutation,
    ) -> Result<WorkspaceCommit, WorkspaceError> {
        let mut undo = self
            .catalog_undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let ((), revision, tables) =
            self.store
                .workspace_write(&mutation.expected_revision, |write, current, school| {
                    apply_intent(write, current, school, mutation.intent, &mut undo)
                })?;
        // Only a committed add or undo changes what Undo reverses.
        *self
            .catalog_undo
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = undo;
        Ok(WorkspaceCommit {
            changes: workspace_changes::changes_for(&tables)
                .into_iter()
                .collect(),
            revision,
            seq: self.store.commit_seq(),
        })
    }
}

/// Saves one weekly personal event in `term_id` and returns its id. Shared
/// by the `SaveEvent` intent and agent tools, so both meet one set of rules
/// (`data::records::validate_local_record`).
pub(crate) fn save_event(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    draft: &Value,
    term_id: &str,
    existing_id: Option<String>,
) -> Result<String, DataError> {
    validate_term(term_id)?;
    let id = existing_id.unwrap_or_else(|| Uuid::now_v7().to_string());
    let mut row = Map::new();
    for key in ["title", "room", "days", "startTime", "endTime", "color"] {
        row.insert(key.into(), draft.get(key).cloned().unwrap_or(Value::Null));
    }
    if let Some(Value::String(title)) = row.get_mut("title") {
        *title = title.trim().to_string();
    }
    if row.get("color").is_none_or(Value::is_null) {
        let color = crate::schedule::event_color(&Value::Object(row.clone()));
        row.insert("color".into(), Value::String(color));
    }
    match row.get_mut("room") {
        Some(Value::String(room)) => *room = room.trim().to_string(),
        Some(room) => *room = Value::String(String::new()),
        None => {}
    }
    if let Some(Value::Array(days)) = row.get_mut("days") {
        days.sort_by_key(|day| day.as_u64());
        days.dedup();
    }
    save_simple_record(
        write,
        current,
        "events",
        "personal_events",
        Value::Object(row),
        Some(id.clone()),
        |row| {
            row.insert("kind".into(), Value::String("event".into()));
            row.insert("termId".into(), Value::String(term_id.into()));
            Ok(())
        },
    )?;
    Ok(id)
}

pub(crate) fn delete_event(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    event_id: &str,
) -> Result<(), DataError> {
    delete_visible(write, current, "events", "personal_events", event_id)
}

/// Copies the weekly events of one quarter into another, for a new quarter
/// that keeps last quarter's fixed meetings.
fn copy_events(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    from_term_id: &str,
    term_id: &str,
) -> Result<(), DataError> {
    validate_term(from_term_id)?;
    if from_term_id == term_id {
        return Err("events cannot be copied into their own quarter".into());
    }
    let events: Vec<Value> = rows(current, "events")?
        .filter(|row| {
            row.get("deletedAt").is_none()
                && object_string(row, "termId").as_deref() == Some(from_term_id)
        })
        .cloned()
        .collect();
    if events.is_empty() {
        return Err("that quarter has no weekly events".into());
    }
    for event in events {
        save_event(write, current, &event, term_id, None)?;
    }
    Ok(())
}

fn apply_intent(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    school: &crate::schedule::SchoolReads,
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
        } => save_event(write, current, &draft, &term_id, existing_id).map(|_| ()),
        WorkspaceIntent::DeleteEvent { event_id } => delete_event(write, current, &event_id),
        WorkspaceIntent::CopyEvents {
            from_term_id,
            term_id,
        } => copy_events(write, current, &from_term_id, &term_id),
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
        } => {
            let row = crate::schedule::corrected_exam_override(
                current,
                school,
                &exam_key,
                crate::schedule::ExamCorrection {
                    date,
                    start_time,
                    end_time,
                    room,
                },
            )
            .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_exam_override(write, current, school, &exam_key, row)
        }
        WorkspaceIntent::MarkSession { key, mark } => {
            let row = crate::schedule::marked_session_override(current, school, &key, &mark)
                .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_exam_override(write, current, school, &key, row)
        }
        WorkspaceIntent::LinkExam {
            exam_key,
            course_id,
        } => {
            let row = crate::schedule::linked_exam_override(
                current,
                school,
                &exam_key,
                course_id.as_deref().filter(|id| !id.is_empty()),
            )
            .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_exam_override(write, current, school, &exam_key, row)
        }
        WorkspaceIntent::RemoveExam { exam_key } => {
            let row = crate::schedule::removed_exam_override(current, school, &exam_key)
                .map_err(|error| DataError::InvalidInput(error.to_string()))?;
            save_exam_override(write, current, school, &exam_key, row)
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
        WorkspaceIntent::MarkRecentSeen { list, id } => {
            let version = crate::schedule::recent_version(current, list, &id)
                .ok_or_else(|| DataError::from("recent item was not found"))?
                .to_string();
            save_recent_seen(
                write,
                current,
                [(crate::schedule::recent_key(list, &id), version)],
            )
        }
        WorkspaceIntent::MarkAllRecentSeen => {
            let now = chrono::Utc::now().fixed_offset();
            save_recent_seen(write, current, crate::schedule::unseen_recent(current, now))
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
        WorkspaceIntent::SetCloseWindow { action } => {
            let mut settings = current
                .get("settings")
                .cloned()
                .unwrap_or_else(|| json!({}));
            object_mut(&mut settings, "settings")?.insert("closeWindow".into(), action.into());
            write.save_settings(&settings)
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

/// Records recent items as seen at the `updatedAt` they were shown for.
fn save_recent_seen(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    seen_now: impl IntoIterator<Item = (String, String)>,
) -> Result<(), DataError> {
    let mut seen = current
        .get("recentSeen")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let entries = object_mut(&mut seen, "recent seen")?;
    // Only items still within the recent window can show again, so
    // older entries are dropped instead of kept forever.
    let since = chrono::Utc::now() - chrono::Duration::days(crate::schedule::RECENT_DAYS);
    entries.retain(|_, at| {
        at.as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .is_some_and(|at| at >= since)
    });
    for (key, version) in seen_now {
        entries.insert(key, Value::String(version));
    }
    write.save_setting("recentSeen", &seen)
}

/// One override per session key: write the new content, or remove the row
/// once nothing personal remains.
fn save_exam_override(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    school: &crate::schedule::SchoolReads,
    key: &str,
    row: Option<Value>,
) -> Result<(), DataError> {
    let existing = rows(current, "scheduleOverrides")?
        .find(|row| {
            row.get("deletedAt").is_none_or(Value::is_null)
                && row.get("targetKey").and_then(Value::as_str) == Some(key)
        })
        .and_then(|row| object_string(row, "id"));
    // A correction the exam adopted from its renamed row moves to the
    // exam's own key.
    if existing.is_none() {
        if let Some(id) = crate::schedule::adopted_exam_override(current, school, key) {
            delete_visible(
                write,
                current,
                "scheduleOverrides",
                "schedule_overrides",
                &id,
            )?;
        }
    }
    match (row, existing) {
        (Some(row), Some(id)) => save_simple_record(
            write,
            current,
            "scheduleOverrides",
            "schedule_overrides",
            row,
            Some(id),
            |_| Ok(()),
        ),
        // Without an id the store reuses the key's deleted row, if any:
        // target keys are unique, tombstones included.
        (Some(row), None) => write.put("schedule_overrides", row, None).map(|_| ()),
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

// Other modules' tests read back writes through its `ReadBack`.
#[cfg(test)]
#[path = "tests/workspace.rs"]
pub(crate) mod tests;
