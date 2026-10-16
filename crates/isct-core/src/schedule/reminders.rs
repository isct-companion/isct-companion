use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration as StdDuration,
};

use chrono::{DateTime, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::calendar::{content_language, localized};
use super::{
    array, resolve_day_agenda, scheduled_courses, text, tokyo_date, visible, AgendaOptions,
    Occurrence, OccurrenceKind, ScheduleError,
};

pub const DEFAULT_ASSIGNMENT_LEADS: [i64; 2] = [1440, 60];

const REMINDER_GRACE_SECONDS: i64 = 5 * 60;
/// How long delivery waits for a shell's one-time legacy log import. A shell
/// that has no legacy log need not call the import at all.
const LEGACY_IMPORT_WAIT: StdDuration = StdDuration::from_secs(60);
const FIRED_TTL_SECONDS: i64 = 3 * 24 * 60 * 60;

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum NotificationSinkError {
    #[error("{0}")]
    Delivery(String),
}

pub trait NotificationSink: Send + Sync {
    fn deliver<'a>(
        &'a self,
        item: NotificationItem,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotificationSinkError>> + Send + 'a>>;
}

#[derive(Debug, thiserror::Error)]
pub enum ReminderError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Data(#[from] crate::data::DataError),
    #[error("{0}")]
    Storage(String),
    #[error(transparent)]
    Policy(#[from] ScheduleError),
    #[error(transparent)]
    Task(#[from] crate::runtime_tasks::RuntimeTaskError),
    #[error(transparent)]
    Delivery(#[from] NotificationSinkError),
}

#[derive(Clone)]
pub struct ReminderService(Arc<ReminderInner>);

struct ReminderInner {
    store: crate::data::DataStore,
    tasks: crate::runtime_tasks::RuntimeTasks,
    pass: Arc<tokio::sync::Semaphore>,
    next_tick: Mutex<tokio::time::Instant>,
    legacy_import_until: tokio::time::Instant,
    fired_path: PathBuf,
}

impl ReminderService {
    pub fn new(store: crate::data::DataStore) -> Self {
        let fired_path = fired_path(store.path());
        Self(Arc::new(ReminderInner {
            store,
            tasks: crate::runtime_tasks::RuntimeTasks::default(),
            pass: Arc::new(tokio::sync::Semaphore::new(1)),
            next_tick: Mutex::new(tokio::time::Instant::now()),
            legacy_import_until: tokio::time::Instant::now() + LEGACY_IMPORT_WAIT,
            fired_path,
        }))
    }

    /// Claims and delivers reminders due within the desktop's five-minute
    /// grace window. Concurrent wakeups coalesce into one pass.
    pub async fn tick(
        &self,
        sink: Arc<dyn NotificationSink>,
        now: DateTime<FixedOffset>,
    ) -> Result<bool, ReminderError> {
        if self.0.tasks.is_closed() {
            return Err(ReminderError::Task(
                crate::runtime_tasks::RuntimeTaskError::Stopped,
            ));
        }
        {
            let mut next = self
                .0
                .next_tick
                .lock()
                .map_err(|_| ReminderError::Storage("reminder clock is unavailable".into()))?;
            let instant = tokio::time::Instant::now();
            if instant < *next {
                return Ok(false);
            }
            *next = instant + StdDuration::from_secs(30);
        }
        let Ok(permit) = self.0.pass.clone().try_acquire_owned() else {
            return Ok(false);
        };
        let inner = self.0.clone();
        // Until the legacy import runs or its wait ends, a startup tick must
        // not deliver reminders the previous runtime already showed.
        let awaiting_import = tokio::time::Instant::now() < self.0.legacy_import_until;
        self.0
            .tasks
            .run(async move {
                let _permit = permit;
                let store = inner.store.clone();
                let fired_path = inner.fired_path.clone();
                let (initialized, permission, plan) = tokio::task::spawn_blocking(move || {
                    if !fired_path.is_file() {
                        if awaiting_import {
                            return Ok((
                                false,
                                false,
                                NotificationPlan {
                                    from: now.to_rfc3339(),
                                    through: now.to_rfc3339(),
                                    items: Vec::new(),
                                },
                            ));
                        }
                        // No legacy log arrived: start with an empty one.
                        // A later import still merges into it.
                        write_fired(&fired_path, &BTreeMap::new())?;
                    }
                    let workspace = store.load_workspace()?;
                    let permission = workspace
                        .pointer("/settings/notifications/localPermission")
                        .and_then(Value::as_str)
                        == Some("granted");
                    let plan = if permission {
                        notification_plan(
                            &workspace,
                            now - Duration::seconds(REMINDER_GRACE_SECONDS),
                            now,
                            None,
                        )?
                    } else {
                        NotificationPlan {
                            from: now.to_rfc3339(),
                            through: now.to_rfc3339(),
                            items: Vec::new(),
                        }
                    };
                    Ok::<_, ReminderError>((true, permission, plan))
                })
                .await
                .map_err(crate::runtime_tasks::RuntimeTaskError::from)??;
                if !initialized || !permission {
                    return Ok(false);
                }
                deliver_items(inner.fired_path.clone(), sink, plan.items, now).await
            })
            .await?
    }

    /// Imports the legacy browser fired-log, or confirms an empty legacy log.
    /// Delivery waits for this for up to a minute after start, then begins
    /// with an empty log; a later import still merges its entries.
    pub async fn import_legacy_fired(
        &self,
        legacy: Value,
        now: DateTime<FixedOffset>,
    ) -> Result<bool, ReminderError> {
        let permit = self
            .0
            .pass
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| crate::runtime_tasks::RuntimeTaskError::Stopped)?;
        let inner = self.0.clone();
        let imported = self
            .0
            .tasks
            .run(async move {
                let _permit = permit;
                let path = inner.fired_path.clone();
                tokio::task::spawn_blocking(move || merge_legacy_fired(&path, legacy, now))
                    .await
                    .map_err(crate::runtime_tasks::RuntimeTaskError::from)?
            })
            .await??;
        let mut next = self
            .0
            .next_tick
            .lock()
            .map_err(|_| ReminderError::Storage("reminder clock is unavailable".into()))?;
        *next = tokio::time::Instant::now();
        Ok(imported)
    }

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }
}

pub(super) async fn deliver_items(
    path: PathBuf,
    sink: Arc<dyn NotificationSink>,
    items: Vec<NotificationItem>,
    now: DateTime<FixedOffset>,
) -> Result<bool, ReminderError> {
    let mut delivered = false;
    let mut first_error = None;
    for item in items {
        let key = item.key.clone();
        let claim_path = path.clone();
        let claim_key = key.clone();
        let claimed =
            tokio::task::spawn_blocking(move || claim_fired(&claim_path, &claim_key, now))
                .await
                .map_err(crate::runtime_tasks::RuntimeTaskError::from)??;
        if !claimed {
            continue;
        }
        delivered = true;
        if let Err(error) = sink.deliver(item).await {
            let rollback_path = path.clone();
            let rollback_key = key.clone();
            tokio::task::spawn_blocking(move || unclaim_fired(&rollback_path, &rollback_key))
                .await
                .map_err(crate::runtime_tasks::RuntimeTaskError::from)??;
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Some(error) = first_error {
        Err(ReminderError::Delivery(error))
    } else {
        Ok(delivered)
    }
}

pub(super) fn read_fired(path: &Path) -> Result<BTreeMap<String, i64>, ReminderError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(ReminderError::Storage(error.to_string())),
    }
}

fn write_fired(path: &Path, fired: &BTreeMap<String, i64>) -> Result<(), ReminderError> {
    let bytes =
        serde_json::to_vec(fired).map_err(|error| ReminderError::Storage(error.to_string()))?;
    crate::notes::atomic_write(path, &bytes)
        .map_err(|error| ReminderError::Storage(error.to_string()))
}

fn merge_legacy_fired(
    path: &Path,
    legacy: Value,
    now: DateTime<FixedOffset>,
) -> Result<bool, ReminderError> {
    let empty = serde_json::Map::new();
    let object = if legacy.is_null() {
        &empty
    } else {
        legacy.as_object().ok_or_else(|| {
            ReminderError::InvalidInput("legacy reminder history must be an object".into())
        })?
    };
    let existed = path.is_file();
    let mut fired = read_fired(path)?;
    for (key, value) in object {
        let Some(timestamp) = value.as_i64() else {
            continue;
        };
        fired
            .entry(key.clone())
            .and_modify(|stored| *stored = (*stored).max(timestamp))
            .or_insert(timestamp);
    }
    let oldest = (now - Duration::seconds(FIRED_TTL_SECONDS)).timestamp_millis();
    fired.retain(|_, fired_at| *fired_at >= oldest);
    write_fired(path, &fired)?;
    Ok(!existed)
}

fn unclaim_fired(path: &Path, key: &str) -> Result<(), ReminderError> {
    let mut fired = read_fired(path)?;
    fired.remove(key);
    write_fired(path, &fired)
}

pub(super) fn fired_path(database: &Path) -> PathBuf {
    let name = database
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("workspace.db");
    database.with_file_name(format!("{name}.reminders-fired.json"))
}

pub(super) fn claim_fired(
    path: &Path,
    key: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, ReminderError> {
    let mut fired = read_fired(path)?;
    if fired.contains_key(key) {
        return Ok(false);
    }
    let now_millis = now.timestamp_millis();
    fired.insert(key.to_owned(), now_millis);
    let oldest = (now - Duration::seconds(FIRED_TTL_SECONDS)).timestamp_millis();
    fired.retain(|_, fired_at| *fired_at >= oldest);
    write_fired(path, &fired)?;
    Ok(true)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum NotificationLocale {
    Ja,
    En,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum NotificationPriority {
    Class,
    Exam,
    Deadline,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NotificationItem {
    pub key: String,
    pub at: String,
    pub title: String,
    pub body: String,
    pub deep_link: String,
    pub priority: NotificationPriority,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct NotificationPlan {
    pub from: String,
    pub through: String,
    pub items: Vec<NotificationItem>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GlanceSnapshot {
    pub term: String,
    pub date: String,
    pub agenda: Vec<Occurrence>,
    pub next: Option<Occurrence>,
    pub remaining: Vec<Occurrence>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub deadlines: Vec<Value>,
}

pub fn glance(
    workspace: &Value,
    now: DateTime<FixedOffset>,
) -> Result<GlanceSnapshot, ScheduleError> {
    let term = reminder_term(workspace);
    let date = tokyo_date(now);
    let agenda = resolve_day_agenda(
        workspace,
        &term,
        &date,
        AgendaOptions {
            include_personal: true,
            ..AgendaOptions::default()
        },
    )?;
    let current = now
        .with_timezone(
            &FixedOffset::east_opt(9 * 3600)
                .ok_or_else(|| ScheduleError::Compatibility("JST offset unavailable".into()))?,
        )
        .format("%H:%M")
        .to_string();
    let remaining: Vec<_> = agenda
        .iter()
        .filter(|row| row.start_time >= current)
        .cloned()
        .collect();
    let next = remaining.first().cloned();
    let mut deadlines: Vec<_> = array(workspace, "assignments")
        .iter()
        .filter(|row| {
            open_assignment(row)
                && text(row, "dueAt")
                    .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
                    .is_some_and(|due| due >= now)
        })
        .cloned()
        .collect();
    deadlines.sort_by(|a, b| text(a, "dueAt").cmp(&text(b, "dueAt")));
    Ok(GlanceSnapshot {
        term,
        date,
        agenda,
        next,
        remaining,
        deadlines,
    })
}

pub(crate) fn assignment_lead_minutes(settings: &Value) -> Result<Vec<i64>, ScheduleError> {
    let Some(value) = settings
        .get("assignmentLeadMinutes")
        .filter(|value| !value.is_null())
    else {
        return Ok(DEFAULT_ASSIGNMENT_LEADS.to_vec());
    };
    let invalid = || {
        ScheduleError::InvalidInput("Choose 1 to 8 distinct assignment reminder times, from 1 minute to 7 days before the deadline.".into())
    };
    let values = value
        .as_array()
        .filter(|values| !values.is_empty() && values.len() <= 8)
        .ok_or_else(invalid)?;
    let mut leads = Vec::new();
    for value in values {
        let lead = value
            .as_i64()
            .filter(|lead| (1..=10080).contains(lead))
            .ok_or_else(invalid)?;
        if leads.contains(&lead) {
            return Err(invalid());
        }
        leads.push(lead);
    }
    Ok(leads)
}

pub fn notification_plan(
    workspace: &Value,
    now: DateTime<FixedOffset>,
    horizon: DateTime<FixedOffset>,
    locale: Option<NotificationLocale>,
) -> Result<NotificationPlan, ScheduleError> {
    if horizon < now {
        return Err(ScheduleError::InvalidInput(
            "notification horizon must not precede now".into(),
        ));
    }
    let start_date = super::parse_civil_date(&tokyo_date(now))?;
    let through_date = super::parse_civil_date(&tokyo_date(horizon))?;
    let days = (through_date - start_date).num_days();
    if days > 366 {
        return Err(ScheduleError::InvalidInput(
            "notification horizon is too long".into(),
        ));
    }
    let configured_locale = content_language(workspace);
    let locale = match locale {
        Some(NotificationLocale::Ja) => "ja",
        Some(NotificationLocale::En) => "en",
        None => configured_locale,
    };
    let japanese = locale == "ja";
    let settings = workspace
        .pointer("/settings/notifications")
        .unwrap_or(&Value::Null);
    let assignment_leads = assignment_lead_minutes(settings)?;
    let mut items = Vec::new();
    if settings
        .get("classes")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let lead = settings
            .get("classLeadMinutes")
            .and_then(Value::as_i64)
            .unwrap_or(10);
        let term = reminder_term(workspace);
        let courses = scheduled_courses(workspace, &term);
        for offset in 0..=days + 1 {
            let date = (start_date + Duration::days(offset)).to_string();
            for block in resolve_day_agenda(
                workspace,
                &term,
                &date,
                AgendaOptions {
                    include_personal: false,
                    courses: Some(courses.clone()),
                    events: Some(vec![]),
                    ..AgendaOptions::default()
                },
            )? {
                if block.cancelled == Some(true)
                    || !matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam)
                {
                    continue;
                }
                let start = DateTime::parse_from_rfc3339(&format!(
                    "{}T{}:00+09:00",
                    block.date, block.start_time
                ))
                .map_err(|_| {
                    ScheduleError::Compatibility("schedule contains an invalid start time".into())
                })?;
                let at = start - Duration::minutes(lead);
                if at < now || at > horizon {
                    continue;
                }
                let title = if block.kind == OccurrenceKind::Exam {
                    format!(
                        "{} · {}",
                        if japanese { "試験" } else { "Exam" },
                        block.title
                    )
                } else {
                    block.title.clone()
                };
                let body = format!(
                    "{} · {}{}",
                    if japanese {
                        format!("{}分後に開始", lead)
                    } else {
                        format!("Starts in {lead} min")
                    },
                    block.start_time,
                    if block.room.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", block.room)
                    }
                );
                items.push(NotificationItem {
                    key: format!("class:{}", block.key),
                    at: at.to_rfc3339(),
                    title,
                    body,
                    deep_link: "/today".into(),
                    priority: if block.kind == OccurrenceKind::Exam {
                        NotificationPriority::Exam
                    } else {
                        NotificationPriority::Class
                    },
                });
            }
        }
    }
    if settings
        .get("assignments")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        for assignment in array(workspace, "assignments")
            .iter()
            .filter(|row| open_assignment(row))
        {
            let Some(due) =
                text(assignment, "dueAt").and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            else {
                continue;
            };
            let course_title = text(assignment, "courseId")
                .and_then(|id| {
                    array(workspace, "courses")
                        .iter()
                        .find(|c| text(c, "id") == Some(id))
                })
                .map(|course| {
                    localized(
                        course.get("names").and_then(|n| n.get("title")),
                        locale,
                        text(course, "title").unwrap_or_default(),
                    )
                })
                .unwrap_or_default();
            for &lead in &assignment_leads {
                let at = due - Duration::minutes(lead);
                if at < now || at > horizon {
                    continue;
                }
                let lead_text = if japanese {
                    if lead % 60 == 0 {
                        format!("締切まで{}時間", lead / 60)
                    } else {
                        format!("締切まで{lead}分")
                    }
                } else if lead % 60 == 0 {
                    let hours = lead / 60;
                    format!(
                        "Due in {hours} {}",
                        if hours == 1 { "hour" } else { "hours" }
                    )
                } else {
                    format!(
                        "Due in {lead} {}",
                        if lead == 1 { "minute" } else { "minutes" }
                    )
                };
                items.push(NotificationItem {
                    key: format!(
                        "assignment:{}:{lead}",
                        text(assignment, "id").unwrap_or_default()
                    ),
                    at: at.to_rfc3339(),
                    title: text(assignment, "title").unwrap_or_default().into(),
                    body: if course_title.is_empty() {
                        lead_text
                    } else {
                        format!("{lead_text} · {course_title}")
                    },
                    deep_link: format!(
                        "/assignments/{}",
                        text(assignment, "id").unwrap_or_default()
                    ),
                    priority: NotificationPriority::Deadline,
                });
            }
        }
    }
    items.sort_by(|a, b| a.at.cmp(&b.at).then(a.key.cmp(&b.key)));
    Ok(NotificationPlan {
        from: now.to_rfc3339(),
        through: horizon.to_rfc3339(),
        items,
    })
}

/// The term whose timetable Today shows: the current quarter once any course
/// belongs to it (including a 3Q–4Q course added in 3Q), else the selected one.
fn reminder_term(workspace: &Value) -> String {
    let current = text(workspace, "currentTerm").unwrap_or_default();
    if !super::term_courses(workspace, current).is_empty() {
        current.into()
    } else {
        text(workspace, "selectedTerm").unwrap_or(current).into()
    }
}
fn open_assignment(row: &Value) -> bool {
    visible(row)
        && row.get("hidden").and_then(Value::as_bool) != Some(true)
        && !super::assignment_submitted(row)
        && text(row, "dueAt").is_some()
}
