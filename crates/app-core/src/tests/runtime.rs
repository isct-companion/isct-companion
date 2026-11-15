use super::*;
use crate::plan::Duty;
use crate::{
    data::DataStore,
    plan::PlanHints,
    scheduler::{ExportError, ScheduledExporter},
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Default)]
struct CountingExporter(AtomicUsize);

impl ScheduledExporter for CountingExporter {
    fn export(&self) -> Result<(), ExportError> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn test_store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("isct-runtime-{name}-{}", Uuid::new_v4()));
    DataStore::initialize_dir(path).unwrap().0
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

/// A runtime with no engine behind its hints, so nothing reads the
/// network.
fn runtime() -> Arc<CoreRuntime> {
    Arc::new(CoreRuntime::new(PlanHints::detached()))
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

/// The person's tasks are a duty of the engine: due at once until a pass
/// ran, then at their next occurrence, which a launch task has only once
/// per run. A change to the tasks makes the duty due again, and the pass
/// runs what is new; without a shell's export port nothing is due.
#[tokio::test]
async fn scheduled_tasks_are_due_at_their_occurrence_and_after_a_change() {
    let store = test_store("scheduled");
    let runtime = runtime();
    disable_background_work(&store);
    add_launch_task(&store, "first");
    let duty = Arc::new(crate::deadlines::Scheduled {
        runtime: runtime.clone(),
        store: store.clone(),
    });
    assert_eq!(duty.next_due(), None, "no export port, nothing due");
    let exporter = Arc::new(CountingExporter::default());
    runtime.attach_exporter(exporter.clone());
    assert!(duty.next_due().is_some_and(|at| at <= chrono::Utc::now()));
    duty.clone().run().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 1);
    assert_eq!(duty.next_due(), None, "a launch task runs once per run");
    duty.clone().run().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 1);

    add_launch_task(&store, "second");
    runtime.scheduled_tasks_changed();
    assert!(duty.next_due().is_some_and(|at| at <= chrono::Utc::now()));
    duty.clone().run().await;
    assert_eq!(exporter.0.load(Ordering::Relaxed), 2);
    runtime.shutdown().await;
}

#[tokio::test]
async fn cancelling_the_caller_does_not_detach_runtime_work() {
    let runtime = runtime();
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
    let runtime = runtime();
    disable_background_work(&store);
    add_launch_task(&store, "first");
    runtime.attach_exporter(Arc::new(CountingExporter::default()));

    runtime.shutdown().await;
    runtime.shutdown().await;
    runtime.resume().await;

    assert_eq!(
        runtime.run(async { 42 }).await,
        Err(RuntimeTaskError::Stopped)
    );
    assert!(
        matches!(
            runtime.run_scheduled(&store, "first".into()).await,
            Err(RuntimeError::NoExporter)
        ),
        "shutdown releases the shell's port"
    );
    runtime.shutdown().await;
}
