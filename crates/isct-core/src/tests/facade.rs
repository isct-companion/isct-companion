use super::*;
use crate::{
    cloud_sync::{CloudFuture, CloudProvider, CloudSyncError, CloudTransport, CloudVersion},
    ports::{EventError, SecretError, SecretVault},
    schedule::{NotificationItem, NotificationSink, NotificationSinkError},
    scheduler::{ExportError, ScheduledExporter},
    school_service::{SchoolExecution, SchoolExecutor, SchoolExecutorFuture},
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("vault unavailable".into()))?
            .insert(account.into(), secret.into());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SecretError::Access("vault unavailable".into()))?
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("vault unavailable".into()))?
            .remove(account);
        Ok(())
    }
}

struct NoopEvents;
impl EventSink for NoopEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), EventError> {
        Ok(())
    }
}

struct NoopExecutor;
impl SchoolExecutor for NoopExecutor {
    fn execute(
        &self,
        _request: SchoolExecution,
        _context: crate::pull_coordinator::PullContext,
    ) -> SchoolExecutorFuture<'_> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

struct NoopExporter;
impl ScheduledExporter for NoopExporter {
    fn export(&self) -> Result<(), ExportError> {
        Ok(())
    }
}

struct NoopNotifications;
impl NotificationSink for NoopNotifications {
    fn deliver<'a>(
        &'a self,
        _item: NotificationItem,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), NotificationSinkError>> + Send + 'a>,
    > {
        Box::pin(async { Ok(()) })
    }
}

struct NoopCloud;
impl CloudTransport for NoopCloud {
    fn disconnect(&self, _provider: CloudProvider) -> Result<(), CloudSyncError> {
        Ok(())
    }
    fn read_workspace<'a>(
        &'a self,
        _provider: CloudProvider,
    ) -> CloudFuture<'a, Option<serde_json::Value>> {
        Box::pin(async { Ok(None) })
    }
    fn merge_and_push<'a>(
        &'a self,
        _provider: CloudProvider,
        document: &'a serde_json::Value,
        _device: &'a str,
    ) -> CloudFuture<'a, serde_json::Value> {
        Box::pin(async move { Ok(document.clone()) })
    }
    fn versions<'a>(&'a self, _provider: CloudProvider) -> CloudFuture<'a, Vec<CloudVersion>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn version_document<'a>(
        &'a self,
        _provider: CloudProvider,
        _revision: u64,
    ) -> CloudFuture<'a, serde_json::Value> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }
}

struct DropEvents(Arc<AtomicBool>);
impl EventSink for DropEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), EventError> {
        Ok(())
    }
}
impl Drop for DropEvents {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("isct-core-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

#[tokio::test]
async fn credential_mutations_work_without_a_school_browser() {
    let store = store("auth");
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    let core = Core::new(store, auth.clone(), PullNetwork::Wifi);
    let profile = SchoolAuthProfile::TokyoTech;

    assert!(core
        .save_credentials(
            profile,
            SchoolCredentials {
                username: "student".into(),
                password: "secret".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        )
        .unwrap());
    assert!(auth.load(profile).unwrap().is_some());
    assert!(core.delete_credentials(profile).unwrap());
    assert!(auth.load(profile).unwrap().is_none());
    core.shutdown().await;
}

#[tokio::test]
async fn wake_reports_every_service_after_runtime_failure() {
    let store = store("wake-report");
    let files = std::env::temp_dir().join(format!("isct-core-files-{}", Uuid::new_v4()));
    let core = Core::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        PullNetwork::Wifi,
    );
    let scheduler = core.services(
        Arc::new(NoopEvents),
        Arc::new(NoopExecutor),
        Arc::new(NoopExporter),
    );
    let cloud = crate::cloud_sync::CloudServices {
        backup: crate::backup::BackupService::new(store, core.runtime.notes.at(files)),
        transport: Arc::new(NoopCloud),
        events: Arc::new(NoopEvents),
    };
    core.runtime.shutdown().await;

    let report = core
        .wake(WakeServices {
            scheduler,
            school_visible: false,
            cloud: Some(cloud),
            reminders: Some(ReminderWake {
                sink: Arc::new(NoopNotifications),
                now: chrono::Utc::now().fixed_offset(),
            }),
        })
        .await;
    assert!(matches!(
        report.runtime,
        Err(RuntimeError::Task(
            crate::runtime_tasks::RuntimeTaskError::Stopped
        ))
    ));
    assert!(matches!(
        report.cloud,
        Some(Err(CloudWakeError::NotAttached))
    ));
    assert!(matches!(report.reminders, Some(Ok(_))));
    core.shutdown().await;
}

#[tokio::test]
async fn shutdown_releases_the_native_event_port() {
    let dropped = Arc::new(AtomicBool::new(false));
    let core = Core::with_events(
        store("event-release"),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        PullNetwork::Wifi,
        Arc::new(DropEvents(dropped.clone())),
    );
    core.start(&tokio::runtime::Handle::current()).unwrap();
    assert!(!dropped.load(Ordering::Acquire));
    core.shutdown().await;
    assert!(dropped.load(Ordering::Acquire));
}
