//! The person's own scheduled tasks (exports). Core decides what is due and
//! tells the `PlanEngine` when the next occurrence comes ([`Scheduler::next_due`]).
//! School and reference data are not scheduled here: each dataset's TTL in
//! `crate::plan` decides when it is read.

use crate::data::{DataError, DataStore, WorkspaceChangeSource};
use chrono::{DateTime, Duration, Utc};
use rrule::{RRuleSet, Tz};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ExportError {
    #[error("{0}")]
    Failed(String),
}

pub trait ScheduledExporter: Send + Sync {
    fn export(&self) -> Result<(), ExportError>;
}

#[derive(Clone)]
pub struct SchedulerServices {
    pub store: DataStore,
    pub exporter: Arc<dyn ScheduledExporter>,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("{0}")]
    InvalidTask(String),
    #[error(transparent)]
    Storage(#[from] DataError),
    #[error(transparent)]
    Export(#[from] ExportError),
}

/// Scheduled task actions (`scheduled_tasks.action`). Unknown stored values
/// fail a manual run with a typed message instead of being matched as text;
/// automatic runs skip them, so a stored task of a retired built-in action
/// (`reference-refresh`, `lms-refresh`, `school-refresh`) never fires.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduledAction {
    Export,
    Hook,
}

impl ScheduledAction {
    fn of(task: &Value) -> Option<Self> {
        task.get("action")
            .cloned()
            .and_then(|action| serde_json::from_value(action).ok())
    }
}

pub struct Scheduler {
    started_at: DateTime<Utc>,
    running: tokio::sync::Mutex<()>,
    invalid_schedules: Mutex<HashSet<String>>,
    /// The next occurrence of any task, as the last pass found it; `None`
    /// until a pass ran or after the tasks changed: due at once.
    next: Mutex<Option<Option<DateTime<Utc>>>>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            started_at: Utc::now(),
            running: tokio::sync::Mutex::new(()),
            invalid_schedules: Mutex::new(HashSet::new()),
            next: Mutex::new(None),
        }
    }
}

fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn deleted(task: &Value) -> bool {
    task.get("deletedAt").is_some_and(|value| !value.is_null())
}

fn reject_subhour_schedule(schedule: &str) -> Result<(), SchedulerError> {
    let normalized = schedule.trim().to_ascii_uppercase();
    let body = normalized.strip_prefix("RRULE:").unwrap_or(&normalized);
    let mut frequency = None;
    for field in body.split(';') {
        let Some((name, value)) = field.split_once('=') else {
            continue;
        };
        match name {
            "FREQ" => frequency = Some(value),
            "BYSECOND" => {
                return Err(SchedulerError::InvalidTask(
                    "scheduled tasks support hourly, daily, or weekly recurrence".into(),
                ));
            }
            // Multiple minutes in one hour are another sub-hour recurrence.
            "BYMINUTE" if value.contains(',') => {
                return Err(SchedulerError::InvalidTask(
                    "scheduled tasks support at most one occurrence per hour".into(),
                ));
            }
            _ => {}
        }
    }
    if frequency.is_some_and(|value| matches!(value, "MINUTELY" | "SECONDLY")) {
        return Err(SchedulerError::InvalidTask(
            "scheduled tasks support hourly, daily, or weekly recurrence".into(),
        ));
    }
    Ok(())
}

/// The task's next occurrence at or after its floor (the last one's next
/// second, never before this run started): `None` for a task that never
/// runs again on its own.
fn next_occurrence(
    task: &Value,
    started_at: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, SchedulerError> {
    if task.get("enabled").and_then(Value::as_bool) != Some(true) || deleted(task) {
        return Ok(None);
    }
    let schedule = task
        .get("schedule")
        .and_then(Value::as_str)
        .ok_or_else(|| SchedulerError::InvalidTask("scheduled task has no schedule".into()))?;
    let last = timestamp(task.get("lastOccurrenceAt").and_then(Value::as_str));
    if schedule == "launch" {
        return Ok(last.is_none_or(|last| last < started_at).then_some(started_at));
    }
    reject_subhour_schedule(schedule)?;
    let updated = timestamp(task.get("updatedAt").and_then(Value::as_str)).ok_or_else(|| {
        SchedulerError::InvalidTask("scheduled task has an invalid updatedAt".into())
    })?;
    let calendar = format!(
        "DTSTART:{}\n{}",
        updated.format("%Y%m%dT%H%M%SZ"),
        if schedule.starts_with("RRULE:") {
            schedule.to_string()
        } else {
            format!("RRULE:{schedule}")
        }
    );
    let set: RRuleSet = calendar.parse().map_err(|error| {
        SchedulerError::InvalidTask(format!("invalid scheduled task RRULE: {error}"))
    })?;
    let floor = last
        .map(|value| value + Duration::seconds(1))
        .unwrap_or(started_at)
        .max(started_at)
        .with_timezone(&Tz::UTC);
    Ok(set
        .after(floor)
        .all(1)
        .dates
        .first()
        .map(|date| date.with_timezone(&Utc)))
}

/// The occurrence due now, if the next one has come.
fn due_occurrence(
    task: &Value,
    now: DateTime<Utc>,
    started_at: DateTime<Utc>,
) -> Result<Option<String>, SchedulerError> {
    Ok(next_occurrence(task, started_at)?
        .filter(|at| *at <= now)
        .map(|at| at.to_rfc3339()))
}

async fn execute(
    services: &SchedulerServices,
    action: Option<ScheduledAction>,
) -> Result<String, SchedulerError> {
    match action {
        Some(ScheduledAction::Export) => {
            // The exporter writes a file; keep it off the async executor.
            let exporter = services.exporter.clone();
            tokio::task::spawn_blocking(move || exporter.export())
                .await
                .map_err(|_| {
                    ExportError::Failed("Scheduled export stopped unexpectedly.".into())
                })??;
            Ok("Export completed".into())
        }
        Some(ScheduledAction::Hook) => Err(SchedulerError::InvalidTask(
            "Scheduled custom hooks must select a built-in action.".into(),
        )),
        None => Err(SchedulerError::InvalidTask(
            "Scheduled task action is unsupported.".into(),
        )),
    }
}

impl Scheduler {
    /// When a pass is next due (`PlanEngine` asks): at once before the
    /// first pass and after the tasks changed, else the next occurrence.
    pub(crate) fn next_due(&self) -> Option<DateTime<Utc>> {
        match self.next.lock().ok().and_then(|next| *next) {
            None => Some(Utc::now()),
            Some(next) => next,
        }
    }

    /// The person's tasks changed: plan again at once.
    pub(crate) fn changed(&self) {
        if let Ok(mut next) = self.next.lock() {
            *next = None;
        }
    }

    /// Runs every task whose occurrence has come, then notes the next one.
    pub async fn run_due(&self, services: &SchedulerServices) -> Result<(), SchedulerError> {
        let ran = self.pass(services).await;
        let next = services
            .store
            .user_rows("scheduled_tasks")
            .map(|tasks| {
                tasks
                    .iter()
                    .filter(|task| ScheduledAction::of(task).is_some())
                    .filter_map(|task| next_occurrence(task, self.started_at).ok().flatten())
                    .min()
            })
            .unwrap_or(None);
        if let Ok(mut slot) = self.next.lock() {
            // A failed read tries again in a minute.
            *slot = Some(if ran.is_ok() {
                next
            } else {
                Some(Utc::now() + Duration::minutes(1))
            });
        }
        ran
    }

    async fn pass(&self, services: &SchedulerServices) -> Result<(), SchedulerError> {
        let tasks = services.store.user_rows("scheduled_tasks")?;
        let now = Utc::now();
        // A task of a retired built-in action never fires on its own.
        for task in tasks
            .into_iter()
            .filter(|task| ScheduledAction::of(task).is_some())
        {
            match due_occurrence(&task, now, self.started_at) {
                Ok(Some(occurrence)) => {
                    let id = task
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| SchedulerError::InvalidTask("task has no id".into()))?;
                    if let Err(error) = self.run(services, id, Some(occurrence), false).await {
                        crate::diagnostics::report("scheduler", &error);
                    }
                }
                Ok(None) => {}
                Err(error) => self.record_invalid_schedule(services, task, now, error)?,
            }
        }

        Ok(())
    }

    fn record_invalid_schedule(
        &self,
        services: &SchedulerServices,
        mut task: Value,
        now: DateTime<Utc>,
        error: SchedulerError,
    ) -> Result<(), SchedulerError> {
        let id = task
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| SchedulerError::InvalidTask("task has no id".into()))?;
        let schedule = task
            .get("schedule")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let key = format!("{id}:{schedule}");
        let mut invalid = match self.invalid_schedules.lock() {
            Ok(invalid) => invalid,
            Err(poisoned) => {
                self.invalid_schedules.clear_poison();
                poisoned.into_inner()
            }
        };
        if !invalid.insert(key.clone()) {
            return Ok(());
        }
        let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
        let occurrence = now.to_rfc3339();
        task["lastOccurrenceAt"] = Value::String(occurrence.clone());
        task["lastRunAt"] = Value::String(occurrence);
        task["lastResult"] = Value::String(format!("Failed: {error}"));
        if let Err(error) = services.store.put_user_record_checked(
            "scheduled_tasks",
            task,
            WorkspaceChangeSource::Scheduler,
            Some(expected),
        ) {
            invalid.remove(&key);
            return Err(error.into());
        }
        drop(invalid);
        Ok(())
    }

    pub async fn run_now(
        &self,
        services: &SchedulerServices,
        id: &str,
    ) -> Result<String, SchedulerError> {
        self.run(services, id, Some(Utc::now().to_rfc3339()), true)
            .await
    }

    async fn run(
        &self,
        services: &SchedulerServices,
        id: &str,
        occurrence: Option<String>,
        force: bool,
    ) -> Result<String, SchedulerError> {
        let _guard = if force {
            self.running.lock().await
        } else {
            match self.running.try_lock() {
                Ok(guard) => guard,
                Err(_) => return Ok("Deferred: scheduler queue is busy".into()),
            }
        };

        let mut task = services
            .store
            .user_rows("scheduled_tasks")?
            .into_iter()
            .find(|task| task.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| {
                SchedulerError::InvalidTask("Scheduled task no longer exists.".into())
            })?;
        if deleted(&task) {
            return Err(SchedulerError::InvalidTask(
                "Scheduled task no longer exists.".into(),
            ));
        }

        let occurrence = if force {
            occurrence.unwrap_or_else(|| Utc::now().to_rfc3339())
        } else {
            // The tick snapshot may be stale by the time it reaches the queue.
            // Re-read and re-evaluate immediately before the durable claim.
            let Some(fresh) = due_occurrence(&task, Utc::now(), self.started_at)? else {
                return Ok("Skipped: scheduled task is no longer due".into());
            };
            fresh
        };
        let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
        task["lastOccurrenceAt"] = Value::String(occurrence);
        task["lastRunAt"] = Value::String(Utc::now().to_rfc3339());
        task["lastResult"] = Value::String("Running".into());
        let mut task = services.store.put_user_record_checked(
            "scheduled_tasks",
            task,
            WorkspaceChangeSource::Scheduler,
            Some(expected),
        )?;

        let message = execute(services, ScheduledAction::of(&task))
            .await
            .unwrap_or_else(|error| format!("Failed: {error}"));
        let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
        task["lastRunAt"] = Value::String(Utc::now().to_rfc3339());
        task["lastResult"] = Value::String(message.clone());
        services.store.put_user_record_checked(
            "scheduled_tasks",
            task,
            WorkspaceChangeSource::Scheduler,
            Some(expected),
        )?;
        Ok(message)
    }
}

#[cfg(test)]
#[path = "tests/scheduler.rs"]
mod tests;
