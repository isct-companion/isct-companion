use super::*;
use crate::{
    data::DataStore,
    notes::{CourseFolder, NoteStore, Notes},
};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

fn document(revision: u64) -> Value {
    json!({"tables":{"plans":[{"id":"plan","rev":revision}]},"localNotes":[]})
}

fn journal() -> Journal {
    Journal {
        state: SyncState {
            providers: vec![ProviderState {
                provider: CloudProvider::GoogleDrive,
                enabled: true,
                last_synced_at: None,
                error: None,
                synced_version: None,
            }],
            ..SyncState::default()
        },
        ..Journal::default()
    }
}

#[test]
fn manual_is_quiet_auto_queues_each_save_and_pull_does_not_echo() {
    let mut journal = journal();
    journal.observe(document(1), true).unwrap();
    assert!(journal.jobs.is_empty());
    journal.state.automatic = true;
    journal.observe(document(2), true).unwrap();
    journal.observe(document(3), true).unwrap();
    journal.observe(document(3), true).unwrap();
    assert_eq!(journal.jobs.len(), 2);
    journal.observe(document(4), false).unwrap();
    assert_eq!(journal.jobs.len(), 2);
    assert_eq!(journal.state.local_version, version(&document(4)).unwrap());
    let now = chrono::Utc::now();
    journal.state.providers[0].last_synced_at =
        Some((now - chrono::Duration::seconds(3599)).to_rfc3339());
    assert!(!journal.upload_due(CloudProvider::GoogleDrive, now));
    journal.state.providers[0].last_synced_at =
        Some((now - chrono::Duration::hours(1)).to_rfc3339());
    assert!(journal.upload_due(CloudProvider::GoogleDrive, now));
    // A pull a minute ago must not postpone an upload that is an hour old.
    journal.state.providers[0].last_synced_at =
        Some((now - chrono::Duration::seconds(60)).to_rfc3339());
    journal.uploaded_at.insert(
        CloudProvider::GoogleDrive,
        (now - chrono::Duration::hours(1)).to_rfc3339(),
    );
    assert!(journal.upload_due(CloudProvider::GoogleDrive, now));
}

#[test]
fn queue_retains_changes_and_restart_recovers_running_delivery() {
    let mut journal = journal();
    journal.enqueue(document(1), Direction::Upload).unwrap();
    journal.enqueue(document(2), Direction::Upload).unwrap();
    journal.enqueue(document(2), Direction::Upload).unwrap();
    assert_eq!(journal.jobs.len(), 2);
    let id = journal.jobs[0].task.id.clone();
    journal.jobs[0].task.deliveries[0].status = DeliveryStatus::Running;
    let mut restored: Journal =
        serde_json::from_str(&serde_json::to_string(&journal).unwrap()).unwrap();
    restored.recover();
    assert_eq!(restored.jobs[0].task.id, id);
    assert_eq!(
        restored.jobs[0].task.deliveries[0].status,
        DeliveryStatus::Pending
    );
    assert_eq!(restored.jobs[0].document, document(1));
}

#[test]
fn pull_deliveries_remain_explicit_and_provider_acknowledgements_independent() {
    let mut journal = journal();
    let mut second = journal.state.providers[0].clone();
    second.provider = CloudProvider::Icloud;
    journal.state.providers.push(second);
    journal.state.providers[0].synced_version = Some(version(&document(1)).unwrap());
    journal.enqueue(document(1), Direction::Upload).unwrap();
    assert_eq!(journal.jobs[0].task.deliveries.len(), 1);
    assert_eq!(
        journal.jobs[0].task.deliveries[0].provider,
        CloudProvider::Icloud
    );
    journal.enqueue(document(1), Direction::Pull).unwrap();
    assert_eq!(journal.jobs[1].task.deliveries.len(), 2);
    assert!(!journal.state.automatic);
    journal.jobs[0].task.deliveries[0].status = DeliveryStatus::Completed;
    let receipt = journal.jobs[0].clone();
    journal.enqueue(document(2), Direction::Upload).unwrap();
    journal.enqueue(document(3), Direction::Upload).unwrap();
    journal.coalesce_uploads().unwrap();
    assert!(
        journal.jobs[0] == receipt,
        "completed provider acknowledgements must not be rewritten by coalescing"
    );
    assert_eq!(
        journal
            .jobs
            .iter()
            .filter(|job| job.task.direction == Direction::Pull)
            .count(),
        1
    );
    let upload = journal
        .jobs
        .iter()
        .find(|job| {
            job.task.direction == Direction::Upload
                && job
                    .task
                    .deliveries
                    .iter()
                    .any(|d| d.status == DeliveryStatus::Pending)
        })
        .unwrap();
    assert_eq!(upload.task.deliveries.len(), 2);
    assert_eq!(upload.task.version, version(&document(3)).unwrap());
}

#[test]
fn unsupported_provider_deliveries_do_not_replay_and_supported_provider_can_return() {
    let legacy = temporary("provider-build").join("cloud.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let providers = [CloudProvider::GoogleDrive, CloudProvider::Icloud];
    let store = JournalStore::open(legacy.clone(), &providers, &providers).unwrap();
    store
        .change(|journal| journal.enqueue(document(1), Direction::Upload))
        .unwrap();
    // The person turned iCloud off after it had synced once.
    let synced = "2026-10-20T09:00:00+00:00".to_string();
    store
        .change(|journal| {
            let icloud = journal
                .state
                .providers
                .iter_mut()
                .find(|row| row.provider == CloudProvider::Icloud)
                .unwrap();
            icloud.enabled = false;
            icloud.last_synced_at = Some(synced.clone());
            journal.state.versions.push(CloudVersion {
                id: "icloud:1".into(),
                provider: CloudProvider::Icloud,
                saved_at: synced.clone(),
                device: "mac".into(),
                version: version(&document(1)).unwrap(),
            });
            Ok(())
        })
        .unwrap();
    drop(store);
    let debug = JournalStore::open(legacy.clone(), &[CloudProvider::GoogleDrive], &[]).unwrap();
    let state = debug.read(Journal::public_state).unwrap();
    assert_eq!(state.providers.len(), 1);
    assert!(state.versions.is_empty());
    assert_eq!(state.tasks[0].deliveries.len(), 1);
    assert_eq!(
        state.tasks[0].deliveries[0].provider,
        CloudProvider::GoogleDrive
    );
    drop(debug);
    let release = JournalStore::open(legacy.clone(), &providers, &providers).unwrap();
    let state = release.read(Journal::public_state).unwrap();
    let icloud = state
        .providers
        .iter()
        .find(|provider| provider.provider == CloudProvider::Icloud)
        .unwrap();
    assert!(
        !icloud.enabled,
        "a build without iCloud must not turn it back on for the build with it"
    );
    assert_eq!(icloud.last_synced_at, Some(synced));
    assert_eq!(state.versions.len(), 1);
    assert_eq!(state.tasks[0].deliveries.len(), 1);
    drop(release);
    // A provider no journal has seen still starts with its default.
    let _ = std::fs::remove_dir_all(legacy.parent().unwrap());
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let fresh = JournalStore::open(legacy, &providers, &[]).unwrap();
    assert!(fresh
        .read(Journal::public_state)
        .unwrap()
        .providers
        .iter()
        .any(|provider| provider.provider == CloudProvider::Icloud && provider.enabled));
}

#[test]
fn failed_sqlite_commit_does_not_change_memory() {
    let legacy = temporary("journal-rollback").join("cloud.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let store = JournalStore::open(
        legacy.clone(),
        &[CloudProvider::GoogleDrive],
        &[CloudProvider::GoogleDrive],
    )
    .unwrap();
    let sqlite = legacy.with_extension("sqlite");
    std::fs::remove_file(&sqlite).unwrap();
    std::fs::create_dir(&sqlite).unwrap();
    assert!(matches!(
        store.change(|journal| {
            journal.state.automatic = true;
            Ok(())
        }),
        Err(CloudSyncError::Storage(_))
    ));
    assert!(!store.read(|journal| journal.state.automatic).unwrap());
    std::fs::remove_dir_all(legacy.parent().unwrap()).unwrap();
}

#[test]
fn binding_rejects_a_different_workspace_without_replacing_the_first_id() {
    let fixture = Fixture::new("workspace-conflict");
    fixture
        .sync
        .bind_workspace(&json!({"sync":{"workspaceId":"first"}}))
        .unwrap();
    let error = fixture
        .sync
        .bind_workspace(&json!({"sync":{"workspaceId":"other"}}))
        .unwrap_err();
    assert!(matches!(error, CloudSyncError::Conflict(_)));
    assert_eq!(
        fixture
            .sync
            .0
            .journal
            .read(|journal| journal.workspace_id.clone())
            .unwrap()
            .as_deref(),
        Some("first")
    );
}

struct FakeTransport {
    remote: Mutex<Option<Value>>,
    fail_push: AtomicBool,
    fail_versions: AtomicBool,
    pushes: AtomicUsize,
    reads: AtomicUsize,
    started: Notify,
    release: Notify,
    block_push: AtomicBool,
    panic_push: AtomicBool,
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self {
            remote: Mutex::new(None),
            fail_push: AtomicBool::new(false),
            fail_versions: AtomicBool::new(false),
            pushes: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            started: Notify::new(),
            release: Notify::new(),
            block_push: AtomicBool::new(false),
            panic_push: AtomicBool::new(false),
        }
    }
}

impl CloudTransport for FakeTransport {
    fn disconnect(&self, _provider: CloudProvider) -> Result<(), CloudSyncError> {
        Ok(())
    }

    fn read_workspace<'a>(&'a self, _provider: CloudProvider) -> CloudFuture<'a, Option<Value>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.remote
                .lock()
                .map(|remote| remote.clone())
                .map_err(|_| CloudSyncError::Transport("Fake transport lock unavailable".into()))
        })
    }

    fn merge_and_push<'a>(
        &'a self,
        _provider: CloudProvider,
        document: &'a Value,
        _device: &'a str,
    ) -> CloudFuture<'a, Value> {
        Box::pin(async move {
            self.pushes.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            if self.block_push.load(Ordering::SeqCst) {
                self.release.notified().await;
            }
            assert!(
                !self.panic_push.swap(false, Ordering::SeqCst),
                "fake provider panic"
            );
            if self.fail_push.swap(false, Ordering::SeqCst) {
                return Err(CloudSyncError::Transport("offline".into()));
            }
            self.remote
                .lock()
                .map_err(|_| CloudSyncError::Transport("Fake transport lock unavailable".into()))?
                .replace(document.clone());
            Ok(document.clone())
        })
    }

    fn versions<'a>(&'a self, _provider: CloudProvider) -> CloudFuture<'a, Vec<CloudVersion>> {
        Box::pin(async move {
            if self.fail_versions.swap(false, Ordering::SeqCst) {
                return Err(CloudSyncError::Transport("history unavailable".into()));
            }
            Ok(Vec::new())
        })
    }

    fn version_document<'a>(
        &'a self,
        _provider: CloudProvider,
        _revision: u64,
    ) -> CloudFuture<'a, Value> {
        Box::pin(async move {
            self.remote
                .lock()
                .map_err(|_| CloudSyncError::Transport("Fake transport lock unavailable".into()))?
                .clone()
                .ok_or_else(|| CloudSyncError::Transport("version missing".into()))
        })
    }
}

#[derive(Default)]
struct FakeObserver {
    states: Mutex<Vec<SyncState>>,
    workspaces: Mutex<Vec<WorkspaceChangeSource>>,
    broken: AtomicBool,
}

impl EventSink for FakeObserver {
    fn emit(&self, event: CoreEvent) -> Result<(), crate::ports::EventError> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(crate::ports::EventError::Delivery(
                "observer unavailable".into(),
            ));
        }
        if let CoreEvent::CloudSyncState(state) = event {
            self.states.lock().unwrap().push(state);
        }
        Ok(())
    }
}

impl crate::data::CommitObserver for FakeObserver {
    fn committed(
        &self,
        source: WorkspaceChangeSource,
        _revision: Option<String>,
        _tables: &std::collections::BTreeSet<String>,
    ) {
        self.workspaces.lock().unwrap().push(source);
    }
}

struct Fixture {
    root: PathBuf,
    store: DataStore,
    sync: CloudSync,
    transport: Arc<FakeTransport>,
    observer: Arc<FakeObserver>,
    backup: BackupService,
    notes: Notes,
    note_store: NoteStore,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = temporary(name);
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize_dir(root.clone()).unwrap().0;
        let observer = Arc::new(FakeObserver::default());
        store.observe(observer.clone());
        let note_store = NoteStore::default();
        let notes = note_store.at(root.join("notes"));
        let backup = BackupService::new(store.clone(), notes.clone());
        let sync = CloudSync::new(
            root.join("cloud.json"),
            &[CloudProvider::GoogleDrive],
            &[CloudProvider::GoogleDrive],
        )
        .unwrap();
        Self {
            root,
            store,
            sync,
            transport: Arc::new(FakeTransport::default()),
            observer,
            backup,
            notes,
            note_store,
        }
    }

    fn services(&self) -> CloudServices {
        CloudServices {
            backup: self.backup.clone(),
            transport: self.transport.clone(),
            events: self.observer.clone(),
        }
    }

    fn add_plan(&self, title: &str) {
        self.store
            .put_user_record(
                "plans",
                json!({
                    "title":title,
                    "start":"2026-10-13T10:00:00+09:00",
                    "duration":60,
                    "exdates":[]
                }),
            )
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("isct-cloud-sync-{name}-{}", uuid::Uuid::new_v4()))
}

#[test]
fn local_retention_commits_the_index_before_unlinking_only_expired_files() {
    let fixture = Fixture::new("retention");
    let now = chrono::Utc::now();
    let directory = fixture.root.join("recovery");
    std::fs::create_dir_all(&directory).unwrap();
    let points = [1, 2, 3, 40, 50]
        .iter()
        .enumerate()
        .map(|(index, days)| {
            let version = version(&document(index as u64 + 1)).unwrap();
            std::fs::write(directory.join(format!("{version}.json")), "{}").unwrap();
            RecoveryPoint {
                id: format!("local:{version}"),
                version,
                saved_at: (now - chrono::Duration::days(*days)).to_rfc3339(),
            }
        })
        .collect::<Vec<_>>();
    fixture
        .sync
        .0
        .journal
        .change(|journal| {
            journal.state.recovery_points = points.clone();
            Ok(())
        })
        .unwrap();
    let missing = directory.join(format!("{}.json", points[0].version));
    std::fs::remove_file(&missing).unwrap();
    fixture.sync.prune_recovery().unwrap();
    assert_eq!(fixture.sync.state().unwrap().recovery_points.len(), 5);
    std::fs::write(missing, "{}").unwrap();
    fixture.sync.prune_recovery().unwrap();
    let state = fixture.sync.state().unwrap();
    assert_eq!(state.recovery_points.len(), 4);
    assert!(directory
        .join(format!("{}.json", points[3].version))
        .exists());
    assert!(!directory
        .join(format!("{}.json", points[4].version))
        .exists());
    assert!(!state
        .recovery_points
        .iter()
        .any(|point| point.id == points[4].id));
}

#[tokio::test]
async fn hourly_automatic_snapshots_coalesce_changes_survive_restart_and_skip_unchanged_content() {
    let mut fixture = Fixture::new("hourly-auto");
    fixture
        .sync
        .set_automatic(fixture.services(), true)
        .await
        .unwrap();
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 1);
    let reads = fixture.transport.reads.load(Ordering::SeqCst);
    for title in ["first", "second"] {
        fixture.add_plan(title);
        fixture.sync.changed(fixture.services()).await.unwrap();
    }
    let waiting = fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(
        fixture.transport.reads.load(Ordering::SeqCst),
        reads,
        "waiting for the hour must not poll the cloud"
    );
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(
        waiting
            .tasks
            .iter()
            .filter(|task| task
                .deliveries
                .iter()
                .any(|delivery| delivery.status == DeliveryStatus::Pending))
            .count(),
        1
    );
    fixture.sync.shutdown().await;
    fixture.sync = CloudSync::new(
        fixture.root.join("cloud.json"),
        &[CloudProvider::GoogleDrive],
        &[CloudProvider::GoogleDrive],
    )
    .unwrap();
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(
        fixture.transport.pushes.load(Ordering::SeqCst),
        1,
        "restart must preserve the hour limit, even after its launch pull"
    );
    assert_eq!(fixture.transport.reads.load(Ordering::SeqCst), reads + 1);
    let reads = reads + 1;
    fixture
        .sync
        .0
        .journal
        .change(|journal| {
            journal.uploaded_at.insert(
                CloudProvider::GoogleDrive,
                (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339(),
            );
            Ok(())
        })
        .unwrap();
    fixture
        .sync
        .provider_result(
            fixture.services(),
            CloudProvider::GoogleDrive,
            false,
            Some("authorization unavailable".into()),
        )
        .await
        .unwrap();
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(
        fixture.transport.reads.load(Ordering::SeqCst),
        reads,
        "disabled providers cannot trigger reconciliation reads"
    );
    fixture
        .sync
        .provider_result(fixture.services(), CloudProvider::GoogleDrive, true, None)
        .await
        .unwrap();
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 2);
    let remote = fixture.transport.remote.lock().unwrap().clone().unwrap();
    assert_eq!(remote["tables"]["plans"].as_array().unwrap().len(), 2);
    fixture.sync.changed(fixture.services()).await.unwrap();
    fixture
        .sync
        .0
        .journal
        .change(|journal| {
            journal.uploaded_at.insert(
                CloudProvider::GoogleDrive,
                (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339(),
            );
            Ok(())
        })
        .unwrap();
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(
        fixture.transport.pushes.load(Ordering::SeqCst),
        2,
        "an elapsed hour alone must not upload unchanged content"
    );
    // A deliberate manual upload need not wait for another hour.
    fixture
        .sync
        .0
        .journal
        .change(|journal| {
            journal
                .uploaded_at
                .insert(CloudProvider::GoogleDrive, chrono::Utc::now().to_rfc3339());
            Ok(())
        })
        .unwrap();
    fixture.add_plan("manual");
    fixture.sync.changed(fixture.services()).await.unwrap();
    fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 3);
    fixture.sync.shutdown().await;
}

/// A device that only reads (a phone opened to check the timetable) never
/// uploads, so without a launch pull it would never see the Mac's edits.
#[tokio::test]
async fn automatic_launch_pulls_other_devices_changes_once_and_manual_launch_does_not() {
    let mac = Fixture::new("launch-mac");
    let automatic = |fixture: &Fixture| {
        fixture
            .sync
            .0
            .journal
            .change(|journal| {
                journal.state.automatic = true;
                Ok(())
            })
            .unwrap()
    };
    let mut phone = Fixture::new("launch-phone");
    phone.transport = mac.transport.clone();
    automatic(&phone);
    // No cloud copy yet: quiet, and no workspace id the Mac's copy would then conflict with.
    let state = phone.sync.resume(phone.services()).await.unwrap();
    assert!(state.providers[0].error.is_none());
    assert!(phone
        .sync
        .0
        .journal
        .read(|journal| journal.workspace_id.clone())
        .unwrap()
        .is_none());

    mac.add_plan("from mac");
    mac.sync
        .sync(mac.services(), Direction::Upload)
        .await
        .unwrap();
    phone.sync.resume(phone.services()).await.unwrap();
    assert!(
        phone.store.user_rows("plans").unwrap().is_empty(),
        "only the first pass of a launch pulls"
    );

    phone.sync.shutdown().await;
    phone.sync = CloudSync::new(
        phone.root.join("cloud.json"),
        &[CloudProvider::GoogleDrive],
        &[CloudProvider::GoogleDrive],
    )
    .unwrap();
    let pushes = mac.transport.pushes.load(Ordering::SeqCst);
    phone.sync.resume(phone.services()).await.unwrap();
    assert_eq!(phone.store.user_rows("plans").unwrap().len(), 1);
    assert_eq!(
        mac.transport.pushes.load(Ordering::SeqCst),
        pushes,
        "a pull that leaves this device level with the cloud uploads nothing"
    );

    let mut manual = Fixture::new("launch-manual");
    manual.transport = mac.transport.clone();
    manual.sync.resume(manual.services()).await.unwrap();
    assert!(manual.store.user_rows("plans").unwrap().is_empty());
    phone.sync.shutdown().await;
    mac.sync.shutdown().await;
    manual.sync.shutdown().await;
}

#[tokio::test]
async fn failed_delivery_is_durable_and_manual_retry_reuses_it() {
    let fixture = Fixture::new("retry");
    fixture.add_plan("first");
    fixture.transport.fail_push.store(true, Ordering::SeqCst);

    let failed = fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(failed.tasks.len(), 1);
    assert_eq!(
        failed.tasks[0].deliveries[0].status,
        DeliveryStatus::Pending
    );
    assert_eq!(
        failed.tasks[0].deliveries[0].error.as_deref(),
        Some("offline")
    );
    let task = failed.tasks[0].id.clone();
    fixture
        .sync
        .set_automatic(fixture.services(), true)
        .await
        .unwrap();
    for name in ["second", "third"] {
        fixture.add_plan(name);
        fixture.sync.changed(fixture.services()).await.unwrap();
    }
    let reads = fixture.transport.reads.load(Ordering::SeqCst);
    let waiting = fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(fixture.transport.reads.load(Ordering::SeqCst), reads);
    assert_eq!(
        fixture.transport.pushes.load(Ordering::SeqCst),
        1,
        "new saves must not bypass the provider cooldown"
    );
    let pending = waiting
        .tasks
        .iter()
        .flat_map(|task| &task.deliveries)
        .filter(|delivery| delivery.status == DeliveryStatus::Pending)
        .collect::<Vec<_>>();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);
    assert_eq!(pending[0].retry_at, failed.tasks[0].deliveries[0].retry_at);
    let changes = || fixture.observer.workspaces.lock().unwrap().clone();
    assert_eq!(changes(), [WorkspaceChangeSource::UserIntent; 3]);

    let retried = fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 2);
    let receipt = retried.tasks.iter().find(|row| row.id == task).unwrap();
    assert_eq!(receipt.deliveries[0].status, DeliveryStatus::Completed);
    assert_eq!(retried.providers[0].error, None);
    // Applying the merged cloud copy is a store write with its own source.
    assert_eq!(changes().last(), Some(&WorkspaceChangeSource::CloudSync));
}

#[tokio::test]
async fn history_and_observer_failures_do_not_retry_a_completed_delivery() {
    let fixture = Fixture::new("broken-observer");
    fixture.add_plan("saved");
    fixture.observer.broken.store(true, Ordering::SeqCst);
    fixture
        .transport
        .fail_versions
        .store(true, Ordering::SeqCst);

    let state = fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.tasks[0].deliveries[0].status,
        DeliveryStatus::Completed
    );
    assert_eq!(state.providers[0].error, None);
    assert!(state.providers[0].last_synced_at.is_some());
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn provider_panic_is_settled_and_the_same_delivery_can_retry() {
    let fixture = Fixture::new("provider-panic");
    fixture.add_plan("saved");
    fixture.transport.panic_push.store(true, Ordering::SeqCst);

    let error = fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap_err();
    assert_eq!(error, CloudSyncError::Task(RuntimeTaskError::Panicked));
    assert_eq!(
        error.to_string(),
        "Cloud sync worker failed; the saved task will retry."
    );
    let pending = fixture.sync.state().unwrap();
    assert!(!pending.syncing);
    assert_eq!(
        pending.tasks[0].deliveries[0].status,
        DeliveryStatus::Pending
    );
    assert_eq!(pending.tasks[0].deliveries[0].retry_at, 0);
    let task = pending.tasks[0].id.clone();

    let retried = fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap();
    let receipt = retried.tasks.iter().find(|row| row.id == task).unwrap();
    assert_eq!(receipt.deliveries[0].status, DeliveryStatus::Completed);
}

#[tokio::test]
async fn caller_drop_does_not_cancel_delivery_or_overwrite_a_new_local_edit() {
    let fixture = Fixture::new("caller-drop");
    fixture.add_plan("before");
    fixture.transport.block_push.store(true, Ordering::SeqCst);
    let sync = fixture.sync.clone();
    let services = fixture.services();
    let caller = tokio::spawn(async move { sync.sync(services, Direction::Upload).await });
    fixture.transport.started.notified().await;

    fixture.add_plan("during-network");
    caller.abort();
    fixture.transport.release.notify_one();

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if fixture.sync.state().unwrap().tasks.iter().any(|task| {
                task.deliveries
                    .iter()
                    .all(|delivery| delivery.status == DeliveryStatus::Completed)
            }) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.store.user_rows("plans").unwrap().len(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn slow_note_gate_does_not_starve_unrelated_runtime_work() {
    let fixture = Fixture::new("slow-note-gate");
    let notes = fixture.notes.clone();
    let (started_send, started_receive) = std::sync::mpsc::sync_channel(1);
    let (release_send, release_receive) = std::sync::mpsc::sync_channel(1);
    let blocker = std::thread::spawn(move || {
        notes
            .transaction(|_| {
                started_send.send(()).unwrap();
                let _ = release_receive.recv_timeout(Duration::from_secs(1));
                Ok::<(), crate::notes::NoteError>(())
            })
            .unwrap();
    });
    started_receive
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert!(fixture.sync.tick(fixture.services()).unwrap());
    *fixture.sync.0.next_tick.lock().unwrap() = Instant::now();
    assert!(!fixture.sync.tick(fixture.services()).unwrap());
    let unrelated = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        "runtime-progress"
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(200), unrelated)
            .await
            .unwrap()
            .unwrap(),
        "runtime-progress"
    );

    release_send.send(()).unwrap();
    blocker.join().unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while fixture.sync.0.tick_pass.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    *fixture.sync.0.next_tick.lock().unwrap() = Instant::now();
    assert!(fixture.sync.tick(fixture.services()).unwrap());
    fixture.sync.shutdown().await;
}

#[tokio::test]
async fn shutdown_interrupts_remote_work_and_leaves_delivery_retryable() {
    let fixture = Fixture::new("shutdown");
    fixture.add_plan("pending");
    fixture.transport.block_push.store(true, Ordering::SeqCst);
    let sync = fixture.sync.clone();
    let services = fixture.services();
    let caller = tokio::spawn(async move { sync.sync(services, Direction::Upload).await });
    fixture.transport.started.notified().await;

    tokio::time::timeout(Duration::from_secs(2), fixture.sync.shutdown())
        .await
        .unwrap();
    assert!(matches!(
        caller.await.unwrap(),
        Err(CloudSyncError::Stopped)
    ));
    let state = fixture.sync.state().unwrap();
    assert!(!state.syncing);
    assert_eq!(state.tasks[0].deliveries[0].status, DeliveryStatus::Pending);
    assert_eq!(state.tasks[0].deliveries[0].retry_at, 0);
}

#[tokio::test]
async fn pending_apply_survives_journal_failure_after_writing_notes_and_replays() {
    let fixture = Fixture::new("pending-apply-commit-failure");
    let source = Fixture::new("pending-apply-commit-source");
    source.add_plan("replayed");
    source
        .notes
        .save(
            CourseFolder {
                term: "2026-3Q".into(),
                course: "cloud-recovery".into(),
                section: "notes".into(),
            },
            String::new(),
            "written before journal acknowledgement".into(),
            "source-device",
        )
        .unwrap();
    let document = source.backup.snapshot().unwrap();
    fixture
        .sync
        .0
        .journal
        .change(|journal| {
            journal.pending_apply = Some(document);
            Ok(())
        })
        .unwrap();

    let connection = rusqlite::Connection::open(&fixture.sync.0.journal.path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER fail_pending_apply_ack BEFORE UPDATE ON journal
             BEGIN SELECT RAISE(FAIL, 'injected journal failure'); END;",
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        fixture.sync.resume(fixture.services()).await,
        Err(CloudSyncError::Storage(_))
    ));
    assert_eq!(
        fixture.backup.snapshot().unwrap()["localNotes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(fixture
        .sync
        .0
        .journal
        .read(|journal| journal.pending_apply.is_some())
        .unwrap());
    // The record tables are applied in the same attempt, before the journal
    // acknowledgement fails.
    assert_eq!(fixture.store.user_rows("plans").unwrap().len(), 1);

    let connection = rusqlite::Connection::open(&fixture.sync.0.journal.path).unwrap();
    connection
        .execute_batch("DROP TRIGGER fail_pending_apply_ack")
        .unwrap();
    drop(connection);

    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(
        fixture.backup.snapshot().unwrap()["localNotes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The replay applies the record tables too, not only the notes.
    assert_eq!(fixture.store.user_rows("plans").unwrap().len(), 1);
    assert!(fixture
        .sync
        .0
        .journal
        .read(|journal| journal.pending_apply.is_none())
        .unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_root_change_keeps_pending_apply_for_the_fresh_backup_service() {
    let fixture = Fixture::new("storage-root-change");
    fixture.add_plan("start-upload");
    let source = Fixture::new("storage-root-change-source");
    source
        .notes
        .save(
            CourseFolder {
                term: "2026-3Q".into(),
                course: "cloud-recovery".into(),
                section: "notes".into(),
            },
            String::new(),
            "write only to the newly selected root".into(),
            "source-device",
        )
        .unwrap();
    *fixture.transport.remote.lock().unwrap() = Some(source.backup.snapshot().unwrap());
    fixture.transport.block_push.store(true, Ordering::SeqCst);

    let sync = fixture.sync.clone();
    let old_services = fixture.services();
    let caller = tokio::spawn(async move { sync.sync(old_services, Direction::Upload).await });
    fixture.transport.started.notified().await;

    // Hold the acknowledgement database after the post-network snapshot. The
    // recovery file is written immediately before that blocked journal update,
    // giving the storage selection a deterministic hand-off point.
    let connection = rusqlite::Connection::open(&fixture.sync.0.journal.path).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    fixture.transport.release.notify_one();
    let recovery = fixture.root.join("recovery");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let written = std::fs::read_dir(&recovery).ok().is_some_and(|entries| {
                entries.filter_map(Result::ok).any(|entry| {
                    entry.path().extension().and_then(|value| value.to_str()) == Some("json")
                })
            });
            if written {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let new_root = fixture.root.join("new-notes");
    fixture
        .note_store
        .select_root(new_root.clone(), || Ok::<(), &str>(()))
        .unwrap();
    connection.execute_batch("COMMIT").unwrap();
    drop(connection);
    caller.await.unwrap().unwrap();

    assert!(fixture
        .sync
        .0
        .journal
        .read(|journal| journal.pending_apply.is_some())
        .unwrap());
    let old_note_directory = fixture
        .root
        .join("notes/2026-3Q/cloud-recovery/notes/notes");
    assert!(!old_note_directory.exists());

    fixture.transport.block_push.store(false, Ordering::SeqCst);
    let fresh_backup = BackupService::new(
        fixture.store.clone(),
        fixture.note_store.at(new_root.clone()),
    );
    let fresh_services = CloudServices {
        backup: fresh_backup.clone(),
        transport: fixture.transport.clone(),
        events: fixture.observer.clone(),
    };
    fixture.sync.resume(fresh_services).await.unwrap();

    assert!(fixture
        .sync
        .0
        .journal
        .read(|journal| journal.pending_apply.is_none())
        .unwrap());
    assert_eq!(
        fresh_backup.snapshot().unwrap()["localNotes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!old_note_directory.exists());
    assert!(new_root.join("2026-3Q/cloud-recovery/notes/notes").is_dir());
}

#[tokio::test]
async fn invalid_restore_is_compatibility_error_before_local_mutation() {
    let fixture = Fixture::new("invalid-restore");
    fixture.add_plan("keep");
    let error = fixture
        .sync
        .restore_document(fixture.services(), json!({"not":"a backup"}))
        .await
        .unwrap_err();
    assert!(matches!(error, CloudSyncError::Compatibility(_)));
    assert_eq!(fixture.store.user_rows("plans").unwrap().len(), 1);
}

#[tokio::test]
async fn maximum_remote_generation_restore_is_rejected_before_enqueue_or_apply() {
    let fixture = Fixture::new("restore-generation-overflow");
    fixture.add_plan("keep-local");
    let source = Fixture::new("restore-generation-overflow-source");
    source.add_plan("must-not-apply");
    let document = source.backup.snapshot().unwrap();
    let mut remote = document.clone();
    remote["sync"] = json!({"generation":{"counter":u64::MAX,"id":"maximum-generation"}});
    *fixture.transport.remote.lock().unwrap() = Some(remote);

    let error = fixture
        .sync
        .restore_document(fixture.services(), document)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        CloudSyncError::Compatibility("Cloud generation is too large.".into())
    );
    assert!(fixture.sync.state().unwrap().tasks.is_empty());
    let plans = fixture.store.user_rows("plans").unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0]["title"], "keep-local");
}

#[tokio::test]
async fn idle_pass_neither_rewrites_the_journal_nor_publishes() {
    let fixture = Fixture::new("idle-pass");
    let path = fixture.sync.0.journal.path.clone();
    let sentinel = || {
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("UPDATE journal SET value='sentinel' WHERE id=1", [])
            .unwrap();
    };
    let stored = || -> String {
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT value FROM journal WHERE id=1", [], |row| row.get(0))
            .unwrap()
    };
    sentinel();
    fixture.sync.resume(fixture.services()).await.unwrap();
    fixture.sync.resume(fixture.services()).await.unwrap();
    assert_eq!(stored(), "sentinel");
    assert!(fixture.observer.states.lock().unwrap().is_empty());

    fixture
        .sync
        .0
        .journal
        .change(|journal| {
            journal.state.automatic = true;
            Ok(())
        })
        .unwrap();
    assert_ne!(stored(), "sentinel");
}
