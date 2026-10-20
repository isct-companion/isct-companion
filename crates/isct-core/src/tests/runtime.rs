use super::*;
use crate::{
    data::DataStore,
    ports::{SecretError, SecretVault},
    scheduler::{ExportError, ScheduledExporter},
    school_auth::SchoolAuthService,
    sync_engine::{Network, SyncEngine},
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

fn services(store: &DataStore, exporter: Arc<dyn ScheduledExporter>) -> SchedulerServices {
    SchedulerServices {
        store: store.clone(),
        exporter,
    }
}

/// A runtime over `store` whose sync engine has no school browser, so a wake
/// reads nothing from the network.
fn runtime(store: &DataStore) -> Arc<CoreRuntime> {
    Arc::new(CoreRuntime::new(SyncEngine::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault)),
        Network::Offline,
    )))
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
    let store = test_store("suspend-resume");
    let runtime = runtime(&store);
    disable_background_work(&store);
    add_launch_task(&store, "first");
    let exporter = Arc::new(CountingExporter::default());
    let services = services(&store, exporter.clone());

    runtime.suspend().await;
    runtime.wake(services.clone()).await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 0);

    runtime.resume().await;
    runtime.wake(services.clone()).await.unwrap();
    wait_for_count(&exporter.0, 1).await;
    let idle = runtime.ticking.clone().lock_owned().await;
    drop(idle);

    add_launch_task(&store, "second");
    runtime.wake(services.clone()).await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 1);

    *runtime.next_scheduled.lock().unwrap() = Instant::now();
    runtime.wake(services).await.unwrap();
    wait_for_count(&exporter.0, 2).await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn cancelling_the_caller_does_not_detach_runtime_work() {
    let runtime = runtime(&test_store("detach"));
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
    let store = test_store("final-shutdown");
    let runtime = runtime(&store);
    disable_background_work(&store);
    let services = services(&store, Arc::new(CountingExporter::default()));

    runtime.shutdown().await;
    runtime.shutdown().await;
    runtime.resume().await;

    assert_eq!(
        runtime.run(async { 42 }).await,
        Err(RuntimeTaskError::Stopped)
    );
    assert!(matches!(
        runtime.wake(services).await,
        Err(RuntimeError::Task(RuntimeTaskError::Stopped))
    ));
    runtime.shutdown().await;
}
