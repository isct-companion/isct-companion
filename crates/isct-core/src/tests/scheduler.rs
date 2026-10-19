use super::*;
use crate::{
    ports::{SecretError, SecretVault},
    pull_coordinator::{PullContext, PullCoordinator, PullNetwork},
    school_auth::SchoolAuthService,
    school_refresh::RefreshEvent,
    school_service::{IngestState, SchoolExecution},
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Barrier,
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
                state: IngestState::Ok,
                changed: 0,
                message: String::new(),
            }])
        })
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

fn action(name: &str) -> Option<ScheduledAction> {
    ScheduledAction::of(&json!({ "action": name }))
}

fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("isct-scheduler-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

fn services(store: &DataStore, exporter: Arc<dyn ScheduledExporter>) -> SchedulerServices {
    let coordinator = PullCoordinator::default();
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    SchedulerServices {
        store: store.clone(),
        references: ReferenceService::new(store.clone(), coordinator.clone()),
        school: SchoolService::new(
            coordinator,
            Arc::new(SchoolAuthService::new(MemoryVault)),
            refresh.clone(),
        ),
        refresh,
        executor: Arc::new(NoopExecutor),
        exporter,
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
    let services = services(&store, exporter.clone());

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
    let services = services(&store, exporter.clone());
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
        .put_user_record_checked(
            "scheduled_tasks",
            invalid,
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(revision),
        )
        .unwrap();
    add_task(&store, "z-valid", "export", true);
    let exporter = Arc::new(CountingExporter::default());
    let services = services(&store, exporter.clone());
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
    let services = Arc::new(services(&store, exporter.clone()));
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
async fn exporter_failure_is_typed_and_persisted() {
    let store = store("typed-export");
    add_task(&store, "typed-export", "export", true);
    let services = services(&store, Arc::new(FailingExporter));
    assert!(matches!(
        execute(&services, action("export"), true).await,
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
    let services = services(&store, Arc::new(CountingExporter::default()));
    services
        .refresh
        .event(RefreshEvent::lifecycle(false))
        .unwrap();
    assert_eq!(
        execute(&services, action("lms-refresh"), false)
            .await
            .unwrap(),
        "Paused: app is not active"
    );
    services
        .refresh
        .event(RefreshEvent::lifecycle(true))
        .unwrap();
    assert_eq!(
        execute(&services, action("lms-refresh"), false)
            .await
            .unwrap(),
        "LMS background refresh completed"
    );
    assert_eq!(
        execute(&services, action("school-refresh"), false)
            .await
            .unwrap(),
        "School refresh completed"
    );
    assert!(matches!(
        execute(&services, action("hook"), false).await,
        Err(SchedulerError::InvalidTask(message))
            if message == "Scheduled custom hooks must select a built-in action."
    ));
    assert!(matches!(
        execute(&services, action("unknown"), false).await,
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
        .put_user_record_checked(
            "scheduled_tasks",
            edited,
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(revision),
        )
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
        .delete_user_record_checked("scheduled_tasks", "delete-wins", Some(revision))
        .unwrap();
    gate.wait();

    assert!(matches!(
        running.await.unwrap(),
        Err(SchedulerError::Storage(DataError::Conflict(_)))
    ));
    assert!(task(&store, "delete-wins")["deletedAt"].is_string());
}
