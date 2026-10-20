use super::*;
use crate::workspace::{WorkspaceIntent, WorkspaceMutation, WorkspaceSnapshot};
use crate::{
    agent::AgentPolicy,
    cloud_sync::{
        CloudFuture, CloudProvider, CloudSync, CloudSyncError, CloudTransport, CloudVersion,
    },
    events::CoreEvent,
    notes::CourseFolder,
    ports::{EventError, SecretError, SecretVault},
    schedule::{NotificationItem, NotificationSink, NotificationSinkError},
    scheduler::{ExportError, ScheduledExporter},
    school_service::{SchoolExecution, SchoolExecutor, SchoolExecutorFuture},
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
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
    let scheduler = core.services(Arc::new(NoopExecutor), Arc::new(NoopExporter));
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

#[derive(Default)]
struct RecordedEvents(Mutex<Vec<(&'static str, serde_json::Value)>>);
impl EventSink for RecordedEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), EventError> {
        self.0
            .lock()
            .map_err(|_| EventError::Delivery("recorder unavailable".into()))?
            .push((event.name(), event.payload()));
        Ok(())
    }
}

#[derive(Default)]
struct WorkspaceEvents {
    changes: Mutex<Vec<(WorkspaceChangeSource, Option<String>)>>,
    fail: AtomicBool,
}

impl EventSink for WorkspaceEvents {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        if let CoreEvent::WorkspaceChanged { source, revision } = event {
            self.changes.lock().unwrap().push((source, revision));
        }
        if self.fail.load(Ordering::Relaxed) {
            return Err(EventError::Delivery("event channel closed".into()));
        }
        Ok(())
    }
}

#[tokio::test]
async fn the_relay_sends_school_status_after_the_events_that_change_it() {
    use crate::pull_coordinator::{PullPriority, PullSnapshot, PullState, PullTarget};
    use crate::sync_state::{self, Commit, Dataset, Key, Outcome};
    let store = store("school-status");
    store
        .write(None, |transaction| {
            sync_state::commit(
                transaction,
                &Key::new(Dataset::LmsDeadlines, "all"),
                Commit {
                    account: "science:student",
                    validator: None,
                    outcome: Outcome::Complete,
                    message: None,
                },
                "2026-10-19T02:46:13Z".parse().unwrap(),
            )
        })
        .unwrap();
    let recorded = Arc::new(RecordedEvents::default());
    let core = Core::with_events(
        store,
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        PullNetwork::Wifi,
        recorded.clone(),
    );
    let relay = core.events.clone().unwrap();
    let finished = CoreEvent::PullStateChanged(vec![PullSnapshot {
        id: "pull".into(),
        target: PullTarget::School {
            site: crate::lifecycle::SchoolSite::ScienceLms,
            course_id: None,
        },
        priority: PullPriority::Manual,
        state: PullState::Succeeded,
        phase: None,
        blocked_by: None,
        attempt: 1,
        waiter_count: 0,
        last_error: None,
    }]);

    relay.emit(finished.clone()).unwrap();
    // Unchanged status and unrelated events send nothing more.
    relay.emit(finished).unwrap();
    relay
        .emit(CoreEvent::WorkspaceChanged {
            source: WorkspaceChangeSource::SchoolPull,
            revision: None,
        })
        .unwrap();

    let names: Vec<_> = recorded
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        names,
        [
            "pull-state-changed",
            "school-status",
            "pull-state-changed",
            "workspace-changed"
        ]
    );
    let sent = recorded.0.lock().unwrap()[1].1.clone();
    assert_eq!(sent["headline"], serde_json::json!({"kind": "idle"}));
    assert_eq!(sent["sources"][0]["site"], "science-lms");
    assert_eq!(
        sent["sources"][0]["lastUpdated"],
        "2026-10-19T02:46:13+00:00"
    );
    // The query answers what the event said, for a shell that missed it.
    assert_eq!(
        serde_json::to_value(core.school_status().await.unwrap()).unwrap(),
        sent
    );
    core.shutdown().await;
}

fn admission_year(
    core: &Core,
    expected_revision: String,
    year: i32,
) -> Result<WorkspaceSnapshot, WorkspaceError> {
    core.workspace.mutate(WorkspaceMutation {
        expected_revision,
        intent: WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(year),
        },
    })
}

/// Shells and agent adapters must not have to publish what they wrote: the
/// store's commit path reports every change, once, with its source.
#[test]
fn core_publishes_each_committed_write_once_whoever_made_it() {
    let store = store("published-writes");
    store
        .put_workspace_setting("devicePreferences", json!({"agentAccess":{"enabled":true}}))
        .unwrap();
    let events = Arc::new(WorkspaceEvents::default());
    let core = Core::with_events(
        store,
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        PullNetwork::Wifi,
        events.clone(),
    );
    let changes = || events.changes.lock().unwrap().clone();

    let committed =
        admission_year(&core, core.workspace.snapshot().unwrap().revision, 2026).unwrap();
    assert_eq!(
        changes(),
        [(
            WorkspaceChangeSource::UserIntent,
            Some(committed.revision.clone())
        )]
    );

    // A rejected intent and one that changes no row publish nothing.
    assert!(matches!(
        admission_year(&core, "stale".into(), 2025),
        Err(WorkspaceError::Conflict(_))
    ));
    admission_year(&core, committed.revision.clone(), 2026).unwrap();
    assert_eq!(changes().len(), 1);

    // An MCP tool writes outside WorkspaceService and is still published.
    core.agent
        .call(
            "plan_event_create",
            &json!({"title":"Review","start":"2026-10-19T09:00:00+09:00","duration":45}),
            AgentPolicy { allow_writes: true },
        )
        .unwrap();
    assert_eq!(changes().last(), Some(&(WorkspaceChangeSource::Mcp, None)));

    // A failed delivery neither fails nor undoes the committed write.
    events.fail.store(true, Ordering::Relaxed);
    let after = admission_year(&core, core.workspace.snapshot().unwrap().revision, 2027).unwrap();
    assert_eq!(after.workspace["academicProfile"]["admissionYear"], 2027);
    assert_eq!(changes().len(), 3);
}

/// Cloud capture follows local changes without a shell asking for it: a
/// committed write or a saved note makes the next wake capture at once
/// instead of after the periodic interval.
#[tokio::test]
async fn local_changes_bring_the_next_cloud_capture_forward() {
    let root = std::env::temp_dir().join(format!("isct-core-capture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = store("capture");
    let core = Core::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        PullNetwork::Wifi,
    );
    core.runtime
        .attach_cloud(
            CloudSync::new(root.join("cloud.json"), &[CloudProvider::GoogleDrive], &[]).unwrap(),
        )
        .unwrap();
    let notes = core.runtime.notes.at(root.join("notes"));
    let services = crate::cloud_sync::CloudServices {
        backup: crate::backup::BackupService::new(store, notes.clone()),
        transport: Arc::new(NoopCloud),
        events: Arc::new(NoopEvents),
    };
    let wake = || core.runtime.wake_cloud(services.clone()).unwrap();
    async fn settle(wake: impl Fn() -> bool) {
        // The interval is five seconds, so no capture is due while idle.
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(!wake());
        }
    }
    async fn captures(wake: impl Fn() -> bool) -> bool {
        for _ in 0..100 {
            if wake() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    assert!(wake());
    settle(&wake).await;
    admission_year(&core, core.workspace.snapshot().unwrap().revision, 2026).unwrap();
    assert!(captures(&wake).await);

    settle(&wake).await;
    notes
        .save(
            CourseFolder {
                term: "2026-3Q".into(),
                course: "CSC.T251".into(),
                section: "section-1".into(),
            },
            String::new(),
            "Note".into(),
            "device",
        )
        .unwrap();
    assert!(captures(&wake).await);

    core.shutdown().await;
    let _ = std::fs::remove_dir_all(root);
}
