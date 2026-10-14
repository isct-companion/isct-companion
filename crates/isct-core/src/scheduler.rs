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

pub struct Scheduler {
    started_at: DateTime<Utc>,
    running: tokio::sync::Mutex<()>,
    invalid_schedules: Mutex<HashSet<String>>,
    seen_agent_write: Mutex<Option<String>>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            started_at: Utc::now(),
            running: tokio::sync::Mutex::new(()),
            invalid_schedules: Mutex::new(HashSet::new()),
            seen_agent_write: Mutex::new(None),
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
    if let Some(problem) = outcomes.iter().find(|outcome| outcome.state != "ok") {
        return Err(SchedulerError::School(SchoolError::Executor(
            problem.message.clone(),
        )));
    }
    Ok(())
}

fn emit(services: &SchedulerServices, source: WorkspaceChangeSource) {
    // Events are hints with snapshot recovery. A failed delivery must not leave
    // a claimed task permanently displaying `Running`.
    let _ = services.events.emit(CoreEvent::WorkspaceChanged { source });
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
    action: &str,
    force: bool,
) -> Result<String, SchedulerError> {
    match action {
        "export" => services
            .exporter
            .export()
            .map(|()| "Export completed".into())
            .map_err(SchedulerError::from),
        "reference-refresh" => Ok(services.references.refresh_all(force, false).await?),
        "lms-refresh" => {
            if !force && !services.refresh.foreground() {
                return Ok("Paused: app is not active".into());
            }
            pull_site(services, SchoolSite::ScienceLms, force).await?;
            Ok("LMS background refresh completed".into())
        }
        "school-refresh" => {
            for site in [SchoolSite::ScienceLms, SchoolSite::Kyomu] {
                pull_site(services, site, force).await?;
            }
            Ok("School refresh completed".into())
        }
        "hook" => Err(SchedulerError::InvalidTask(
            "Scheduled custom hooks must select a built-in action.".into(),
        )),
        _ => Err(SchedulerError::InvalidTask(
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
                    let _ = self.run(services, id, Some(occurrence), false).await;
                }
                Ok(None) => {}
                Err(error) => self.record_invalid_schedule(services, task, now, error)?,
            }
        }

        // MCP writes happen in another process; their durable audit is the
        // cross-process signal. DataStore stores newest entries first.
        let latest_agent_write = services
            .store
            .load_workspace()?
            .pointer("/settings/agentAccess/lastWrites")
            .and_then(Value::as_array)
            .and_then(|writes| writes.first())
            .and_then(|write| serde_json::to_string(write).ok());
        let mut seen = match self.seen_agent_write.lock() {
            Ok(seen) => seen,
            Err(poisoned) => {
                self.seen_agent_write.clear_poison();
                poisoned.into_inner()
            }
        };
        if latest_agent_write != *seen {
            emit(services, WorkspaceChangeSource::Mcp);
        }
        *seen = latest_agent_write;
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

        let action = task
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let message = execute(services, &action, force)
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
mod tests {
    use super::*;
    use crate::{
        ports::{EventError, SecretError, SecretVault},
        pull_coordinator::{PullContext, PullCoordinator, PullNetwork},
        school_auth::SchoolAuthService,
        school_refresh::RefreshEvent,
        school_service::SchoolExecution,
    };
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Barrier, Mutex as StdMutex,
    };
    use uuid::Uuid;

    #[derive(Default)]
    struct MemoryVault;

    impl SecretVault for MemoryVault {
        fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
            Ok(())
        }

        fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
            Ok(None)
        }

        fn delete(&self, _account: &str) -> Result<(), SecretError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopExecutor;

    impl SchoolExecutor for NoopExecutor {
        fn execute(
            &self,
            request: SchoolExecution,
            _context: PullContext,
        ) -> crate::school_service::SchoolExecutorFuture<'_> {
            Box::pin(async move {
                Ok(vec![IngestOutcome {
                    site: request.site,
                    kind: "test".into(),
                    state: "ok".into(),
                    changed: 0,
                    message: String::new(),
                }])
            })
        }
    }

    #[derive(Default)]
    struct RecordingEvents {
        events: StdMutex<Vec<CoreEvent>>,
        fail: AtomicBool,
    }

    impl EventSink for RecordingEvents {
        fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
            if self.fail.load(Ordering::Relaxed) {
                return Err(EventError::Delivery("event channel closed".into()));
            }
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingExporter(AtomicUsize);

    impl ScheduledExporter for CountingExporter {
        fn export(&self) -> Result<(), ExportError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    struct InspectingExporter {
        store: DataStore,
        id: String,
        observed_claim: AtomicBool,
    }

    impl ScheduledExporter for InspectingExporter {
        fn export(&self) -> Result<(), ExportError> {
            let claimed = self
                .store
                .user_rows("scheduled_tasks")
                .map_err(|error| ExportError::Failed(error.to_string()))?
                .into_iter()
                .find(|task| task["id"].as_str() == Some(self.id.as_str()))
                .is_some_and(|task| task["lastResult"] == "Running");
            self.observed_claim.store(claimed, Ordering::Relaxed);
            Ok(())
        }
    }

    struct FailingExporter;

    impl ScheduledExporter for FailingExporter {
        fn export(&self) -> Result<(), ExportError> {
            Err(ExportError::Failed("export destination unavailable".into()))
        }
    }

    struct BlockingExporter {
        gate: Arc<Barrier>,
    }

    impl ScheduledExporter for BlockingExporter {
        fn export(&self) -> Result<(), ExportError> {
            self.gate.wait();
            self.gate.wait();
            Ok(())
        }
    }

    fn store(name: &str) -> DataStore {
        let path =
            std::env::temp_dir().join(format!("isct-scheduler-{name}-{}.db", Uuid::new_v4()));
        DataStore::initialize(path).unwrap().0
    }

    fn services(
        store: &DataStore,
        exporter: Arc<dyn ScheduledExporter>,
        events: Arc<dyn EventSink>,
    ) -> SchedulerServices {
        let coordinator = PullCoordinator::default();
        SchedulerServices {
            store: store.clone(),
            references: ReferenceService::new(store.clone(), coordinator.clone(), events.clone()),
            school: SchoolService::new(
                coordinator,
                Arc::new(SchoolAuthService::new(MemoryVault)),
                events.clone(),
            ),
            refresh: SchoolRefresh::new(PullNetwork::Wifi),
            executor: Arc::new(NoopExecutor),
            exporter,
            events,
        }
    }

    fn add_task(store: &DataStore, id: &str, action: &str, enabled: bool) -> Value {
        store
            .put_user_record(
                "scheduled_tasks",
                json!({
                    "id":id,
                    "rev":0,
                    "updatedAt":"2026-10-13T00:00:00Z",
                    "updatedBy":"test",
                    "name":"Test task",
                    "action":action,
                    "schedule":"launch",
                    "enabled":enabled
                }),
                "test",
            )
            .unwrap()
    }

    fn task(store: &DataStore, id: &str) -> Value {
        store
            .user_rows("scheduled_tasks")
            .unwrap()
            .into_iter()
            .find(|task| task["id"].as_str() == Some(id))
            .unwrap()
    }

    #[test]
    fn rrule_claims_only_new_occurrences() {
        let task = json!({
            "enabled":true,
            "updatedAt":"2026-10-08T00:00:00Z",
            "schedule":"RRULE:FREQ=HOURLY;INTERVAL=4;BYMINUTE=0"
        });
        let started = "2026-10-08T07:59:00Z".parse().unwrap();
        let now = "2026-10-08T08:01:00Z".parse().unwrap();
        assert!(due_occurrence(&task, now, started).unwrap().is_some());
        let mut claimed = task;
        claimed["lastOccurrenceAt"] = Value::String("2026-10-08T08:00:00Z".into());
        assert!(due_occurrence(&claimed, now, started).unwrap().is_none());
    }

    #[test]
    fn legacy_runtime_boundaries_do_not_catch_up_or_run_disabled_and_subhour_tasks() {
        let started = "2026-09-29T12:05:00Z".parse().unwrap();
        let now = "2026-09-29T12:05:01Z".parse().unwrap();
        let daily = json!({
            "enabled":true,
            "updatedAt":"2026-09-29T01:00:00Z",
            "schedule":"FREQ=DAILY;BYHOUR=12;BYMINUTE=0"
        });
        assert!(due_occurrence(&daily, now, started).unwrap().is_none());

        let mut disabled = daily.clone();
        disabled["enabled"] = json!(false);
        assert!(due_occurrence(&disabled, now, started).unwrap().is_none());

        for schedule in [
            "FREQ=MINUTELY;INTERVAL=30",
            "FREQ=SECONDLY",
            "FREQ=DAILY;BYHOUR=21;BYSECOND=30",
            "FREQ=HOURLY;BYMINUTE=0,30",
        ] {
            let task = json!({
                "enabled":true,
                "updatedAt":"2026-09-29T01:00:00Z",
                "schedule":schedule
            });
            assert!(matches!(
                due_occurrence(&task, now, started),
                Err(SchedulerError::InvalidTask(message)) if message.contains("hour")
            ));
        }
    }

    #[test]
    fn launch_tasks_are_claimed_only_once_per_process_start() {
        let started = "2026-10-13T03:15:00Z".parse().unwrap();
        let now = "2026-10-13T03:15:01Z".parse().unwrap();
        let mut task = json!({"enabled":true,"schedule":"launch"});
        assert_eq!(
            due_occurrence(&task, now, started).unwrap(),
            Some(started.to_rfc3339())
        );
        task["lastOccurrenceAt"] = json!(started.to_rfc3339());
        assert!(due_occurrence(&task, now, started).unwrap().is_none());
        task["deletedAt"] = json!(now.to_rfc3339());
        assert!(due_occurrence(&task, now, started).unwrap().is_none());
    }

    #[test]
    fn reference_refresh_runs_at_launch_before_its_next_hour() {
        let started = "2026-10-13T03:15:00Z".parse().unwrap();
        let now = "2026-10-13T03:15:01Z".parse().unwrap();
        let mut task = json!({
            "id":"builtin:reference-subscriptions",
            "enabled":true,
            "updatedAt":"2026-10-12T00:00:00Z",
            "schedule":"RRULE:FREQ=HOURLY;INTERVAL=1;BYMINUTE=0",
            "lastRunAt":"2026-10-12T03:00:00Z"
        });
        assert_eq!(
            due_occurrence(&task, now, started).unwrap(),
            Some(started.to_rfc3339())
        );
        task["lastRunAt"] = json!(started.to_rfc3339());
        task["lastOccurrenceAt"] = json!(started.to_rfc3339());
        assert!(due_occurrence(&task, now, started).unwrap().is_none());
    }

    #[tokio::test]
    async fn durable_claim_precedes_the_export_side_effect() {
        let store = store("claim-first");
        add_task(&store, "claim-first", "export", true);
        let exporter = Arc::new(InspectingExporter {
            store: store.clone(),
            id: "claim-first".into(),
            observed_claim: AtomicBool::new(false),
        });
        let services = services(
            &store,
            exporter.clone(),
            Arc::new(RecordingEvents::default()),
        );

        assert_eq!(
            Scheduler::default()
                .run_now(&services, "claim-first")
                .await
                .unwrap(),
            "Export completed"
        );
        assert!(exporter.observed_claim.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn automatic_run_rechecks_disabled_task_before_claim() {
        let store = store("stale-auto");
        add_task(&store, "stale-auto", "export", false);
        let exporter = Arc::new(CountingExporter::default());
        let services = services(
            &store,
            exporter.clone(),
            Arc::new(RecordingEvents::default()),
        );
        let result = Scheduler::default()
            .run(
                &services,
                "stale-auto",
                Some(Utc::now().to_rfc3339()),
                false,
            )
            .await
            .unwrap();
        assert_eq!(result, "Skipped: scheduled task is no longer due");
        assert_eq!(exporter.0.load(Ordering::Relaxed), 0);
        assert!(task(&store, "stale-auto")["lastResult"].is_null());
    }

    #[tokio::test]
    async fn invalid_subhour_schedule_is_recorded_once_without_blocking_other_tasks() {
        let store = store("invalid-schedule");
        let mut invalid = add_task(&store, "a-invalid", "export", true);
        let revision = invalid["rev"].as_u64().unwrap();
        invalid["schedule"] = json!("FREQ=MINUTELY;INTERVAL=30");
        store
            .put_user_record_checked("scheduled_tasks", invalid, "test", Some(revision))
            .unwrap();
        add_task(&store, "z-valid", "export", true);
        let exporter = Arc::new(CountingExporter::default());
        let services = services(
            &store,
            exporter.clone(),
            Arc::new(RecordingEvents::default()),
        );
        let scheduler = Scheduler::default();

        scheduler.tick(&services).await.unwrap();
        let recorded = task(&store, "a-invalid");
        assert!(recorded["lastResult"]
            .as_str()
            .is_some_and(|result| result.starts_with("Failed:") && result.contains("hourly")));
        let recorded_revision = recorded["rev"].as_u64().unwrap();
        assert_eq!(exporter.0.load(Ordering::Relaxed), 1);

        scheduler.tick(&services).await.unwrap();
        assert_eq!(task(&store, "a-invalid")["rev"], recorded_revision);
        assert_eq!(exporter.0.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automatic_runs_defer_while_manual_runs_wait() {
        let store = store("running-lock");
        add_task(&store, "running-lock", "export", true);
        let exporter = Arc::new(CountingExporter::default());
        let services = Arc::new(services(
            &store,
            exporter.clone(),
            Arc::new(RecordingEvents::default()),
        ));
        let scheduler = Arc::new(Scheduler::default());
        let guard = scheduler.running.lock().await;

        assert_eq!(
            scheduler
                .run(
                    &services,
                    "running-lock",
                    Some(Utc::now().to_rfc3339()),
                    false,
                )
                .await
                .unwrap(),
            "Deferred: scheduler queue is busy"
        );
        let waiting = {
            let scheduler = scheduler.clone();
            let services = services.clone();
            tokio::spawn(async move { scheduler.run_now(&services, "running-lock").await })
        };
        tokio::task::yield_now().await;
        assert_eq!(exporter.0.load(Ordering::Relaxed), 0);
        drop(guard);
        assert_eq!(waiting.await.unwrap().unwrap(), "Export completed");
        assert_eq!(exporter.0.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn event_delivery_failure_does_not_leave_task_running() {
        let store = store("event-failure");
        add_task(&store, "event-failure", "export", true);
        let events = Arc::new(RecordingEvents::default());
        events.fail.store(true, Ordering::Relaxed);
        let services = services(&store, Arc::new(CountingExporter::default()), events);

        Scheduler::default()
            .run_now(&services, "event-failure")
            .await
            .unwrap();
        assert_eq!(
            task(&store, "event-failure")["lastResult"],
            "Export completed"
        );
    }

    #[tokio::test]
    async fn exporter_failure_is_typed_and_persisted() {
        let store = store("typed-export");
        add_task(&store, "typed-export", "export", true);
        let services = services(
            &store,
            Arc::new(FailingExporter),
            Arc::new(RecordingEvents::default()),
        );
        assert!(matches!(
            execute(&services, "export", true).await,
            Err(SchedulerError::Export(ExportError::Failed(message)))
                if message == "export destination unavailable"
        ));
        assert_eq!(
            Scheduler::default()
                .run_now(&services, "typed-export")
                .await
                .unwrap(),
            "Failed: export destination unavailable"
        );
        assert_eq!(
            task(&store, "typed-export")["lastResult"],
            "Failed: export destination unavailable"
        );
    }

    #[tokio::test]
    async fn action_dispatch_preserves_messages_and_foreground_policy() {
        let store = store("actions");
        let services = services(
            &store,
            Arc::new(CountingExporter::default()),
            Arc::new(RecordingEvents::default()),
        );
        services
            .refresh
            .event(RefreshEvent {
                kind: "background".into(),
                profile: None,
                course_id: None,
                network: None,
            })
            .unwrap();
        assert_eq!(
            execute(&services, "lms-refresh", false).await.unwrap(),
            "Paused: app is not active"
        );
        services
            .refresh
            .event(RefreshEvent {
                kind: "foreground".into(),
                profile: None,
                course_id: None,
                network: None,
            })
            .unwrap();
        assert_eq!(
            execute(&services, "lms-refresh", false).await.unwrap(),
            "LMS background refresh completed"
        );
        assert_eq!(
            execute(&services, "school-refresh", false).await.unwrap(),
            "School refresh completed"
        );
        assert!(matches!(
            execute(&services, "hook", false).await,
            Err(SchedulerError::InvalidTask(message))
                if message == "Scheduled custom hooks must select a built-in action."
        ));
        assert!(matches!(
            execute(&services, "unknown", false).await,
            Err(SchedulerError::InvalidTask(message))
                if message == "Scheduled task action is unsupported."
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn in_flight_user_edit_wins_over_final_scheduler_write() {
        let store = store("edit-wins");
        add_task(&store, "edit-wins", "export", true);
        let gate = Arc::new(Barrier::new(2));
        let services = Arc::new(services(
            &store,
            Arc::new(BlockingExporter { gate: gate.clone() }),
            Arc::new(RecordingEvents::default()),
        ));
        let scheduler = Arc::new(Scheduler::default());
        let running = {
            let scheduler = scheduler.clone();
            let services = services.clone();
            tokio::spawn(async move { scheduler.run_now(&services, "edit-wins").await })
        };
        gate.wait();
        let mut edited = task(&store, "edit-wins");
        let revision = edited["rev"].as_u64().unwrap();
        edited["name"] = json!("User changed name");
        store
            .put_user_record_checked("scheduled_tasks", edited, "user", Some(revision))
            .unwrap();
        gate.wait();

        assert!(matches!(
            running.await.unwrap(),
            Err(SchedulerError::Storage(DataError::Conflict(_)))
        ));
        assert_eq!(task(&store, "edit-wins")["name"], "User changed name");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn in_flight_user_delete_is_not_undone_by_final_write() {
        let store = store("delete-wins");
        add_task(&store, "delete-wins", "export", true);
        let gate = Arc::new(Barrier::new(2));
        let services = Arc::new(services(
            &store,
            Arc::new(BlockingExporter { gate: gate.clone() }),
            Arc::new(RecordingEvents::default()),
        ));
        let scheduler = Arc::new(Scheduler::default());
        let running = {
            let scheduler = scheduler.clone();
            let services = services.clone();
            tokio::spawn(async move { scheduler.run_now(&services, "delete-wins").await })
        };
        gate.wait();
        let revision = task(&store, "delete-wins")["rev"].as_u64().unwrap();
        store
            .delete_user_record_checked("scheduled_tasks", "delete-wins", "user", Some(revision))
            .unwrap();
        gate.wait();

        assert!(matches!(
            running.await.unwrap(),
            Err(SchedulerError::Storage(DataError::Conflict(_)))
        ));
        assert!(task(&store, "delete-wins")["deletedAt"].is_string());
    }

    #[tokio::test]
    async fn mcp_event_emits_only_when_the_latest_audit_changes() {
        let store = store("mcp-audit");
        for mut task in store.user_rows("scheduled_tasks").unwrap() {
            let revision = task["rev"].as_u64().unwrap();
            task["enabled"] = json!(false);
            store
                .put_user_record_checked("scheduled_tasks", task, "test", Some(revision))
                .unwrap();
        }
        let events = Arc::new(RecordingEvents::default());
        let services = services(
            &store,
            Arc::new(CountingExporter::default()),
            events.clone(),
        );
        let scheduler = Scheduler::default();

        scheduler.tick(&services).await.unwrap();
        store.record_agent_write("first").unwrap();
        scheduler.tick(&services).await.unwrap();
        scheduler.tick(&services).await.unwrap();
        store.record_agent_write("second").unwrap();
        scheduler.tick(&services).await.unwrap();

        let sources = events
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    CoreEvent::WorkspaceChanged {
                        source: WorkspaceChangeSource::Mcp
                    }
                )
            })
            .count();
        assert_eq!(sources, 2);
    }
}
