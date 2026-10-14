//! Authoritative workspace snapshots and user intents.
//!
//! A mutation checks the revision and applies every row change in one SQLite
//! IMMEDIATE transaction.  The returned snapshot is assembled and validated
//! before that transaction commits.

use crate::data::{DataError, DataStore, WorkspaceWrite};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use thiserror::Error;
#[cfg(feature = "ts")]
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub revision: String,
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
    PatchSettings {
        #[cfg_attr(feature = "ts", ts(type = "unknown"))]
        patch: Value,
    },
    SetAdmissionYear {
        #[cfg_attr(feature = "ts", ts(type = "number | null"))]
        admission_year: Option<i32>,
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
}

impl WorkspaceService {
    pub fn new(store: DataStore) -> Self {
        Self { store }
    }

    pub fn snapshot(&self) -> Result<WorkspaceSnapshot, WorkspaceError> {
        let (workspace, school_records) = self.store.workspace_snapshot_parts()?;
        snapshot(workspace, school_records).map_err(WorkspaceError::from)
    }

    pub fn mutate(&self, mutation: WorkspaceMutation) -> Result<WorkspaceSnapshot, WorkspaceError> {
        let result = self.store.workspace_write(
            &mutation.expected_revision,
            |write, current, school_records| {
                apply_intent(write, current, school_records, mutation.intent)?;
                let (workspace, school_records) = write.snapshot_parts()?;
                snapshot(workspace, school_records)
            },
        )?;
        Ok(result)
    }
}

fn snapshot(workspace: Value, school_records: Value) -> Result<WorkspaceSnapshot, DataError> {
    let revision = snapshot_revision(&workspace, &school_records)?;
    Ok(WorkspaceSnapshot {
        revision,
        workspace,
        school_records,
    })
}

pub(crate) fn snapshot_revision(
    workspace: &Value,
    school_records: &Value,
) -> Result<String, DataError> {
    let bytes = serde_json::to_vec(&(workspace, school_records)).map_err(DataError::from)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn apply_intent(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    school_records: &Value,
    intent: WorkspaceIntent,
) -> Result<(), DataError> {
    match intent {
        WorkspaceIntent::PatchSettings { patch } => {
            require_object(&patch, "settings patch")?;
            let mut settings = current
                .get("settings")
                .cloned()
                .unwrap_or_else(|| json!({}));
            merge_patch(&mut settings, patch);
            write.save_settings(&settings)
        }
        WorkspaceIntent::SetAdmissionYear { admission_year } => {
            if admission_year.is_some_and(|year| year < 2000) {
                return Err("admission year must be 2000 or later".into());
            }
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
            add_catalog_courses(write, current, &term_id, offerings)
        }
        WorkspaceIntent::UndoCatalogCourses {
            term_id,
            offering_ids,
        } => undo_catalog_courses(write, current, &term_id, offering_ids),
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
        let source_key = existing
            .get("sourceKey")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                existing
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            });
        write.put(
            "course_overrides",
            json!({
                "sourceKey": source_key,
                "courseId": existing.get("id"),
                "title": draft.get("title"),
                "instructor": draft.get("instructor"),
                "color": draft.get("color"),
                "lessonLog": existing.pointer("/overrides/lessonLog")
            }),
            None,
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

fn delete_course_batch(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_ids: Vec<String>,
) -> Result<(), DataError> {
    let ids: HashSet<_> = course_ids.into_iter().collect();
    for course in rows(current, "courses")?.filter(|row| {
        row.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(id))
    }) {
        let id = string(course, "id")?;
        if course.pointer("/source/provider").and_then(Value::as_str) == Some("manual") {
            write.delete("manual_courses", id, revision(course))?;
        }
    }
    for (field, table) in [
        ("memberships", "manual_memberships"),
        ("planItems", "plan_items"),
    ] {
        for row in rows(current, field)?.filter(|row| {
            row.get("courseId")
                .and_then(Value::as_str)
                .is_some_and(|id| ids.contains(id))
        }) {
            if row
                .pointer("/evidence/provider")
                .and_then(Value::as_str)
                .is_none_or(|provider| provider == "manual")
            {
                write.delete(table, string(row, "id")?, revision(row))?;
            }
        }
    }
    Ok(())
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
    if changes
        .language
        .as_deref()
        .is_some_and(|language| !matches!(language, "ja" | "en" | "ja-en" | "other"))
    {
        return Err("instruction language is invalid".into());
    }
    let ids: HashSet<_> = course_ids.into_iter().collect();
    for course in rows(current, "courses")?.filter(|row| {
        row.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(id))
    }) {
        let mut updated = course.clone();
        let object = object_mut(&mut updated, "course")?;
        if let Some(language) = &changes.language {
            object.insert(
                "instructionLanguage".into(),
                Value::String(language.clone()),
            );
        }
        if let Some(instructor) = changes
            .instructor
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            object.insert("instructor".into(), Value::String(instructor.into()));
        }
        if let Some(credits) = changes.credits {
            object.insert("credits".into(), Value::from(credits));
        }
        if course.pointer("/source/provider").and_then(Value::as_str) == Some("manual") {
            write.put("manual_courses", updated, revision(course))?;
        } else {
            let overlay = json!({"sourceKey": course.get("sourceKey").or_else(|| course.get("id")), "courseId":course.get("id"), "title":Value::Null, "instructor":changes.instructor, "color":Value::Null, "lessonLog":Value::Null});
            write.put("course_overrides", overlay, None)?;
        }
    }
    Ok(())
}

fn set_course_on_timetable(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    course_id: &str,
    on: bool,
) -> Result<(), DataError> {
    let course = visible_row(current, "courses", course_id)
        .ok_or_else(|| "course was not found".to_string())?;
    let term = string(course, "termId")?;
    let membership = rows(current, "memberships")?.find(|row| {
        row.get("courseId").and_then(Value::as_str) == Some(course_id)
            && row.get("termId").and_then(Value::as_str) == Some(term)
            && row.get("deletedAt").is_none()
    });
    let plan_item = rows(current, "planItems")?.find(|row| {
        row.get("courseId").and_then(Value::as_str) == Some(course_id)
            && row.get("termId").and_then(Value::as_str) == Some(term)
            && row.get("deletedAt").is_none()
    });
    if on {
        if membership.is_none() && plan_item.is_none() {
            write.put("manual_memberships", json!({"id":Uuid::now_v7().to_string(),"kind":"membership","rev":0,"updatedAt":"","updatedBy":"","termId":term,"courseId":course_id,"status":"declared","evidence":{"provider":"manual"}}), None)?;
        }
    } else {
        if let Some(row) = membership.filter(|row| {
            row.pointer("/evidence/provider").and_then(Value::as_str) == Some("manual")
        }) {
            write.delete("manual_memberships", string(row, "id")?, revision(row))?;
        }
        if let Some(row) = plan_item {
            write.delete("plan_items", string(row, "id")?, revision(row))?;
        }
    }
    Ok(())
}

fn add_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    offerings: Vec<Value>,
) -> Result<(), DataError> {
    validate_term(term_id)?;
    let locale = workspace_content_locale(current);
    let scheduled_ids: HashSet<_> = rows(current, "memberships")?
        .chain(rows(current, "planItems")?)
        .filter(|row| {
            row.get("termId").and_then(Value::as_str) == Some(term_id)
                && row.get("deletedAt").is_none_or(Value::is_null)
        })
        .filter_map(|row| row.get("courseId").and_then(Value::as_str))
        .collect();
    let scheduled: Vec<_> = rows(current, "courses")?
        .filter(|course| {
            course
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| scheduled_ids.contains(id))
                && course.get("termId").and_then(Value::as_str) == Some(term_id)
        })
        .collect();
    let mut removed_plans = HashSet::new();
    for offering in offerings {
        let offering_id = string(&offering, "id")?;
        let year = offering
            .get("year")
            .and_then(Value::as_i64)
            .or_else(|| term_id.get(..4).and_then(|year| year.parse().ok()))
            .ok_or_else(|| "catalog offering is missing academic year".to_string())?;
        let course_id =
            if offering_id.len() == 9 && offering_id.bytes().all(|byte| byte.is_ascii_digit()) {
                crate::data::canonical_course_id(
                    i32::try_from(year).map_err(|_| "academic year is invalid")?,
                    offering_id,
                )?
            } else {
                format!("{year}-{offering_id}")
            };
        let meetings = catalog_meetings(&offering, locale)?;
        for victim in &scheduled {
            if meetings_conflict(&meetings, victim.get("meetings").unwrap_or(&Value::Null)) {
                if let Some(plan) = rows(current, "planItems")?.find(|row| {
                    row.get("courseId") == victim.get("id")
                        && row.get("termId").and_then(Value::as_str) == Some(term_id)
                        && row.get("deletedAt").is_none_or(Value::is_null)
                }) {
                    let id = string(plan, "id")?;
                    if removed_plans.insert(id.to_string()) {
                        write.delete("plan_items", id, revision(plan))?;
                    }
                }
            }
        }
        let credits = offering
            .get("credits")
            .cloned()
            .unwrap_or_else(|| json!({"lecture":0,"exercise":0,"lab":0}));
        let total = ["lecture", "exercise", "lab"]
            .into_iter()
            .filter_map(|key| credits.get(key).and_then(Value::as_f64))
            .sum::<f64>();
        let title = localized(offering.get("title"), locale);
        let instructor = offering
            .get("instructors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|value| localized(Some(value), locale))
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        let source_provider = offering
            .get("_sourceProvider")
            .and_then(Value::as_str)
            .unwrap_or("syllabus");
        let source_external_id = offering
            .get("_sourceExternalId")
            .and_then(Value::as_str)
            .unwrap_or(offering_id);
        let existing = rows(current, "courses")?
            .find(|course| course.get("id").and_then(Value::as_str) == Some(&course_id));
        let course = write.put(
            "manual_courses",
            json!({
                "id":course_id, "kind":"course", "termId":term_id, "offeringId":offering_id,
                "names":{"title":offering.get("title"),"instructor":offering.get("instructors").and_then(Value::as_array).and_then(|items| items.first())},
                "code":offering.get("code").and_then(Value::as_str).unwrap_or_default(),
                "title":title, "instructor":instructor, "credits":total,
                "creditSplit":credits,
                "instructionLanguage":normalize_language(offering.get("instructionLanguage").and_then(Value::as_str)),
                "color":"#556b8d", "meetings":meetings,
                "url":offering.get("syllabusUrl"),
                "source":{"provider":source_provider,"externalId":source_external_id}
            }),
            existing.and_then(revision),
        )?;
        let course_id = string(&course, "id")?;
        write.put(
            "plan_items",
            json!({"kind":"plan-item","termId":term_id,"courseId":course_id,"offeringId":offering_id,"state":"planned"}),
            None,
        )?;
    }
    Ok(())
}

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
    let scope: HashSet<_> = terms.iter().cloned().collect();
    let add: HashSet<_> = add.into_iter().collect();
    let keep: HashSet<_> = keep.into_iter().collect();
    let retime: HashSet<_> = retime.into_iter().collect();
    let scheduled_ids: HashSet<_> = rows(current, "memberships")?
        .chain(rows(current, "planItems")?)
        .filter(|row| row.get("deletedAt").is_none_or(Value::is_null))
        .filter_map(|row| row.get("courseId").and_then(Value::as_str))
        .collect();
    let mut unmatched_local: HashSet<String> = rows(current, "courses")?
        .filter(|course| {
            course
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| scheduled_ids.contains(id))
                && course
                    .get("termId")
                    .and_then(Value::as_str)
                    .is_some_and(|term| scope.contains(term))
        })
        .filter_map(|course| course.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let offerings = current.get("offerings").and_then(Value::as_array);
    let mut selected = Vec::new();
    for lms in school_records
        .get("lmsCourses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if lms.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let lms_id = lms
            .get("lmsId")
            .and_then(Value::as_u64)
            .ok_or_else(|| "LMS course is missing lmsId".to_string())?;
        let offering_id = lms.get("offeringId").and_then(Value::as_str);
        for term in lms_terms(lms)
            .into_iter()
            .filter(|term| scope.contains(term))
        {
            let matched = rows(current, "courses")?.find(|course| {
                course.get("termId").and_then(Value::as_str) == Some(&term)
                    && course
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| scheduled_ids.contains(id))
                    && (offering_id.is_some()
                        && course.get("offeringId").and_then(Value::as_str) == offering_id
                        || base_code(
                            course
                                .get("code")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ) == base_code(
                            lms.get("shortname")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ))
            });
            if let Some(course) = matched {
                let course_id = string(course, "id")?;
                unmatched_local.remove(course_id);
                if retime.contains(&format!("local:{course_id}")) {
                    if let Some(grid) = lms.pointer(&format!("/timetables/{term}")) {
                        if course.pointer("/source/provider").and_then(Value::as_str)
                            == Some("manual")
                        {
                            let mut updated = course.clone();
                            object_mut(&mut updated, "course")?
                                .insert("meetings".into(), meetings_from_grid(grid)?);
                            write.put("manual_courses", updated, revision(course))?;
                        }
                    }
                }
                continue;
            }
            let key = format!("lms:{lms_id}:{term}");
            if !add.contains(&key) {
                continue;
            }
            if let Some(offering) = offering_id.and_then(|id| {
                offerings.and_then(|rows| {
                    rows.iter()
                        .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                })
            }) {
                let mut offering = offering.clone();
                object_mut(&mut offering, "offering")?
                    .insert("_sourceProvider".into(), Value::String("lms".into()));
                object_mut(&mut offering, "offering")?.insert(
                    "_sourceExternalId".into(),
                    Value::String(lms_id.to_string()),
                );
                if let Some(grid) = lms.pointer(&format!("/timetables/{term}")) {
                    object_mut(&mut offering, "offering")?
                        .insert("schedule".into(), grid_schedule(grid)?);
                }
                selected.push((term.clone(), offering));
            } else {
                let id = offering_id
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("lms-{lms_id}"));
                selected.push((term.clone(), json!({
                    "id":id, "year":term.get(..4).and_then(|year|year.parse::<i64>().ok()),
                    "_sourceProvider":"lms", "_sourceExternalId":lms_id.to_string(),
                    "code":lms.get("shortname"), "title":lms.get("title"), "instructors":[],
                    "credits":{"lecture":0,"exercise":0,"lab":0}, "instructionLanguage":"other",
                    "schedule":lms.pointer(&format!("/timetables/{term}")).map(grid_schedule).transpose()?.unwrap_or_else(|| json!([])),
                    "syllabusUrl":Value::Null
                })));
            }
        }
    }
    for term in &terms {
        let term_offerings = selected
            .iter()
            .filter(|(candidate, _)| candidate == term)
            .map(|(_, offering)| offering.clone())
            .collect::<Vec<_>>();
        if !term_offerings.is_empty() {
            add_catalog_courses(write, current, term, term_offerings)?;
        }
    }
    for course_id in unmatched_local {
        if !keep.contains(&format!("local:{course_id}")) {
            set_course_on_timetable(write, current, &course_id, false)?;
        }
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
        values.extend(
            rows(workspace, "courses")?
                .filter(|row| row.get("deletedAt").is_none_or(Value::is_null))
                .filter_map(|row| {
                    row.get("termId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                }),
        );
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
    let term_scope: HashSet<_> = terms.iter().cloned().collect();
    let locale = workspace_content_locale(workspace);
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
            let title = localized(
                offering
                    .and_then(|row| row.get("title"))
                    .or_else(|| row.get("title")),
                locale,
            );
            let code = offering
                .and_then(|row| row.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    let shortname = row
                        .get("shortname")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
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
    let scheduled: HashSet<_> = rows(workspace, "memberships")?
        .chain(rows(workspace, "planItems")?)
        .filter(|row| row.get("deletedAt").is_none_or(Value::is_null))
        .filter_map(|row| row.get("courseId").and_then(Value::as_str))
        .collect();
    let mut common = Vec::new();
    let mut local_only = Vec::new();
    let mut matched = HashSet::new();
    for term in &terms {
        for course in rows(workspace, "courses")?.filter(|course| {
            course.get("termId").and_then(Value::as_str) == Some(term)
                && course.get("deletedAt").is_none_or(Value::is_null)
                && course
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| scheduled.contains(id))
        }) {
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
                matched.insert(
                    string(entry, "key")
                        .map_err(WorkspaceError::from)?
                        .to_string(),
                );
                let time_differs = entry
                    .get("gridMeetings")
                    .is_some_and(|grid| !grid.is_null())
                    && course_slots(course)
                        != value_string_set(entry.get("slots").unwrap_or(&Value::Null));
                common.push(
                    json!({"termId":term,"local":course,"lms":entry,"timeDiffers":time_differs}),
                );
            } else {
                local_only.push(json!({"key":format!("local:{}",string(course,"id").map_err(WorkspaceError::from)?),"termId":term,"course":course,"slots":course_slots(course)}));
            }
        }
    }
    let lms_only = remote
        .into_iter()
        .filter(|entry| {
            !matched.contains(entry.get("key").and_then(Value::as_str).unwrap_or_default())
        })
        .collect::<Vec<_>>();
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

fn grid_schedule(grid: &Value) -> Result<Value, DataError> {
    let mut schedule = Vec::new();
    for row in grid.as_array().into_iter().flatten() {
        schedule.push(json!({"kind":"weekly","day":row.get("day"),"periods":row.get("periods")}));
    }
    Ok(Value::Array(schedule))
}

fn meetings_from_grid(grid: &Value) -> Result<Value, DataError> {
    catalog_meetings(&json!({"schedule":grid_schedule(grid)?}), "ja")
}

fn undo_catalog_courses(
    write: &WorkspaceWrite<'_, '_>,
    current: &Value,
    term_id: &str,
    offering_ids: Vec<String>,
) -> Result<(), DataError> {
    validate_term(term_id)?;
    let offering_ids: HashSet<_> = offering_ids.into_iter().collect();
    let course_ids: HashSet<_> = rows(current, "courses")?
        .filter(|course| {
            course.get("termId").and_then(Value::as_str) == Some(term_id)
                && course
                    .get("offeringId")
                    .and_then(Value::as_str)
                    .is_some_and(|id| offering_ids.contains(id))
        })
        .filter_map(|course| course.get("id").and_then(Value::as_str))
        .collect();
    for plan in rows(current, "planItems")?.filter(|row| {
        row.get("courseId")
            .and_then(Value::as_str)
            .is_some_and(|id| course_ids.contains(id))
            && row.get("deletedAt").is_none_or(Value::is_null)
    }) {
        write.delete("plan_items", string(plan, "id")?, revision(plan))?;
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

fn meetings_conflict(left: &Value, right: &Value) -> bool {
    let slots = |meetings: &Value| -> HashSet<(u64, u64)> {
        meetings
            .as_array()
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
                    .flat_map(move |period| days.clone().into_iter().map(move |day| (day, period)))
            })
            .collect()
    };
    let left = slots(left);
    !left.is_disjoint(&slots(right))
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
    let source_key = course
        .get("sourceKey")
        .and_then(Value::as_str)
        .unwrap_or(course_id);
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
    write
        .put(
            "course_overrides",
            json!({"sourceKey":source_key,"courseId":course_id,"lessonLog":log}),
            None,
        )
        .map(|_| ())
}

fn merge_patch(target: &mut Value, patch: Value) {
    let (Some(target), Value::Object(patch)) = (target.as_object_mut(), patch) else {
        return;
    };
    for (key, value) in patch {
        if value.is_object() && target.get(&key).is_some_and(Value::is_object) {
            if let Some(child) = target.get_mut(&key) {
                merge_patch(child, value);
            }
        } else {
            target.insert(key, value);
        }
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
mod tests {
    use super::*;

    fn service(name: &str) -> WorkspaceService {
        let path =
            std::env::temp_dir().join(format!("isct-workspace-{name}-{}.db", Uuid::new_v4()));
        WorkspaceService::new(DataStore::initialize(path).unwrap().0)
    }

    #[test]
    fn stale_revision_cannot_overwrite_a_newer_settings_change() {
        let service = service("conflict");
        let original = service.snapshot().unwrap();
        let changed = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision.clone(),
                intent: WorkspaceIntent::PatchSettings {
                    patch: json!({"appearance":"dark"}),
                },
            })
            .unwrap();
        assert_ne!(changed.revision, original.revision);
        assert_eq!(changed.workspace["settings"]["appearance"], "dark");

        let error = service
            .mutate(WorkspaceMutation {
                expected_revision: original.revision,
                intent: WorkspaceIntent::PatchSettings {
                    patch: json!({"density":"compact"}),
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

    #[test]
    fn catalog_course_preserves_localized_fields_credits_rooms_and_long_period_ranges() {
        let service = service("catalog-course");
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
        assert_ne!(next.revision, original.revision);
        let task = next.workspace["scheduledTasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == task_id)
            .unwrap();
        assert_eq!(task["lastOccurrenceAt"], "2026-09-29T12:00:00.000Z");
        assert_eq!(task["lastRunAt"], "2026-09-29T12:00:01.000Z");
        assert_eq!(task["lastResult"], "Export completed");
        assert_eq!(service.snapshot().unwrap().revision, next.revision);
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
}
