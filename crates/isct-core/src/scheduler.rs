//! Scheduled domain work. Shells provide wakeups; Core decides what is due.

use crate::{
    data::{DataError, DataStore},
    events::{CoreEvent, WorkspaceChangeSource},
    lifecycle::SchoolSite,
    ports::EventSink,
    pull_coordinator::PullOptions,
    reference_refresh::{ReferenceError, ReferenceService},
    school_refresh::SchoolRefresh,
    school_service::{IngestOutcome, SchoolError, SchoolExecutor, SchoolService},
};
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
    pub references: ReferenceService,
    pub school: SchoolService,
    pub refresh: SchoolRefresh,
    pub executor: Arc<dyn SchoolExecutor>,
    pub exporter: Arc<dyn ScheduledExporter>,
    pub events: Arc<dyn EventSink>,
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("{0}")]
    InvalidTask(String),
    #[error(transparent)]
    Storage(#[from] DataError),
    #[error(transparent)]
    Reference(#[from] ReferenceError),
    #[error(transparent)]
    School(#[from] SchoolError),
    #[error(transparent)]
    Export(#[from] ExportError),
}

/// Built-in scheduled task actions (`scheduled_tasks.action`). Unknown stored
/// values fail the run with a typed message instead of being matched as text.
#[derive(Clone, Copy, Debug, serde::Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ScheduledAction {
    Export,
    ReferenceRefresh,
    LmsRefresh,
    SchoolRefresh,
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
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            started_at: Utc::now(),
            running: tokio::sync::Mutex::new(()),
            invalid_schedules: Mutex::new(HashSet::new()),
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

fn due_occurrence(
    task: &Value,
    now: DateTime<Utc>,
    started_at: DateTime<Utc>,
) -> Result<Option<String>, SchedulerError> {
    if task.get("enabled").and_then(Value::as_bool) != Some(true) || deleted(task) {
        return Ok(None);
    }
    let schedule = task
        .get("schedule")
        .and_then(Value::as_str)
        .ok_or_else(|| SchedulerError::InvalidTask("scheduled task has no schedule".into()))?;
    let last = timestamp(task.get("lastOccurrenceAt").and_then(Value::as_str));
    if crate::reference_refresh::reference_due_at_launch(task, started_at) {
        return Ok(Some(started_at.to_rfc3339()));
    }
    if schedule == "launch" {
        return Ok(last
            .is_none_or(|last| last < started_at)
            .then(|| started_at.to_rfc3339()));
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
    let dates = set
        .after(floor)
        .before(now.with_timezone(&Tz::UTC))
        .all(1)
        .dates;
    Ok(dates.first().map(DateTime::to_rfc3339))
}

fn task_result_problem(outcomes: &[IngestOutcome]) -> Result<(), SchedulerError> {
    if let Some(problem) = outcomes.iter().find(|outcome| outcome.state.is_problem()) {
        return Err(SchedulerError::School(SchoolError::Executor(
            problem.message.clone(),
        )));
    }
    Ok(())
}

fn emit(services: &SchedulerServices, source: WorkspaceChangeSource) {
    // Events are hints with snapshot recovery. A failed delivery must not leave
    // a claimed task permanently displaying `Running`.
    let _ = services.events.emit(CoreEvent::WorkspaceChanged {
        source,
        revision: None,
    });
}

async fn pull_site(
    services: &SchedulerServices,
    site: SchoolSite,
    force: bool,
) -> Result<(), SchedulerError> {
    if !force {
        services.refresh.enqueue_scheduled(site)?;
        return Ok(());
    }
    let outcomes = services
        .school
        .pull(
            services.executor.clone(),
            site,
            None,
            PullOptions::manual(),
            true,
        )
        .await?;
    task_result_problem(&outcomes)
}

async fn execute(
    services: &SchedulerServices,
    action: Option<ScheduledAction>,
    force: bool,
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
        Some(ScheduledAction::ReferenceRefresh) => {
            Ok(services.references.refresh_all(force).await?)
        }
        Some(ScheduledAction::LmsRefresh) => {
            if !force && !services.refresh.foreground() {
                return Ok("Paused: app is not active".into());
            }
            pull_site(services, SchoolSite::ScienceLms, force).await?;
            Ok("LMS background refresh completed".into())
        }
        Some(ScheduledAction::SchoolRefresh) => {
            for site in [SchoolSite::ScienceLms, SchoolSite::Kyomu] {
                pull_site(services, site, force).await?;
            }
            Ok("School refresh completed".into())
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
    pub async fn tick(&self, services: &SchedulerServices) -> Result<(), SchedulerError> {
        let tasks = services.store.user_rows("scheduled_tasks")?;
        let now = Utc::now();
        for task in tasks {
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
            "scheduler",
            Some(expected),
        ) {
            invalid.remove(&key);
            return Err(error.into());
        }
        drop(invalid);
        emit(services, WorkspaceChangeSource::Scheduler);
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
            "scheduler",
            Some(expected),
        )?;
        emit(services, WorkspaceChangeSource::Scheduler);

        let message = execute(services, ScheduledAction::of(&task), force)
            .await
            .unwrap_or_else(|error| format!("Failed: {error}"));
        let expected = task.get("rev").and_then(Value::as_u64).unwrap_or(0);
        task["lastRunAt"] = Value::String(Utc::now().to_rfc3339());
        task["lastResult"] = Value::String(message.clone());
        services.store.put_user_record_checked(
            "scheduled_tasks",
            task,
            "scheduler",
            Some(expected),
        )?;
        emit(services, WorkspaceChangeSource::Scheduler);
        Ok(message)
    }
}

#[cfg(test)]
#[path = "tests/scheduler.rs"]
mod tests;
