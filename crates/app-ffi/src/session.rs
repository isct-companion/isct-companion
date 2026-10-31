use crate::{
    events::{Events, Subscription},
    json::{project, project_input},
    operation::{Operation, OperationResult, ScheduleSnapshot},
    ports::*,
    types::*,
    JsonValue, SdkError,
};
use app_core::{
    backup::BackupService,
    cloud_sync::{CloudServices, CloudSync},
    data::DataStore,
    runtime_tasks::RuntimeTasks,
    school_auth::SchoolAuthService,
    Core,
};
use futures_util::FutureExt;
use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
};
use tokio::runtime::{Handle, Runtime};
use tokio_util::sync::CancellationToken;

/// The mobile shells' one web screen, the presenter of every site it opens.
const WEB_SURFACE: &str = "web";

#[derive(Clone, Debug, uniffi::Record)]
pub struct SessionConfig {
    pub data_directory: String,
    pub files_directory: String,
    /// Sent to Core's planner as the first `Hint::Network`.
    pub initial_network: SyncNetwork,
    pub cloud_providers: Vec<CloudProvider>,
    pub authorized_cloud_providers: Vec<CloudProvider>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct Capabilities {
    pub school_browser: bool,
    pub downloads: bool,
    pub cloud: bool,
    pub notifications: bool,
    pub ocr: bool,
}
struct RuntimeOwner(Option<Runtime>);
impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        if let Some(runtime) = self.0.take() {
            runtime.shutdown_background()
        }
    }
}
pub(crate) struct Inner {
    pub core: Arc<Core>,
    pub events: Arc<Events>,
    pub tasks: RuntimeTasks,
    pub handle: Handle,
    gate: Mutex<bool>,
    next_operation: AtomicU64,
    operations: Mutex<HashMap<u64, Weak<Operation>>>,
    shutdown: tokio::sync::Mutex<bool>,
    closed: CancellationToken,
    pub backup: BackupService,
    /// Always present so imports and restores go through the journal; the
    /// providers only run when the shell installed a `NativeCloud`.
    sync: CloudSync,
    sync_services: CloudServices,
    /// Drive's sign-in, when the shell installed a Drive port.
    drive: Option<Arc<DriveAuth>>,
    pub notifications: Option<Arc<dyn NativeNotifications>>,
    pub download_session: Option<Arc<dyn NativeDownloadSession>>,
    pub files: PathBuf,
    backup_directory: PathBuf,
    capabilities: Capabilities,
    browser_shutdown_error: Option<Arc<Mutex<Option<SdkError>>>>,
    /// The Matrix driver every photo and camera scan goes through.
    #[cfg(feature = "ocr")]
    pub matrix: Arc<app_core::matrix_scan::MatrixCameraDriver>,
}
impl Inner {
    /// The journal and its providers, when the shell installed a cloud store.
    pub(crate) fn cloud(&self) -> Result<(&CloudSync, &CloudServices), SdkError> {
        if self.capabilities.cloud {
            Ok((&self.sync, &self.sync_services))
        } else {
            Err(unavailable("Cloud transport"))
        }
    }
}
#[derive(uniffi::Object)]
pub struct Session {
    pub(crate) inner: Arc<Inner>,
    runtime: Mutex<RuntimeOwner>,
}
impl Inner {
    fn native_close_result(&self) -> Result<(), SdkError> {
        if let Some(error) = &self.browser_shutdown_error {
            if let Some(error) = error.lock().map_err(|_| SdkError::Closed)?.clone() {
                return Err(error);
            }
        }
        Ok(())
    }
    async fn close(&self) -> Result<(), SdkError> {
        let mut done = self.shutdown.lock().await;
        if *done {
            return self.native_close_result();
        }
        *self.gate.lock().map_err(|_| SdkError::Closed)? = true;
        self.tasks.close();
        self.closed.cancel();
        if let Ok(operations) = self.operations.lock() {
            for op in operations.values().filter_map(Weak::upgrade) {
                if op.cancellable {
                    op.cancellation.cancel()
                }
            }
        }
        self.core.shutdown().await;
        self.tasks.wait().await;
        self.events.close();
        *done = true;
        self.native_close_result()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let Ok(owner) = self.runtime.get_mut() else {
            return;
        };
        let Some(runtime) = owner.0.take() else {
            return;
        };
        let inner = self.inner.clone();
        // A foreign finalizer must not block or drop a Tokio runtime on a UI thread.
        let owner = RuntimeOwner(Some(runtime));
        let _ = std::thread::Builder::new()
            .name("app-core-close".into())
            .spawn(move || {
                if let Some(runtime) = owner.0.as_ref() {
                    let _ = runtime.block_on(inner.close());
                }
            });
    }
}
impl Session {
    pub(crate) async fn shutdown_with_timeout(
        &self,
        limit: std::time::Duration,
    ) -> Result<(), SdkError> {
        let inner = self.inner.clone();
        let drain = self.inner.handle.spawn(async move { inner.close().await });
        // Expiry detaches only this wait. The runtime still owns the drain and
        // every admitted commit, and callers may wait for shutdown again.
        self.inner
            .handle
            .spawn(async move {
                tokio::time::timeout(limit, drain)
                    .await
                    .map_err(|_| SdkError::Timeout {
                        detail:
                            "Shutdown is still draining owned work; retry shutdown to wait again"
                                .into(),
                    })?
                    .map_err(app_core::runtime_tasks::RuntimeTaskError::from)?
            })
            .await
            .map_err(app_core::runtime_tasks::RuntimeTaskError::from)?
    }

    pub(crate) fn start<F, Fut>(
        &self,
        cancellable: bool,
        work: F,
    ) -> Result<Arc<Operation>, SdkError>
    where
        F: FnOnce(Arc<Inner>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<OperationResult, SdkError>> + Send + 'static,
    {
        self.start_identified(cancellable, move |inner, _, cancel| work(inner, cancel))
    }
    pub(crate) fn start_identified<F, Fut>(
        &self,
        cancellable: bool,
        work: F,
    ) -> Result<Arc<Operation>, SdkError>
    where
        F: FnOnce(Arc<Inner>, u64, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = Result<OperationResult, SdkError>> + Send + 'static,
    {
        let closed = self.inner.gate.lock().map_err(|_| SdkError::Closed)?;
        if *closed {
            return Err(SdkError::Closed);
        }
        let id = self.inner.next_operation.fetch_add(1, Ordering::Relaxed);
        let (operation, sender) = Operation::new(id, cancellable);
        let operation = Arc::new(operation);
        let mut operations = self.inner.operations.lock().map_err(|_| SdkError::Closed)?;
        operations.retain(|_, value| value.strong_count() > 0);
        operations.insert(id, Arc::downgrade(&operation));
        drop(operations);
        let inner = self.inner.clone();
        let cancel = operation.cancellation.clone();
        if !self.inner.tasks.spawn_on(&self.inner.handle, async move {
            let result = std::panic::AssertUnwindSafe(async move { work(inner, id, cancel).await })
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(SdkError::Internal {
                        detail: "Owned operation unwound; query again before retrying".into(),
                    })
                });
            sender.send_replace(Some(result));
        }) {
            return Err(SdkError::Closed);
        }
        Ok(operation)
    }
    pub(crate) fn blocking<F>(&self, work: F) -> Result<Arc<Operation>, SdkError>
    where
        F: FnOnce(Arc<Inner>) -> Result<OperationResult, SdkError> + Send + 'static,
    {
        self.start(false, move |inner, _| async move {
            tokio::task::spawn_blocking(move || work(inner))
                .await
                .map_err(app_core::runtime_tasks::RuntimeTaskError::from)?
        })
    }
}
#[uniffi::export]
impl Session {
    // One argument per native port keeps every shell's call site explicit
    // about which ports it installs.
    #[allow(clippy::too_many_arguments)]
    #[uniffi::constructor]
    pub async fn open(
        config: SessionConfig,
        vault: Arc<dyn NativeVault>,
        browser: Option<Arc<dyn NativeBrowser>>,
        cloud: Option<Arc<dyn NativeCloud>>,
        notifications: Option<Arc<dyn NativeNotifications>>,
        downloads: Option<Arc<dyn NativeDownloadSession>>,
        web_authentication: Option<Arc<dyn NativeWebAuthentication>>,
        drive_tokens: Option<Arc<dyn NativeDriveTokens>>,
    ) -> Result<Arc<Self>, SdkError> {
        // The first session installs Core's logger for the process; later
        // sessions find it in place, which is not an error.
        let _ = app_core::diagnostics::initialize();
        let data = PathBuf::from(&config.data_directory);
        let files = PathBuf::from(&config.files_directory);
        if !data.is_absolute() || !files.is_absolute() {
            return Err(SdkError::InvalidInput {
                detail: "SDK directories must be absolute".into(),
            });
        }
        let owner = RuntimeOwner(Some(
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| SdkError::Internal {
                    detail: e.to_string(),
                })?,
        ));
        let handle = owner.0.as_ref().ok_or(SdkError::Closed)?.handle().clone();
        let inner = handle
            .spawn(initialize(
                config,
                vault,
                browser,
                Ports {
                    cloud,
                    notifications,
                    downloads,
                    web_authentication,
                    drive_tokens,
                },
            ))
            .await
            .map_err(app_core::runtime_tasks::RuntimeTaskError::from)??;
        Ok(Arc::new(Self {
            inner,
            runtime: Mutex::new(owner),
        }))
    }
    pub fn capabilities(&self) -> Result<Capabilities, SdkError> {
        if *self.inner.gate.lock().map_err(|_| SdkError::Closed)? {
            Err(SdkError::Closed)
        } else {
            Ok(self.inner.capabilities.clone())
        }
    }
    /// Subscribe first, then query; consume queued hints afterward.
    pub fn subscribe(&self) -> Result<Arc<Subscription>, SdkError> {
        let gate = self.inner.gate.lock().map_err(|_| SdkError::Closed)?;
        if *gate {
            return Err(SdkError::Closed);
        }
        self.inner.events.subscribe()
    }
    /// The revision the next mutation is made against, without the
    /// workspace; screens read their records through `records` queries.
    pub fn workspace_revision(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(|i| {
            Ok(OperationResult::Text {
                value: i.core.workspace.revision()?,
            })
        })
    }
    /// Applies one intent and answers what it changed, the next revision and
    /// the commit sequence; a shell queries again what reads the changes.
    pub fn mutate_commit(&self, mutation: WorkspaceMutation) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let commit = i.core.workspace.mutate_commit(project_input(mutation)?)?;
            Ok(OperationResult::Committed {
                commit: project(commit)?,
            })
        })
    }
    pub fn schedule_query(&self, queries: Vec<WorkspaceQuery>) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let snapshot = i.core.schedule_query(project_input(queries)?)?;
            Ok(OperationResult::Schedule {
                snapshot: ScheduleSnapshot {
                    values: project(snapshot.values)?,
                    reads: project(snapshot.reads)?,
                    seq: snapshot.seq,
                },
            })
        })
    }
    pub fn finder_query(&self, request: FinderRequest) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Finder {
                page: project(i.core.finder_query(project_input(request)?)?)?,
            })
        })
    }
    /// A lifecycle fact, network change, opened view or refresh button. Core
    /// decides what is due; this never waits for a read.
    pub fn sync_hint(&self, hint: Hint) -> Result<(), SdkError> {
        if *self.inner.gate.lock().map_err(|_| SdkError::Closed)? {
            return Err(SdkError::Closed);
        }
        self.inner.core.sync_hint(project_input(hint)?);
        Ok(())
    }
    /// The planner's current runs and sources; the same value every
    /// `sync-snapshot` event carries. Read it on start and after
    /// `StreamUpdate::SnapshotRequired`.
    pub fn sync_snapshot(&self) -> Result<Arc<Operation>, SdkError> {
        self.start(false, |i, _| async move {
            Ok(OperationResult::Sync {
                snapshot: project(i.core.sync_snapshot().await)?,
            })
        })
    }

    pub fn credential_status(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(|i| {
            Ok(OperationResult::Credentials {
                status: project(i.core.auth.status()?)?,
            })
        })
    }
    /// Settings' "Save and connect": Core keeps the entered credentials as
    /// this device's candidate, lifts any pause and signs in with them. The
    /// result carries Core's reason when the school turned them down.
    pub fn connect_school(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let result = i
                .core
                .connect_school(project_input(profile)?, project_input(credentials)?)
                .await?;
            Ok(OperationResult::SchoolConnect {
                result: project(result)?,
            })
        })
    }
    /// The saved (or drafted) school OTP setup as an `otpauth://` link for
    /// the shell to open in an authenticator or copy. Only on explicit request.
    pub fn school_otp_link(&self, draft: Option<String>) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::Text {
                value: i.core.auth.otp_setup_link(draft.as_deref())?,
            })
        })
    }
    pub fn delete_credentials(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            let profile = project_input(profile)?;
            let value = i.core.delete_credentials(profile)?;
            Ok(OperationResult::Flag { value })
        })
    }
    pub fn school_session(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(|i| {
            Ok(OperationResult::SchoolSession {
                snapshot: project(i.core.school_browser()?.session_snapshot()?)?,
            })
        })
    }
    /// Shows the site's view on the app's one web screen: at `target` as an
    /// app link, or as the person left it.
    pub fn open_school(
        &self,
        site: SchoolSite,
        target: Option<String>,
    ) -> Result<Arc<Operation>, SdkError> {
        let entry = match target {
            Some(url) => app_core::school_browser::SchoolWebEntry::Go { url },
            None => app_core::school_browser::SchoolWebEntry::Resume,
        };
        self.start(false, move |i, _| async move {
            Ok(OperationResult::SchoolWeb {
                result: project(
                    i.core
                        .school_browser()?
                        .open(project_input(site)?, WEB_SURFACE, entry)
                        .await?,
                )?,
            })
        })
    }
    /// The page the person sees of `site`. Read it after subscribing to
    /// events and keep the higher `revision` of it and `school-web-page`.
    pub fn school_web_page(&self, site: SchoolSite) -> Result<Arc<Operation>, SdkError> {
        self.blocking(move |i| {
            Ok(OperationResult::SchoolWebPage {
                page: project(i.core.school_browser()?.page(project_input(site)?))?,
            })
        })
    }
    /// The OS is about to suspend the process: Core stops leasing new keys.
    pub fn suspend(&self) -> Result<Arc<Operation>, SdkError> {
        self.start(false, |i, _| async move {
            i.core.runtime.suspend().await;
            Ok(OperationResult::Done)
        })
    }
    pub fn resume(&self) -> Result<Arc<Operation>, SdkError> {
        self.start(false, |i, _| async move {
            i.core.runtime.resume().await;
            if let Ok((sync, services)) = i.cloud() {
                sync.resume(services.clone()).await?;
            }
            Ok(OperationResult::Done)
        })
    }
    /// The shell's periodic wake (about once a second): forwards
    /// `Hint::Wake` to the sync planner, then runs reminders, cloud sync and
    /// the user's scheduled tasks.
    pub fn wake(&self) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            wake_services(&i).await?;
            Ok(OperationResult::Done)
        })
    }
    /// One wake from the OS background scheduler (iOS BGAppRefreshTask,
    /// Android WorkManager), sent while the app is out of front. Core
    /// resumes, runs one `wake`, reads what may be read in the background
    /// (LMS deadlines and assignment states over the token, and references;
    /// nothing that needs the school WebView) and waits for
    /// those reads at most `budget_seconds`. It then suspends again unless
    /// the app came to the front. Cancel it when the OS ends the task early.
    pub fn background_wake(&self, budget_seconds: u32) -> Result<Arc<Operation>, SdkError> {
        self.start(true, move |i, cancel| async move {
            let budget = std::time::Duration::from_secs(u64::from(budget_seconds));
            let work = async {
                let (woke, _) = tokio::join!(wake_services(&i), i.core.sync_in_background(budget));
                woke
            };
            let woke = tokio::select! {
                biased;
                _ = cancel.cancelled() => Ok(()),
                woke = work => woke,
            };
            i.core.end_background_wake().await;
            woke?;
            Ok(OperationResult::Done)
        })
    }
    pub fn export_backup(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(|i| {
            Ok(OperationResult::Text {
                value: i.backup.export().map_err(backup_error)?,
            })
        })
    }
    /// Import a backup file: Core writes it as this device's newest edits and
    /// uploads it to every enabled cloud, where other devices merge it.
    pub fn apply_backup(&self, document: JsonValue) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            i.sync
                .restore_document(i.sync_services.clone(), document.0)
                .await?;
            Ok(OperationResult::Done)
        })
    }
    pub fn cloud_state(&self) -> Result<Arc<Operation>, SdkError> {
        self.blocking(|i| {
            let (sync, _) = i.cloud()?;
            Ok(OperationResult::Cloud {
                state: project(sync.state()?)?,
            })
        })
    }
    pub fn sync_cloud(&self, direction: Direction) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let (sync, services) = i.cloud()?;
            Ok(OperationResult::Cloud {
                state: project(
                    sync.sync(services.clone(), project_input(direction)?)
                        .await?,
                )?,
            })
        })
    }
    /// Core signs in to Drive before turning it on (a web session on iOS,
    /// the account prompt on Android).
    pub fn set_cloud_enabled(
        &self,
        provider: CloudProvider,
        enabled: bool,
    ) -> Result<Arc<Operation>, SdkError> {
        self.start(false, move |i, _| async move {
            let (sync, services) = i.cloud()?;
            if enabled && matches!(provider, CloudProvider::GoogleDrive) && i.drive.is_none() {
                return Err(unavailable("Google Drive"));
            }
            let sign_in = crate::ports::DriveSignIn(i.drive.as_deref());
            Ok(OperationResult::Cloud {
                state: project(
                    sync.set_enabled(
                        services.clone(),
                        project_input(provider)?,
                        enabled,
                        &sign_in,
                    )
                    .await?,
                )?,
            })
        })
    }
    pub async fn shutdown(&self) -> Result<(), SdkError> {
        self.shutdown_with_timeout(std::time::Duration::from_secs(15))
            .await
    }
}
/// Decodes the payload of a `sync-snapshot` stream event, so a shell can
/// render it without another `sync_snapshot` round trip.
#[uniffi::export]
pub fn decode_sync_snapshot(payload: JsonValue) -> Result<SyncSnapshot, SdkError> {
    project_input(payload.0)
}
pub(crate) fn unavailable(name: &str) -> SdkError {
    SdkError::Unavailable {
        detail: format!("{name} is not installed"),
    }
}
fn backup_error(e: app_core::backup::BackupError) -> SdkError {
    crate::error::coded(&e)
}
fn runtime_error(e: app_core::runtime::RuntimeError) -> SdkError {
    crate::error::coded(&e)
}
struct Exporter(BackupService, PathBuf);
impl app_core::scheduler::ScheduledExporter for Exporter {
    fn export(&self) -> Result<(), app_core::scheduler::ExportError> {
        self.0
            .write_scheduled(&self.1)
            .map(|_| ())
            .map_err(|e| app_core::scheduler::ExportError::Failed(e.to_string()))
    }
}
fn services(i: &Arc<Inner>) -> Result<app_core::scheduler::SchedulerServices, SdkError> {
    Ok(i.core.services(Arc::new(Exporter(
        i.backup.clone(),
        i.backup_directory.clone(),
    ))))
}
fn ocr_available() -> bool {
    #[cfg(feature = "ocr")]
    {
        app_ocr::available()
    }
    #[cfg(not(feature = "ocr"))]
    {
        false
    }
}

/// The optional native ports a shell passed to `Session::open`.
struct Ports {
    cloud: Option<Arc<dyn NativeCloud>>,
    notifications: Option<Arc<dyn NativeNotifications>>,
    downloads: Option<Arc<dyn NativeDownloadSession>>,
    web_authentication: Option<Arc<dyn NativeWebAuthentication>>,
    drive_tokens: Option<Arc<dyn NativeDriveTokens>>,
}

async fn initialize(
    config: SessionConfig,
    vault: Arc<dyn NativeVault>,
    browser: Option<Arc<dyn NativeBrowser>>,
    ports: Ports,
) -> Result<Arc<Inner>, SdkError> {
    let Ports {
        cloud,
        notifications,
        downloads,
        web_authentication,
        drive_tokens,
    } = ports;
    let data = PathBuf::from(&config.data_directory);
    let files = PathBuf::from(&config.files_directory);
    let backup_directory = data.join("backups");
    let drive = DriveAuth::new(vault.clone(), web_authentication, drive_tokens)?.map(Arc::new);
    let capabilities = Capabilities {
        school_browser: browser.is_some(),
        downloads: downloads.is_some(),
        cloud: cloud.is_some() || drive.is_some(),
        notifications: notifications.is_some(),
        ocr: ocr_available(),
    };
    let initial_network: app_core::sync_engine::Network = project_input(config.initial_network)?;
    let native_cloud = cloud.clone();
    let store_drive = drive.clone();
    let (store, auth, sync) = tokio::task::spawn_blocking(move || {
        let (store, _) = DataStore::initialize_dir(data.clone())?;
        let auth = Arc::new(SchoolAuthService::with_status_cache(
            Vault(vault),
            data.join("school-credential-status.json"),
        ));
        // A provider runs only with its store: iCloud needs `NativeCloud`,
        // Drive a Drive port. Core reads Drive's saved authorization itself.
        let available = |provider: &CloudProvider| match provider {
            CloudProvider::Icloud => cloud.is_some(),
            CloudProvider::GoogleDrive => store_drive.is_some(),
        };
        let supported = config
            .cloud_providers
            .into_iter()
            .filter(available)
            .collect::<Vec<_>>();
        let mut authorized = config
            .authorized_cloud_providers
            .into_iter()
            .filter(|provider| {
                !matches!(provider, CloudProvider::GoogleDrive) && available(provider)
            })
            .collect::<Vec<_>>();
        if let Some(drive) = &store_drive {
            if drive.authorized()? {
                authorized.push(CloudProvider::GoogleDrive);
            }
        }
        let sync = CloudSync::new(
            data.join("cloud-sync-state.json"),
            &project_input::<_, Vec<app_core::cloud_sync::CloudProvider>>(supported)?,
            &project_input::<_, Vec<app_core::cloud_sync::CloudProvider>>(authorized)?,
        )?;
        Ok::<_, SdkError>((store, auth, sync))
    })
    .await
    .map_err(app_core::runtime_tasks::RuntimeTaskError::from)??;
    let events = Events::new();
    let core = Arc::new(Core::with_events(store, auth, events.clone()));
    core.sync_hint(app_core::sync_engine::Hint::Network {
        network: initial_network,
    });
    let browser_shutdown_error = if let Some(native) = browser {
        let browser = Arc::new(Browser::new(native));
        let errors = browser.shutdown_error.clone();
        // Without a native cookie port Core cannot read school data over
        // HTTP; the shell must pass one together with the browser.
        let cookies = downloads.clone().ok_or_else(|| {
            SdkError::from(app_core::school_browser::SchoolBrowserError::Platform(
                "A school browser needs the native cookie session.".into(),
            ))
        })?;
        core.attach_school_browser(browser, Arc::new(crate::ports::NativeCookies(cookies)))?;
        Some(errors)
    } else {
        None
    };
    let backup = BackupService::new(core.store.clone());
    core.runtime
        .attach_cloud(sync.clone())
        .map_err(runtime_error)?;
    let transport: Arc<dyn app_core::cloud_sync::CloudTransport> =
        if native_cloud.is_some() || drive.is_some() {
            Arc::new(cloud_stores(native_cloud, drive.clone())?)
        } else {
            Arc::new(NoCloud)
        };
    let sync_services = CloudServices {
        backup: backup.clone(),
        transport,
        events: events.clone(),
    };
    let inner = Arc::new(Inner {
        core,
        events,
        tasks: RuntimeTasks::default(),
        handle: Handle::current(),
        gate: Mutex::new(false),
        next_operation: AtomicU64::new(1),
        operations: Mutex::new(HashMap::new()),
        shutdown: tokio::sync::Mutex::new(false),
        closed: CancellationToken::new(),
        backup,
        sync,
        sync_services,
        drive,
        notifications,
        download_session: downloads,
        files,
        backup_directory,
        capabilities,
        browser_shutdown_error,
        #[cfg(feature = "ocr")]
        matrix: Arc::new(app_core::matrix_scan::MatrixCameraDriver::new(
            app_ocr::matrix_vision(),
        )),
    });
    inner.core.start(&Handle::current())?;
    Ok(inner)
}

/// `Hint::Wake` for the sync planner, then reminders, cloud sync and the
/// user's scheduled tasks. All services run even when one fails; the first
/// typed error is returned and snapshots expose any successful work.
async fn wake_services(i: &Arc<Inner>) -> Result<(), SdkError> {
    i.core.sync_hint(app_core::sync_engine::Hint::Wake);
    let report = i
        .core
        .wake(app_core::facade::WakeServices {
            scheduler: services(i)?,
            cloud: i.cloud().ok().map(|(_, services)| services.clone()),
            reminders: i
                .notifications
                .as_ref()
                .map(|sink| app_core::facade::ReminderWake {
                    sink: Arc::new(Notifications(sink.clone())),
                    now: chrono::Utc::now().fixed_offset(),
                }),
        })
        .await;
    report.runtime.map_err(runtime_error)?;
    if let Some(result) = report.cloud {
        result?;
    }
    if let Some(result) = report.reminders {
        result?;
    }
    Ok(())
}
