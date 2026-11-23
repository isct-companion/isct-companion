use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
};

use super::SchoolReads;
use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{
    array, resolve_day_agenda, text, tokyo_date, visible, AgendaOptions, Occurrence,
    OccurrenceKind, ScheduleError,
};
use crate::locale::{pick, ContentLanguage};

#[cfg(any(test, feature = "mobile-reminders"))]
mod mobile;
#[cfg(any(test, feature = "mobile-reminders"))]
pub use mobile::{
    PendingReminder, PendingReminderState, ReminderDeliveryPlan, ReminderPort, ReminderPortFacts,
    ReminderReceipt,
};

pub const DEFAULT_ASSIGNMENT_LEADS: [i64; 2] = [1440, 60];

const CLASS_LEAD_PRESETS: [u32; 4] = [5, 10, 15, 30];
const EXAM_LEAD_PRESETS: [u32; 5] = [10, 30, 60, 1440, 2880];
const ASSIGNMENT_LEAD_PRESETS: [u32; 10] = [15, 30, 60, 180, 360, 720, 1440, 2880, 4320, 10080];
const MIN_ASSIGNMENT_LEADS: u32 = 1;
const MAX_ASSIGNMENT_LEADS: u32 = 8;

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ReminderLeadOptions {
    pub classes: Vec<u32>,
    pub exams: Vec<u32>,
    pub assignments: AssignmentLeadOptions,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AssignmentLeadOptions {
    pub presets: Vec<u32>,
    pub min_selected: u32,
    pub max_selected: u32,
}

pub fn reminder_lead_options(settings: &Value) -> Result<ReminderLeadOptions, ScheduleError> {
    let preferences = notification_preferences(settings);
    let mut exams = EXAM_LEAD_PRESETS.to_vec();
    exams.push(preferences.exam_lead_minutes as u32);
    exams.sort_unstable();
    exams.dedup();

    let mut assignments = ASSIGNMENT_LEAD_PRESETS.to_vec();
    assignments.extend(
        assignment_lead_minutes(settings)?
            .into_iter()
            .map(|lead| lead as u32),
    );
    assignments.sort_unstable_by(|a, b| b.cmp(a));
    assignments.dedup();

    Ok(ReminderLeadOptions {
        classes: CLASS_LEAD_PRESETS.to_vec(),
        exams,
        assignments: AssignmentLeadOptions {
            presets: assignments,
            min_selected: MIN_ASSIGNMENT_LEADS,
            max_selected: MAX_ASSIGNMENT_LEADS,
        },
    })
}

const REMINDER_GRACE_SECONDS: i64 = 5 * 60;
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

/// The reminder log's file name in `StorePaths::local`.
pub const REMINDER_LOG: &str = "reminders-fired.json";
/// How far ahead a pass looks for the next reminder; with none in it, the
/// next pass is this far out. A workspace change brings it forward.
const LOOKAHEAD_HOURS: i64 = 36;

struct ReminderInner {
    store: crate::data::DataStore,
    tasks: crate::runtime_tasks::RuntimeTasks,
    pass: Arc<tokio::sync::Semaphore>,
    /// The desktop port posts directly when a reminder is due.
    sink: Mutex<Option<Arc<dyn NotificationSink>>>,
    /// The phone port reconciles Core's plan with its OS scheduler.
    #[cfg(feature = "mobile-reminders")]
    mobile_port: Mutex<Option<Arc<dyn ReminderPort>>>,
    /// When the next pass is due (`PlanEngine` asks); `None` before the
    /// first pass, which is due at once.
    next: Mutex<Option<DateTime<Utc>>>,
    fired_path: PathBuf,
}

impl ReminderService {
    pub fn new(store: crate::data::DataStore) -> Self {
        let fired_path = store.paths().local(REMINDER_LOG);
        Self(Arc::new(ReminderInner {
            store,
            tasks: crate::runtime_tasks::RuntimeTasks::default(),
            pass: Arc::new(tokio::sync::Semaphore::new(1)),
            sink: Mutex::new(None),
            #[cfg(feature = "mobile-reminders")]
            mobile_port: Mutex::new(None),
            next: Mutex::new(None),
            fired_path,
        }))
    }

    /// The shell's notifications, for every later pass.
    pub fn attach(&self, sink: Arc<dyn NotificationSink>) {
        if let Ok(mut slot) = self.0.sink.lock() {
            *slot = Some(sink);
        }
    }

    pub(crate) fn sink(&self) -> Option<Arc<dyn NotificationSink>> {
        self.0.sink.lock().ok().and_then(|sink| sink.clone())
    }

    /// When the next pass is due. `None` without a notification port.
    pub(crate) fn next_due(&self) -> Option<DateTime<Utc>> {
        #[cfg(not(feature = "mobile-reminders"))]
        self.sink()?;
        #[cfg(feature = "mobile-reminders")]
        if self.sink().is_none() && self.mobile_port().is_none() {
            return None;
        }
        Some(
            self.0
                .next
                .lock()
                .ok()
                .and_then(|next| *next)
                .unwrap_or_else(crate::clock::now),
        )
    }

    /// A handle for the store's commit observer, which must not keep the
    /// service (and so the store) alive.
    pub(crate) fn changes(&self) -> ReminderChanges {
        ReminderChanges(Arc::downgrade(&self.0))
    }

    fn plan_next(&self, at: DateTime<Utc>) {
        if let Ok(mut next) = self.0.next.lock() {
            *next = Some(at);
        }
    }

    /// Claims and delivers reminders due within the five-minute grace
    /// window, and notes when the next one is due. Concurrent passes
    /// coalesce into one.
    pub async fn deliver_due(
        &self,
        sink: Arc<dyn NotificationSink>,
        now: DateTime<FixedOffset>,
    ) -> Result<bool, ReminderError> {
        if self.0.tasks.is_closed() {
            return Err(ReminderError::Task(
                crate::runtime_tasks::RuntimeTaskError::Stopped,
            ));
        }
        let Ok(permit) = self.0.pass.clone().try_acquire_owned() else {
            return Ok(false);
        };
        let inner = self.0.clone();
        let service = self.clone();
        self.0
            .tasks
            .run(async move {
                let _permit = permit;
                let store = inner.store.clone();
                let fired_path = inner.fired_path.clone();
                let horizon = now + Duration::hours(LOOKAHEAD_HOURS);
                let plan = tokio::task::spawn_blocking(move || {
                    if !fired_path.is_file() {
                        write_fired(&fired_path, &BTreeMap::new())?;
                    }
                    // Only what the notification plan reads.
                    let fields = crate::workspace_changes::loaded_fields(
                        super::REMINDER_FIELDS.iter().copied(),
                    );
                    let (workspace, school, _) = store.view(&fields)?;
                    let permission = workspace
                        .pointer("/settings/notifications/localPermission")
                        .and_then(Value::as_str)
                        == Some("granted");
                    if !permission {
                        return Ok::<_, ReminderError>(None);
                    }
                    Ok(Some(notification_plan(
                        &workspace,
                        &school,
                        now - Duration::seconds(REMINDER_GRACE_SECONDS),
                        horizon,
                        None,
                    )?))
                })
                .await
                .map_err(crate::runtime_tasks::RuntimeTaskError::from)??;
                let Some(plan) = plan else {
                    // Turning permission on is a workspace change.
                    service.plan_next(horizon.with_timezone(&Utc));
                    return Ok(false);
                };
                let at = |item: &NotificationItem| {
                    DateTime::parse_from_rfc3339(&item.at)
                        .ok()
                        .map(|at| at.with_timezone(&Utc))
                };
                let now_utc = now.with_timezone(&Utc);
                let (due, later): (Vec<_>, Vec<_>) = plan
                    .items
                    .into_iter()
                    .partition(|item| at(item).is_none_or(|at| at <= now_utc));
                let next = later
                    .iter()
                    .filter_map(at)
                    .min()
                    .unwrap_or_else(|| horizon.with_timezone(&Utc));
                service.plan_next(next);
                let delivered = deliver_items(inner.fired_path.clone(), sink, due, now).await;
                if delivered.is_err() {
                    // A reminder the port refused is claimed again soon.
                    service.plan_next(next.min(now_utc + Duration::minutes(1)));
                }
                delivered
            })
            .await?
    }

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
        if let Ok(mut sink) = self.0.sink.lock() {
            *sink = None;
        }
        #[cfg(feature = "mobile-reminders")]
        if let Ok(mut port) = self.0.mobile_port.lock() {
            *port = None;
        }
    }
}

/// [`ReminderService::changed`] through a weak handle.
#[derive(Clone)]
pub(crate) struct ReminderChanges(std::sync::Weak<ReminderInner>);

impl ReminderChanges {
    pub(crate) fn changed(&self) {
        if let Some(inner) = self.0.upgrade() {
            changed(&inner);
        }
    }
}

fn changed(inner: &ReminderInner) {
    let delay = Duration::seconds(5);
    #[cfg(feature = "mobile-reminders")]
    let delay = if inner.mobile_port.lock().is_ok_and(|port| port.is_some()) {
        // Include reconciliation in the OS opportunity that committed the
        // change. The planner can go quiet while a future duty is pending.
        Duration::zero()
    } else {
        delay
    };
    let soon = crate::clock::now() + delay;
    if let Ok(mut next) = inner.next.lock() {
        *next = Some(next.map_or(soon, |next| next.min(soon)));
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
    write_history(path, fired)
}

fn write_history(path: &Path, history: &impl Serialize) -> Result<(), ReminderError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ReminderError::Storage(error.to_string()))?;
    }
    let bytes =
        serde_json::to_vec(history).map_err(|error| ReminderError::Storage(error.to_string()))?;
    crate::storage::durable_replace(path, &bytes)
        .map_err(|error| ReminderError::Storage(error.to_string()))
}

fn unclaim_fired(path: &Path, key: &str) -> Result<(), ReminderError> {
    let mut fired = read_fired(path)?;
    fired.remove(key);
    write_fired(path, &fired)
}

pub(super) fn claim_fired(
    path: &Path,
    key: &str,
    now: DateTime<FixedOffset>,
) -> Result<bool, ReminderError> {
    let mut fired = read_fired(path)?;
    if !claim_fired_entry(&mut fired, key, now) {
        return Ok(false);
    }
    write_fired(path, &fired)?;
    Ok(true)
}

fn claim_fired_entry(
    fired: &mut BTreeMap<String, i64>,
    key: &str,
    now: DateTime<FixedOffset>,
) -> bool {
    if fired.contains_key(key) {
        return false;
    }
    let now_millis = now.timestamp_millis();
    fired.insert(key.to_owned(), now_millis);
    let oldest = (now - Duration::seconds(FIRED_TTL_SECONDS)).timestamp_millis();
    fired.retain(|_, fired_at| *fired_at >= oldest);
    true
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
    school: &SchoolReads,
    now: DateTime<FixedOffset>,
) -> Result<GlanceSnapshot, ScheduleError> {
    let term = reminder_term(workspace);
    let date = tokyo_date(now);
    let agenda = resolve_day_agenda(
        workspace,
        school,
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

/// The reminder switches and times in `settings.notifications`, each with its
/// one default. The workspace projection writes these back, so every shell
/// reads plain values.
pub(crate) struct NotificationPreferences {
    pub classes: bool,
    pub class_lead_minutes: i64,
    /// Older installations delivered exams with class reminders; an unset
    /// exam switch and time follow the class ones.
    pub exams: bool,
    pub exam_lead_minutes: i64,
    /// Quizzes count as exams on Today and in exam reminders.
    pub quizzes: bool,
    pub assignments: bool,
}

pub(crate) fn notification_preferences(notifications: &Value) -> NotificationPreferences {
    let flag = |name: &str| notifications.get(name).and_then(Value::as_bool);
    let minutes = |name: &str| notifications.get(name).and_then(Value::as_i64);
    let classes = flag("classes").unwrap_or(false);
    let class_lead_minutes = minutes("classLeadMinutes").unwrap_or(10);
    NotificationPreferences {
        classes,
        class_lead_minutes,
        exams: flag("exams").unwrap_or(classes),
        exam_lead_minutes: minutes("examLeadMinutes").unwrap_or(class_lead_minutes),
        quizzes: flag("quizzes").unwrap_or(true),
        assignments: flag("assignments").unwrap_or(false),
    }
}

pub(crate) fn assignment_lead_minutes(settings: &Value) -> Result<Vec<i64>, ScheduleError> {
    let Some(value) = settings
        .get("assignmentLeadMinutes")
        .filter(|value| !value.is_null())
    else {
        return Ok(DEFAULT_ASSIGNMENT_LEADS.to_vec());
    };
    let invalid = || {
        ScheduleError::InvalidInput(format!("Choose {MIN_ASSIGNMENT_LEADS} to {MAX_ASSIGNMENT_LEADS} distinct assignment reminder times, from 1 minute to 7 days before the deadline."))
    };
    let values = value
        .as_array()
        .filter(|values| {
            (MIN_ASSIGNMENT_LEADS as usize..=MAX_ASSIGNMENT_LEADS as usize).contains(&values.len())
        })
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
    school: &SchoolReads,
    now: DateTime<FixedOffset>,
    horizon: DateTime<FixedOffset>,
    locale: Option<NotificationLocale>,
) -> Result<NotificationPlan, ScheduleError> {
    notification_plan_with_tracked(workspace, school, now, horizon, locale, &BTreeSet::new())
}

/// Include the current business reminder for each tracked key even outside
/// the registration window. An OS request can still be pending past grace,
/// and a delivered reminder can move beyond the current scheduling horizon.
fn notification_plan_with_tracked(
    workspace: &Value,
    school: &SchoolReads,
    now: DateTime<FixedOffset>,
    horizon: DateTime<FixedOffset>,
    locale: Option<NotificationLocale>,
    tracked: &BTreeSet<&str>,
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
    let language = match locale {
        Some(NotificationLocale::Ja) => ContentLanguage::Ja,
        Some(NotificationLocale::En) => ContentLanguage::En,
        None => ContentLanguage::of(workspace),
    };
    let japanese = language == ContentLanguage::Ja;
    let settings = workspace
        .pointer("/settings/notifications")
        .unwrap_or(&Value::Null);
    let assignment_leads = assignment_lead_minutes(settings)?;
    let mut items = Vec::new();
    let preferences = notification_preferences(settings);
    let (classes, exams) = (preferences.classes, preferences.exams);
    let (class_lead, exam_lead) = (
        preferences.class_lead_minutes,
        preferences.exam_lead_minutes,
    );
    if !(1..=10080).contains(&exam_lead) {
        return Err(ScheduleError::InvalidInput(
            "Exam reminder time must be between 1 minute and 7 days.".into(),
        ));
    }
    let quizzes = preferences.quizzes;
    if classes || exams {
        let class_keys: BTreeSet<_> = tracked
            .iter()
            .filter_map(|key| key.strip_prefix("class:"))
            .collect();
        let mut dates: BTreeSet<_> = (0..=days + (class_lead.max(exam_lead) + 1439) / 1440 + 1)
            .map(|offset| (start_date + Duration::days(offset)).to_string())
            .collect();
        dates.extend(
            class_keys
                .iter()
                .filter_map(|key| super::occurrence::key_date(key))
                .map(str::to_owned),
        );
        if !class_keys.is_empty() {
            dates.extend(
                array(workspace, "scheduleOverrides")
                    .iter()
                    .filter(|row| visible(row))
                    .filter_map(|row| {
                        let key = text(row, "targetKey")?;
                        if class_keys.contains(key) {
                            text(row, "date").map(str::to_owned)
                        } else {
                            None
                        }
                    }),
            );
        }
        let term = reminder_term(workspace);
        let mut terms = std::collections::BTreeSet::from([term.clone()]);
        if exams {
            terms.extend(super::exams::exam_terms(workspace, school));
        }
        for source_term in terms {
            let sessions = super::occurrence::CourseSessions::new(workspace, &source_term, None)?;
            let published =
                super::exams::published_sessions_by_date(workspace, school, &source_term);
            let published_dates = published.iter().filter_map(|(date, exams)| {
                exams.iter()
                    .any(|exam| class_keys.contains(exam.key.as_str()))
                    .then_some(date)
            });
            let source_dates: BTreeSet<_> = dates.iter().chain(published_dates).collect();
            for date in source_dates {
                for block in sessions.agenda_on(
                    date,
                    published.get(date).map(Vec::as_slice).unwrap_or_default(),
                    AgendaOptions {
                        include_personal: false,
                        ..AgendaOptions::default()
                    },
                )? {
                    if block.cancelled == Some(true)
                        || !matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam)
                    {
                        continue;
                    }
                    // With quizzes switched off, a quiz reminds like its class.
                    let quiz = block.quiz == Some(true);
                    let is_exam = block.kind == OccurrenceKind::Exam && (!quiz || quizzes);
                    if (is_exam && !exams) || (!is_exam && (!classes || source_term != term)) {
                        continue;
                    }
                    let lead = if is_exam { exam_lead } else { class_lead };
                    let start = DateTime::parse_from_rfc3339(&format!(
                        "{}T{}:00+09:00",
                        block.date, block.start_time
                    ))
                    .map_err(|_| {
                        ScheduleError::Compatibility(
                            "schedule contains an invalid start time".into(),
                        )
                    })?;
                    let at = start - Duration::minutes(lead);
                    let key = format!("class:{}", block.key);
                    if !(now..=horizon).contains(&at) && !tracked.contains(key.as_str()) {
                        continue;
                    }
                    let title = if block.kind == OccurrenceKind::Exam {
                        let label = match (quiz, japanese) {
                            (true, true) => "小テスト",
                            (true, false) => "Quiz",
                            (false, true) => "試験",
                            (false, false) => "Exam",
                        };
                        format!("{label} · {}", block.title)
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
                        key,
                        at: at.to_rfc3339(),
                        title,
                        body,
                        deep_link: "/today".into(),
                        priority: if is_exam {
                            NotificationPriority::Exam
                        } else {
                            NotificationPriority::Class
                        },
                    });
                }
            }
        }
    }
    if preferences.assignments {
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
                .and_then(|course| {
                    pick(course.pointer("/names/title"), language).or_else(|| text(course, "title"))
                })
                .unwrap_or_default()
                .to_owned();
            for &lead in &assignment_leads {
                let at = due - Duration::minutes(lead);
                let key = format!(
                    "assignment:{}:{lead}",
                    text(assignment, "id").unwrap_or_default()
                );
                if !(now..=horizon).contains(&at) && !tracked.contains(key.as_str()) {
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
                    key,
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
pub(super) fn reminder_term(workspace: &Value) -> String {
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
