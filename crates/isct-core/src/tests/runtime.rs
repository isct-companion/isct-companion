use super::*;
use crate::{
    data::DataStore,
    events::CoreEvent,
    ports::{EventError, EventSink, SecretError, SecretVault},
    pull_coordinator::{PullContext, PullOptions, PullTarget},
    reference_refresh::ReferenceService,
    scheduler::{ExportError, ScheduledExporter},
    school_auth::SchoolAuthService,
    school_service::{IngestOutcome, IngestState, SchoolExecution, SchoolExecutor, SchoolService},
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;
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
struct PullEvents(AtomicUsize);

impl EventSink for PullEvents {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        if matches!(event, CoreEvent::PullStateChanged(_)) {
            self.0.fetch_add(1, Ordering::Release);
        }
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

fn test_store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("isct-runtime-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

fn disable_background_work(store: &DataStore) {
    for table in ["scheduled_tasks", "subscriptions"] {
        for mut row in store.user_rows(table).unwrap() {
            let revision = row["rev"].as_u64().unwrap();
            row["enabled"] = Value::Bool(false);
            store
                .put_user_record_checked(
                    table,
                    row,
                    crate::data::WorkspaceChangeSource::UserIntent,
                    Some(revision),
                )
                .unwrap();
        }
    }
}

fn services(
    runtime: &Arc<CoreRuntime>,
    store: &DataStore,
    exporter: Arc<dyn ScheduledExporter>,
) -> SchedulerServices {
    SchedulerServices {
        store: store.clone(),
        references: ReferenceService::new(store.clone(), runtime.coordinator.clone()),
        school: SchoolService::new(
            runtime.coordinator.clone(),
            Arc::new(SchoolAuthService::new(MemoryVault)),
            runtime.refresh.clone(),
        ),
        refresh: runtime.refresh.clone(),
        executor: Arc::new(NoopExecutor),
        exporter,
    }
}

fn add_launch_task(store: &DataStore, id: &str) {
    store
        .put_user_record(
            "scheduled_tasks",
            json!({
                "id": id,
                "rev": 0,
                "updatedAt": "2026-10-13T00:00:00Z",
                "updatedBy": "test",
                "name": "Runtime lifecycle test",
                "action": "export",
                "schedule": "launch",
                "enabled": true
            }),
        )
        .unwrap();
}

async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while counter.load(Ordering::Relaxed) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn suspend_defers_automatic_work_and_resume_respects_scheduler_gate() {
    let runtime = Arc::new(CoreRuntime::new(PullNetwork::Wifi));
    let store = test_store("suspend-resume");
    disable_background_work(&store);
    add_launch_task(&store, "first");
    let exporter = Arc::new(CountingExporter::default());
    let services = services(&runtime, &store, exporter.clone());

    runtime.suspend().await;
    runtime.wake(services.clone(), false).await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 0);

    runtime.resume().await;
    runtime.wake(services.clone(), false).await.unwrap();
    wait_for_count(&exporter.0, 1).await;
    let idle = runtime.ticking.clone().lock_owned().await;
    drop(idle);

    add_launch_task(&store, "second");
    runtime.wake(services.clone(), false).await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 1);

    *runtime.next_scheduled.lock().unwrap() = Instant::now();
    runtime.wake(services, false).await.unwrap();
    wait_for_count(&exporter.0, 2).await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn shutdown_waits_for_post_pull_metadata_and_event_work() {
    let runtime = Arc::new(CoreRuntime::new(PullNetwork::Wifi));
    let postprocessing = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let metadata_written = Arc::new(AtomicBool::new(false));
    let events_emitted = Arc::new(AtomicUsize::new(0));

    let caller = {
        let runtime = runtime.clone();
        let coordinator = runtime.coordinator.clone();
        let postprocessing = postprocessing.clone();
        let release = release.clone();
        let metadata_written = metadata_written.clone();
        let events_emitted = events_emitted.clone();
        tokio::spawn(async move {
            runtime
                .run(async move {
                    coordinator
                        .run(
                            PullTarget::Reference {
                                kind: "test".into(),
                                index_url: "https://example.test/index.json".into(),
                                year: 2026,
                            },
                            PullOptions::manual(),
                            |context| async move {
                                assert!(context.phase("downloaded").await);
                                assert!(context.applying().await);
                                Ok(json!({"changed": 1}))
                            },
                        )
                        .await
                        .unwrap();
                    postprocessing.notify_one();
                    release.notified().await;
                    metadata_written.store(true, Ordering::Release);
                    events_emitted.fetch_add(1, Ordering::Release);
                })
                .await
        })
    };

    postprocessing.notified().await;
    let mut shutdown = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.shutdown().await })
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut shutdown)
            .await
            .is_err()
    );
    assert!(!metadata_written.load(Ordering::Acquire));
    assert_eq!(events_emitted.load(Ordering::Acquire), 0);

    release.notify_one();
    assert!(caller.await.unwrap().is_ok());
    shutdown.await.unwrap();
    assert!(metadata_written.load(Ordering::Acquire));
    assert_eq!(events_emitted.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn cancelling_the_caller_does_not_detach_runtime_work() {
    let runtime = Arc::new(CoreRuntime::new(PullNetwork::Wifi));
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let finished = Arc::new(AtomicBool::new(false));
    let caller = {
        let runtime = runtime.clone();
        let started = started.clone();
        let release = release.clone();
        let finished = finished.clone();
        tokio::spawn(async move {
            runtime
                .run(async move {
                    started.notify_one();
                    release.notified().await;
                    finished.store(true, Ordering::Release);
                })
                .await
        })
    };

    started.notified().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let mut shutdown = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.shutdown().await })
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), &mut shutdown)
            .await
            .is_err()
    );

    release.notify_one();
    shutdown.await.unwrap();
    assert!(finished.load(Ordering::Acquire));
}

#[tokio::test]
async fn shutdown_is_final_and_idempotent() {
    let runtime = Arc::new(CoreRuntime::new(PullNetwork::Wifi));
    let store = test_store("final-shutdown");
    disable_background_work(&store);
    let services = services(&runtime, &store, Arc::new(CountingExporter::default()));

    runtime.shutdown().await;
    runtime.shutdown().await;
    runtime.resume().await;

    assert_eq!(
        runtime.run(async { 42 }).await,
        Err(RuntimeTaskError::Stopped)
    );
    assert!(matches!(
        runtime.wake(services, false).await,
        Err(RuntimeError::Task(RuntimeTaskError::Stopped))
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn runtime_owns_one_pull_forwarder_and_releases_it_on_shutdown() {
    let runtime = Arc::new(CoreRuntime::new(PullNetwork::Wifi));
    let events = Arc::new(PullEvents::default());
    runtime
        .start_event_forwarding(events.clone(), &tokio::runtime::Handle::current())
        .unwrap();
    runtime
        .start_event_forwarding(events.clone(), &tokio::runtime::Handle::current())
        .unwrap();

    runtime
        .coordinator
        .run(
            PullTarget::Reference {
                kind: "forwarding-test".into(),
                index_url: "https://example.test/index.json".into(),
                year: 2026,
            },
            PullOptions::manual(),
            |_| async { Ok(json!({"changed": 0})) },
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while events.0.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(1), runtime.shutdown())
        .await
        .unwrap();
    assert_eq!(Arc::strong_count(&events), 1);
    assert_eq!(
        runtime.start_event_forwarding(events, &tokio::runtime::Handle::current()),
        Err(RuntimeTaskError::Stopped)
    );
}
