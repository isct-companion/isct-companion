use super::*;
use crate::data::DataStore;
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
    json!({"tables":{"personal_events":[{"id":"plan","rev":revision}]}})
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

// A journal written while OneDrive was offered still opens: its OneDrive
// row, deliveries and upload time are dropped and Drive's are kept.
#[test]
fn a_journal_naming_a_retired_provider_still_opens() {
    let legacy = temporary("journal-retired").join("cloud.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    let mut saved = serde_json::to_value(journal()).unwrap();
    saved["providers"]
        .as_array_mut()
        .unwrap()
        .push(json!({"provider":"onedrive","enabled":true,"lastSyncedAt":null,"error":null}));
    saved["uploaded_at"] =
        json!({"onedrive":"2026-10-01T00:00:00Z","google-drive":"2026-10-02T00:00:00Z"});
    std::fs::write(&legacy, serde_json::to_vec(&saved).unwrap()).unwrap();
    let store = JournalStore::open(
        legacy.clone(),
        &[CloudProvider::GoogleDrive],
        &[CloudProvider::GoogleDrive],
    )
    .unwrap();
    let (providers, uploaded) = store
        .read(|journal| {
            (
                journal
                    .state
                    .providers
                    .iter()
                    .map(|row| row.provider)
                    .collect::<Vec<_>>(),
                journal.uploaded_at.clone(),
            )
        })
        .unwrap();
    assert_eq!(providers, [CloudProvider::GoogleDrive]);
    assert_eq!(
        uploaded
            .get(&CloudProvider::GoogleDrive)
            .map(String::as_str),
        Some("2026-10-02T00:00:00Z")
    );
    std::fs::remove_dir_all(legacy.parent().unwrap()).unwrap();
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

/// Every provider's workspace envelope in memory, behind the real
/// `DocumentCloud` merge-and-push loop, so devices with different provider
/// sets can share clouds the way real ones do.
#[derive(Default)]
struct MemoryClouds(Mutex<std::collections::HashMap<CloudProvider, document::CloudWorkspaceFile>>);

impl document::WorkspaceTransport for MemoryClouds {
    fn read<'a>(
        &'a self,
        provider: CloudProvider,
    ) -> document::BoxFuture<'a, Option<document::CloudWorkspaceFile>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().get(&provider).cloned()) })
    }

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a document::CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> document::BoxFuture<'a, ()> {
        Box::pin(async move {
            let mut clouds = self.0.lock().unwrap();
            if clouds.get(&provider).map(|file| file.revision) != expected_revision {
                return Err(document::DocumentError::Conflict);
            }
            clouds.insert(provider, file.clone());
            Ok(())
        })
    }

    fn disconnect(&self, _provider: CloudProvider) -> Result<(), document::DocumentError> {
        Ok(())
    }
}

impl MemoryClouds {
    fn document(&self, provider: CloudProvider) -> Value {
        self.0.lock().unwrap()[&provider].document.clone()
    }

    fn titles(&self, provider: CloudProvider) -> Vec<String> {
        titles(
            self.document(provider)["tables"]["personal_events"]
                .as_array()
                .unwrap(),
        )
    }
}

fn titles(rows: &[Value]) -> Vec<String> {
    let mut titles = rows
        .iter()
        .filter(|row| row["deletedAt"].is_null())
        .map(|row| row["title"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    titles.sort();
    titles
}

struct Fixture {
    root: PathBuf,
    store: DataStore,
    sync: CloudSync,
    transport: Arc<FakeTransport>,
    /// Replaces `transport` for devices on shared `MemoryClouds`.
    clouds: Option<Arc<dyn CloudTransport>>,
    observer: Arc<FakeObserver>,
    backup: BackupService,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self::with(name, &[CloudProvider::GoogleDrive], None)
    }

    /// A device with these providers on, sharing `clouds` with other devices.
    fn on(name: &str, providers: &[CloudProvider], clouds: &Arc<MemoryClouds>) -> Self {
        let transport = document::DocumentCloud::new(MemoryCloudsRef(clouds.clone()), None);
        Self::with(name, providers, Some(Arc::new(transport)))
    }

    fn with(
        name: &str,
        providers: &[CloudProvider],
        clouds: Option<Arc<dyn CloudTransport>>,
    ) -> Self {
        let root = temporary(name);
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize_dir(root.clone()).unwrap().0;
        let observer = Arc::new(FakeObserver::default());
        store.observe(observer.clone());
        let backup = BackupService::new(store.clone());
        let sync = CloudSync::new(root.join("cloud.json"), providers, providers).unwrap();
        Self {
            root,
            store,
            sync,
            transport: Arc::new(FakeTransport::default()),
            clouds,
            observer,
            backup,
        }
    }

    fn services(&self) -> CloudServices {
        CloudServices {
            backup: self.backup.clone(),
            transport: self
                .clouds
                .clone()
                .unwrap_or_else(|| self.transport.clone()),
            events: self.observer.clone(),
        }
    }

    fn add_event(&self, title: &str) {
        self.store
            .put_user_record(
                "personal_events",
                json!({
                    "kind":"event", "termId":"2026-3Q", "title":title, "room":"",
                    "days":[1], "startTime":"10:00", "endTime":"11:00"
                }),
            )
            .unwrap();
    }

    fn titles(&self) -> Vec<String> {
        titles(&self.store.user_rows("personal_events").unwrap())
    }

    fn automatic(&self) {
        self.sync
            .0
            .journal
            .change(|journal| {
                journal.state.automatic = true;
                Ok(())
            })
            .unwrap();
    }

    /// Moves this process's last remote check and each provider's last
    /// upload back by `hours`, as if the app had stayed open that long.
    fn age(&self, check_hours: u64, upload_hours: i64) {
        self.sync.0.remote_check.lock().unwrap().last =
            Instant::now().checked_sub(Duration::from_secs(check_hours * 3600));
        self.sync
            .0
            .journal
            .change(|journal| {
                for provider in journal.state.providers.iter().map(|row| row.provider) {
                    journal.uploaded_at.insert(
                        provider,
                        (chrono::Utc::now() - chrono::Duration::hours(upload_hours)).to_rfc3339(),
                    );
                }
                Ok(())
            })
            .unwrap();
    }
}

/// `DocumentCloud` owns its transport; devices share the clouds through this.
struct MemoryCloudsRef(Arc<MemoryClouds>);

impl document::WorkspaceTransport for MemoryCloudsRef {
    fn read<'a>(
        &'a self,
        provider: CloudProvider,
    ) -> document::BoxFuture<'a, Option<document::CloudWorkspaceFile>> {
        self.0.read(provider)
    }

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a document::CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> document::BoxFuture<'a, ()> {
        self.0.write(provider, file, expected_revision)
    }

    fn disconnect(&self, provider: CloudProvider) -> Result<(), document::DocumentError> {
        self.0.disconnect(provider)
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
        fixture.add_event(title);
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
    assert_eq!(
        remote["tables"]["personal_events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
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
    fixture.add_event("manual");
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
    // No cloud copy yet: quiet.
    let state = phone.sync.resume(phone.services()).await.unwrap();
    assert!(state.providers[0].error.is_none());

    mac.add_event("from mac");
    mac.sync
        .sync(mac.services(), Direction::Upload)
        .await
        .unwrap();
    phone.sync.resume(phone.services()).await.unwrap();
    assert!(
        phone.store.user_rows("personal_events").unwrap().is_empty(),
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
    assert_eq!(phone.store.user_rows("personal_events").unwrap().len(), 1);
    assert_eq!(
        mac.transport.pushes.load(Ordering::SeqCst),
        pushes,
        "a pull that leaves this device level with the cloud uploads nothing"
    );

    let mut manual = Fixture::new("launch-manual");
    manual.transport = mac.transport.clone();
    manual.sync.resume(manual.services()).await.unwrap();
    assert!(manual
        .store
        .user_rows("personal_events")
        .unwrap()
        .is_empty());
    phone.sync.shutdown().await;
    mac.sync.shutdown().await;
    manual.sync.shutdown().await;
}

#[tokio::test]
async fn failed_delivery_is_durable_and_manual_retry_reuses_it() {
    let fixture = Fixture::new("retry");
    fixture.add_event("first");
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
        fixture.add_event(name);
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
    let pending = |direction| {
        waiting
            .tasks
            .iter()
            .filter(|task| task.direction == direction)
            .flat_map(|task| &task.deliveries)
            .filter(|delivery| delivery.status == DeliveryStatus::Pending)
            .collect::<Vec<_>>()
    };
    // Turning Auto on queues a remote check; it waits out the same cooldown.
    assert_eq!(pending(Direction::Pull).len(), 1);
    let pending = pending(Direction::Upload);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].attempts, 1);
    assert_eq!(pending[0].retry_at, failed.tasks[0].deliveries[0].retry_at);
    let changes = || fixture.observer.workspaces.lock().unwrap().clone();
    assert_eq!(changes(), [WorkspaceChangeSource::UserIntent; 3]);

    // Another device's copy is merged in on the retry, so applying the result
    // is a store write with its own source.
    let other = Fixture::new("retry-other");
    other.add_event("from another device");
    *fixture.transport.remote.lock().unwrap() = Some(other.backup.snapshot().unwrap());
    let retried = fixture
        .sync
        .sync(fixture.services(), Direction::Upload)
        .await
        .unwrap();
    assert_eq!(fixture.transport.pushes.load(Ordering::SeqCst), 2);
    let receipt = retried.tasks.iter().find(|row| row.id == task).unwrap();
    assert_eq!(receipt.deliveries[0].status, DeliveryStatus::Completed);
    assert_eq!(retried.providers[0].error, None);
    assert_eq!(changes().last(), Some(&WorkspaceChangeSource::CloudSync));
}

#[tokio::test]
async fn history_and_observer_failures_do_not_retry_a_completed_delivery() {
    let fixture = Fixture::new("broken-observer");
    fixture.add_event("saved");
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
    fixture.add_event("saved");
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
    fixture.add_event("before");
    fixture.transport.block_push.store(true, Ordering::SeqCst);
    let sync = fixture.sync.clone();
    let services = fixture.services();
    let caller = tokio::spawn(async move { sync.sync(services, Direction::Upload).await });
    fixture.transport.started.notified().await;

    fixture.add_event("during-network");
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
    assert_eq!(fixture.store.user_rows("personal_events").unwrap().len(), 2);
}

#[tokio::test]
async fn shutdown_interrupts_remote_work_and_leaves_delivery_retryable() {
    let fixture = Fixture::new("shutdown");
    fixture.add_event("pending");
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
async fn pending_apply_survives_journal_failure_and_replays() {
    let fixture = Fixture::new("pending-apply-commit-failure");
    let source = Fixture::new("pending-apply-commit-source");
    source.add_event("replayed");
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
    assert!(fixture
        .sync
        .0
        .journal
        .read(|journal| journal.pending_apply.is_some())
        .unwrap());
    // The record tables are applied in the same attempt, before the journal
    // acknowledgement fails.
    assert_eq!(fixture.store.user_rows("personal_events").unwrap().len(), 1);

    let connection = rusqlite::Connection::open(&fixture.sync.0.journal.path).unwrap();
    connection
        .execute_batch("DROP TRIGGER fail_pending_apply_ack")
        .unwrap();
    drop(connection);

    fixture.sync.resume(fixture.services()).await.unwrap();
    // The replay applies the records again without duplicating them.
    assert_eq!(fixture.store.user_rows("personal_events").unwrap().len(), 1);
    assert!(fixture
        .sync
        .0
        .journal
        .read(|journal| journal.pending_apply.is_none())
        .unwrap());
}

#[tokio::test]
async fn invalid_restore_is_compatibility_error_before_local_mutation() {
    let fixture = Fixture::new("invalid-restore");
    fixture.add_event("keep");
    let error = fixture
        .sync
        .restore_document(fixture.services(), json!({"not":"a backup"}))
        .await
        .unwrap_err();
    assert!(matches!(error, CloudSyncError::Compatibility(_)));
    assert_eq!(fixture.store.user_rows("personal_events").unwrap().len(), 1);
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

/// iCloud on the iPhone and Drive on the Android phone were started apart, so
/// their copies carry different workspace ids, and Android's backup import
/// put it a restore generation ahead. A Mac with both providers must join them
/// instead of rejecting either; every device then ends on one workspace with
/// every record, including a never-synced tablet that brings its own data.
#[tokio::test]
async fn copies_started_separately_merge_across_providers_and_every_device_converges() {
    use CloudProvider::{GoogleDrive as Drive, Icloud};
    let clouds = Arc::new(MemoryClouds::default());
    let android = Fixture::on("join-android", &[Drive], &clouds);
    android.add_event("android");
    // A restore uploads at once, as an ordinary newest edit.
    let imported = android.backup.snapshot().unwrap();
    android
        .sync
        .restore_document(android.services(), imported)
        .await
        .unwrap();
    assert_eq!(clouds.titles(Drive), ["android"]);
    let iphone = Fixture::on("join-iphone", &[Icloud], &clouds);
    iphone.add_event("iphone");
    iphone
        .sync
        .sync(iphone.services(), Direction::Upload)
        .await
        .unwrap();

    let mac = Fixture::on("join-mac", &[Drive, Icloud], &clouds);
    mac.add_event("mac");
    let state = mac
        .sync
        .sync(mac.services(), Direction::Upload)
        .await
        .unwrap();
    assert!(state.providers.iter().all(|row| row.error.is_none()));
    let all = ["android", "iphone", "mac"].map(String::from).to_vec();
    assert_eq!(mac.titles(), all);
    for provider in [Drive, Icloud] {
        assert_eq!(clouds.titles(provider), all, "{provider:?}");
    }

    let tablet = Fixture::on("join-tablet", &[Drive], &clouds);
    tablet.add_event("tablet");
    for device in [&iphone, &android, &tablet] {
        device
            .sync
            .sync(device.services(), Direction::Pull)
            .await
            .unwrap();
    }
    assert_eq!(iphone.titles(), all);
    assert_eq!(android.titles(), all);
    assert_eq!(
        tablet.titles(),
        ["android", "iphone", "mac", "tablet"].map(String::from),
        "a device that never met the cloud keeps its records"
    );
    for device in [android, iphone, mac, tablet] {
        device.sync.shutdown().await;
    }
}

/// A Mac with both providers stays open while the iPhone writes to iCloud.
/// Without a local edit on the Mac, its hourly remote check must still bring
/// the change to the Drive-only Android phone, within the hourly upload limit.
#[tokio::test]
async fn a_device_with_both_providers_relays_on_its_remote_checks_without_a_local_edit() {
    use CloudProvider::{GoogleDrive as Drive, Icloud};
    let clouds = Arc::new(MemoryClouds::default());
    let mac = Fixture::on("relay-mac", &[Drive, Icloud], &clouds);
    mac.automatic();
    mac.add_event("mac");
    mac.sync
        .sync(mac.services(), Direction::Upload)
        .await
        .unwrap();
    let iphone = Fixture::on("relay-iphone", &[Icloud], &clouds);
    iphone
        .sync
        .sync(iphone.services(), Direction::Pull)
        .await
        .unwrap();
    iphone.add_event("iphone");
    iphone
        .sync
        .sync(iphone.services(), Direction::Upload)
        .await
        .unwrap();
    assert_eq!(clouds.titles(Drive), ["mac"]);

    // Coming to the front within five minutes of the last check reads nothing.
    mac.sync.resume(mac.services()).await.unwrap();
    assert_eq!(mac.titles(), ["mac"]);

    // An hour on, the check pulls iCloud's change. Drive took an upload ten
    // minutes ago, so the relay waits for the hour instead of skipping it.
    mac.age(2, 0);
    mac.sync
        .0
        .journal
        .change(|journal| {
            journal.uploaded_at.insert(
                Drive,
                (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339(),
            );
            Ok(())
        })
        .unwrap();
    // The pass a periodic `tick` runs, without the shell's front signal.
    mac.sync.resume_inner(&mac.services()).await.unwrap();
    assert_eq!(mac.titles(), ["iphone", "mac"]);
    assert_eq!(clouds.titles(Drive), ["mac"]);
    assert!(mac.sync.state().unwrap().tasks.iter().any(|task| {
        task.direction == Direction::Upload
            && task.deliveries.iter().any(|delivery| {
                delivery.provider == Drive && delivery.status == DeliveryStatus::Pending
            })
    }));

    mac.age(0, 2);
    mac.sync.resume(mac.services()).await.unwrap();
    assert_eq!(clouds.titles(Drive), ["iphone", "mac"]);
    let android = Fixture::on("relay-android", &[Drive], &clouds);
    android
        .sync
        .sync(android.services(), Direction::Pull)
        .await
        .unwrap();
    assert_eq!(android.titles(), ["iphone", "mac"]);
    for device in [android, iphone, mac] {
        device.sync.shutdown().await;
    }
}
