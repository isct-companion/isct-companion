//! Durable, device-owned sync queue. Network work never precedes its journal entry.
use crate::{
    cloud::{CloudProvider, DesktopCloudService},
    cloud_merge,
    data::DataStore,
    lms_files,
    secrets::OsSecretVault,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tauri::{Emitter, Manager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    pub provider: CloudProvider,
    pub enabled: bool,
    pub last_synced_at: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub synced_version: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVersion {
    pub id: String,
    pub provider: CloudProvider,
    pub saved_at: String,
    pub device: String,
    #[serde(default)]
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPoint {
    pub id: String,
    pub version: String,
    pub saved_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Pull,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub provider: CloudProvider,
    pub status: String,
    pub attempts: u32,
    pub retry_at: i64,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub direction: Direction,
    pub version: String,
    pub created_at: String,
    pub deliveries: Vec<Delivery>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub syncing: bool,
    #[serde(default)]
    pub automatic: bool,
    #[serde(default)]
    pub local_version: String,
    pub providers: Vec<ProviderState>,
    pub versions: Vec<CloudVersion>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub recovery_points: Vec<RecoveryPoint>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Job {
    task: Task,
    document: Value,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Journal {
    #[serde(flatten)]
    state: SyncState,
    #[serde(default)]
    jobs: Vec<Job>,
    #[serde(default)]
    observed: String,
    #[serde(default)]
    workspace_id: Option<String>,
    #[serde(default)]
    pending_apply: Option<Value>,
}
impl Journal {
    fn observe(&mut self, document: Value, schedule: bool) -> Result<bool, String> {
        let version = cloud_merge::version(&document)?;
        if self.observed == version {
            return Ok(false);
        }
        self.observed = version.clone();
        self.state.local_version = version;
        if schedule && self.state.automatic {
            self.enqueue(document, Direction::Upload)?;
        }
        Ok(true)
    }
    fn enqueue(&mut self, document: Value, direction: Direction) -> Result<(), String> {
        let version = cloud_merge::version(&document)?;
        let deliveries: Vec<_> = self
            .state
            .providers
            .iter()
            .filter(|p| p.enabled)
            .filter(|p| direction == Direction::Pull || p.synced_version.as_ref() != Some(&version))
            .filter(|p| {
                !self.jobs.iter().any(|j| {
                    j.task.direction == direction
                        && j.task.version == version
                        && j.task
                            .deliveries
                            .iter()
                            .any(|d| d.provider == p.provider && d.status != "completed")
                })
            })
            .map(|p| Delivery {
                provider: p.provider,
                status: "pending".into(),
                attempts: 0,
                retry_at: 0,
                error: None,
            })
            .collect();
        if !deliveries.is_empty() {
            self.jobs.push(Job {
                document,
                task: Task {
                    id: uuid::Uuid::now_v7().to_string(),
                    direction,
                    version,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    deliveries,
                },
            });
        }
        Ok(())
    }
    fn recover(&mut self) {
        self.state.syncing = false;
        for job in &mut self.jobs {
            for delivery in &mut job.task.deliveries {
                if delivery.status == "running" {
                    delivery.status = "pending".into();
                    delivery.retry_at = 0;
                }
            }
        }
    }
    fn state(&self) -> SyncState {
        let mut state = self.state.clone();
        state.tasks = self.jobs.iter().map(|j| j.task.clone()).collect();
        state
    }
}
pub struct CloudSync {
    path: PathBuf,
    journal: Mutex<Journal>,
    run: tokio::sync::Mutex<()>,
    wake: tokio::sync::Notify,
}
impl CloudSync {
    pub fn new(legacy: PathBuf, authorized: &[CloudProvider]) -> Result<Self, String> {
        let path = legacy.with_extension("sqlite");
        let connection = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
        connection.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS journal (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL)").map_err(|e| e.to_string())?;
        use rusqlite::OptionalExtension;
        let saved: Option<String> = connection
            .query_row("SELECT value FROM journal WHERE id=1", [], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        let mut journal: Journal = if let Some(saved) = saved {
            serde_json::from_str(&saved).map_err(|e| format!("Invalid sync journal: {e}"))?
        } else if legacy.exists() {
            serde_json::from_slice(&std::fs::read(&legacy).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Invalid sync configuration: {e}"))?
        } else {
            let mut providers = vec![ProviderState {
                provider: CloudProvider::GoogleDrive,
                enabled: authorized.contains(&CloudProvider::GoogleDrive),
                last_synced_at: None,
                error: None,
                synced_version: None,
            }];
            #[cfg(target_vendor = "apple")]
            providers.push(ProviderState {
                provider: CloudProvider::Icloud,
                enabled: true,
                last_synced_at: None,
                error: None,
                synced_version: None,
            });
            Journal {
                state: SyncState {
                    providers,
                    ..Default::default()
                },
                ..Default::default()
            }
        };
        journal.recover();
        journal.state.providers.retain(|p| supported(p.provider));
        // The common cross-platform channel runs first; each delivery has its own acknowledgement.
        journal
            .state
            .providers
            .sort_by_key(|p| p.provider != CloudProvider::GoogleDrive);
        let service = Self {
            path,
            journal: Mutex::new(journal),
            run: tokio::sync::Mutex::new(()),
            wake: tokio::sync::Notify::new(),
        };
        service.change(|_| Ok(()))?;
        Ok(service)
    }
    // Hold the mutex through the SQLite commit: snapshots and their intent cannot be reordered.
    fn change<T>(&self, f: impl FnOnce(&mut Journal) -> Result<T, String>) -> Result<T, String> {
        let mut guard = self
            .journal
            .lock()
            .map_err(|_| "Sync journal unavailable")?;
        let mut next = guard.clone();
        let result = f(&mut next)?;
        let connection = rusqlite::Connection::open(&self.path).map_err(|e| e.to_string())?;
        connection
            .execute_batch("PRAGMA synchronous=FULL")
            .map_err(|e| e.to_string())?;
        connection.execute("INSERT INTO journal(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&next).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        *guard = next;
        Ok(result)
    }
    pub fn state(&self) -> Result<SyncState, String> {
        Ok(self
            .journal
            .lock()
            .map_err(|_| "Sync state unavailable")?
            .state())
    }
    fn publish(&self, app: &tauri::AppHandle) -> Result<(), String> {
        app.emit("cloud-sync-state", self.state()?)
            .map_err(|e| e.to_string())
    }
    pub fn provider_result(
        &self,
        app: &tauri::AppHandle,
        provider: CloudProvider,
        enabled: bool,
        error: Option<String>,
    ) -> Result<SyncState, String> {
        self.change(|j| {
            let row = j
                .state
                .providers
                .iter_mut()
                .find(|p| p.provider == provider)
                .ok_or("Unsupported sync provider")?;
            row.enabled = enabled;
            row.error = error;
            Ok(())
        })?;
        self.publish(app)?;
        self.state()
    }
    pub async fn set_provider(
        &self,
        app: &tauri::AppHandle,
        provider: CloudProvider,
        enabled: bool,
    ) -> Result<SyncState, String> {
        if !supported(provider) {
            return Err("Unsupported sync provider".into());
        }
        let _guard = self.run.lock().await;
        if !enabled {
            app.state::<DesktopCloudService<OsSecretVault>>()
                .disconnect(provider)?;
        }
        self.change(|j| {
            if !enabled {
                j.state.versions.retain(|v| v.provider != provider);
                for job in &mut j.jobs {
                    job.task.deliveries.retain(|d| d.provider != provider);
                }
            }
            Ok(())
        })?;
        self.provider_result(app, provider, enabled, None)?;
        if enabled && self.state()?.automatic {
            let document = snapshot(app)?;
            self.change(|j| j.enqueue(document, Direction::Upload))?;
            self.wake.notify_one();
        }
        self.state()
    }
    pub fn set_automatic(
        &self,
        app: &tauri::AppHandle,
        automatic: bool,
    ) -> Result<SyncState, String> {
        self.change(|j| {
            j.state.automatic = automatic;
            j.observed.clear();
            Ok(())
        })?;
        self.capture(app, true)?;
        self.state()
    }
    /// Called after every relevant save, and on resume. Scanning at startup also
    /// recovers a save interrupted before its notification reached this service.
    pub fn capture(&self, app: &tauri::AppHandle, schedule: bool) -> Result<(), String> {
        if self
            .journal
            .lock()
            .map_err(|_| "Sync journal unavailable")?
            .pending_apply
            .is_some()
        {
            return Ok(());
        }
        let document = snapshot(app)?;
        let version = cloud_merge::version(&document)?;
        if self
            .journal
            .lock()
            .map_err(|_| "Sync journal unavailable")?
            .observed
            == version
        {
            return Ok(());
        }
        self.change(|j| j.observe(document, schedule))?;
        self.publish(app)?;
        self.wake.notify_one();
        Ok(())
    }
    pub fn changed(app: &tauri::AppHandle) {
        if let Some(sync) = app.try_state::<Self>() {
            if let Err(error) = sync.capture(app, true) {
                eprintln!("Could not journal cloud update: {error}");
            }
        }
    }
    pub fn start(app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            loop {
                let sync = app.state::<Self>();
                if let Err(error) = sync.resume(&app).await {
                    eprintln!("Cloud queue: {error}");
                }
                // Fallback for external MCP writes; unchanged data never causes network work.
                if let Err(error) = sync.capture(&app, true) {
                    eprintln!("Cloud capture: {error}");
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), sync.wake.notified()).await;
            }
        });
    }
    pub async fn sync(
        &self,
        app: &tauri::AppHandle,
        direction: Direction,
    ) -> Result<SyncState, String> {
        let document = snapshot(app)?;
        self.change(|j| {
            // Manual retry keeps the existing operation identity and saved source changes.
            for job in &mut j.jobs {
                for d in &mut job.task.deliveries {
                    d.retry_at = 0;
                }
            }
            j.enqueue(document, direction)
        })?;
        self.publish(app)?;
        self.wake.notify_one();
        self.resume(app).await
    }
    pub async fn resume(&self, app: &tauri::AppHandle) -> Result<SyncState, String> {
        let Ok(_guard) = self.run.try_lock() else {
            return self.state();
        };
        self.finish_apply(app)?;
        let jobs = self
            .journal
            .lock()
            .map_err(|_| "Sync journal unavailable")?
            .jobs
            .clone();
        if !jobs.iter().any(|j| {
            j.task
                .deliveries
                .iter()
                .any(|d| d.status != "completed" && d.retry_at <= chrono::Utc::now().timestamp())
        }) {
            return self.state();
        }
        for mut job in jobs {
            if !job
                .task
                .deliveries
                .iter()
                .any(|d| d.status != "completed" && d.retry_at <= chrono::Utc::now().timestamp())
            {
                continue;
            }
            self.change(|j| {
                j.state.syncing = true;
                Ok(())
            })?;
            self.publish(app)?;
            if job.task.direction == Direction::Upload {
                // Reconcile reachable replicas before publishing a manual or automatic upload.
                // Failure of one provider leaves its own delivery pending.
                for provider in self.state()?.providers.into_iter().filter(|p| p.enabled) {
                    let transport = app.state::<DesktopCloudService<OsSecretVault>>();
                    if let Ok(Ok(Some(remote))) = tokio::time::timeout(
                        Duration::from_secs(45),
                        transport.read_workspace(provider.provider),
                    )
                    .await
                    {
                        let remote = remote.merged_document();
                        if self.bind_workspace(&remote).is_ok() {
                            job.document = cloud_merge::merge(&job.document, &remote);
                        }
                    }
                }
                job.task.version = cloud_merge::version(&job.document)?;
                self.change(|j| {
                    let saved = j
                        .jobs
                        .iter_mut()
                        .find(|saved| saved.task.id == job.task.id)
                        .ok_or("Sync task missing")?;
                    saved.document = job.document.clone();
                    saved.task.version = job.task.version.clone();
                    Ok(())
                })?;
            }
            let mut convergence = job.document.clone();
            let mut delivered = false;
            for delivery in &job.task.deliveries {
                if delivery.status == "completed"
                    || delivery.retry_at > chrono::Utc::now().timestamp()
                {
                    continue;
                }
                if !self
                    .state()?
                    .providers
                    .iter()
                    .any(|p| p.provider == delivery.provider && p.enabled)
                {
                    continue;
                }
                self.change(|j| {
                    j.state.syncing = true;
                    let d = delivery_mut(j, &job.task.id, delivery.provider)?;
                    d.status = "running".into();
                    d.attempts += 1;
                    d.error = None;
                    Ok(())
                })?;
                self.publish(app)?;
                let result = tokio::time::timeout(
                    Duration::from_secs(45),
                    self.deliver(app, &job, delivery.provider),
                )
                .await
                .unwrap_or_else(|_| {
                    Err("Sync interrupted or timed out; the saved task will retry.".into())
                });
                if let Ok((document, _)) = &result {
                    delivered = true;
                    convergence = cloud_merge::merge(&convergence, document);
                }
                self.change(|j| {
                    j.state.syncing = false;
                    let d = delivery_mut(j, &job.task.id, delivery.provider)?;
                    match &result {
                        Ok(_) => {
                            d.status = "completed".into();
                            d.error = None;
                        }
                        Err(error) => {
                            d.status = "pending".into();
                            d.error = Some(error.clone());
                            d.retry_at = chrono::Utc::now().timestamp()
                                + (30_i64 * 2_i64.pow(d.attempts.min(4) - 1)).min(300);
                        }
                    }
                    let p = j
                        .state
                        .providers
                        .iter_mut()
                        .find(|p| p.provider == delivery.provider)
                        .ok_or("Unsupported provider")?;
                    match result {
                        Ok((document, versions)) => {
                            p.error = None;
                            p.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
                            p.synced_version = Some(cloud_merge::version(&document)?);
                            j.state.versions.retain(|v| v.provider != delivery.provider);
                            j.state.versions.extend(versions);
                        }
                        Err(error) => p.error = Some(error),
                    }
                    Ok(())
                })?;
                self.publish(app)?;
            }
            if delivered && job.task.direction == Direction::Upload {
                // Finish the explicitly requested upload even in manual mode when a
                // concurrent remote edit was discovered halfway through the run.
                self.change(|j| j.enqueue(convergence, Direction::Upload))?;
                if self.state()?.automatic {
                    let latest = snapshot(app)?;
                    self.change(|j| j.enqueue(latest, Direction::Upload))?;
                }
                self.wake.notify_one();
            }
        }
        self.change(|j| {
            j.state.syncing = false;
            // Keep the last ten completed task receipts, plus every unfinished task.
            let mut completed = 0;
            j.jobs.reverse();
            j.jobs.retain(|job| {
                if job.task.deliveries.iter().all(|d| d.status == "completed") {
                    completed += 1;
                    completed <= 10
                } else {
                    true
                }
            });
            j.jobs.reverse();
            j.state.versions.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
            Ok(())
        })?;
        self.publish(app)?;
        self.state()
    }
    fn bind_workspace(&self, document: &Value) -> Result<(), String> {
        let remote_id = document
            .pointer("/sync/workspaceId")
            .and_then(Value::as_str);
        self.change(|j| {
            if let (Some(local), Some(remote)) = (&j.workspace_id, remote_id) {
                if local != remote { return Err("This cloud account contains a different workspace. Disconnect it before continuing.".into()); }
            }
            if j.workspace_id.is_none() { j.workspace_id = remote_id.map(str::to_owned); }
            Ok(())
        })
    }
    async fn deliver(
        &self,
        app: &tauri::AppHandle,
        job: &Job,
        provider: CloudProvider,
    ) -> Result<(Value, Vec<CloudVersion>), String> {
        let transport = app.state::<DesktopCloudService<OsSecretVault>>();
        #[cfg(target_os = "android")]
        if provider == CloudProvider::GoogleDrive {
            transport.refresh_android(app)?;
        }
        let remote = transport.read_workspace(provider).await?;
        let remote_doc = remote.as_ref().map(|r| r.merged_document());
        let workspace_id = self.change(|j| {
            let remote_id = remote_doc.as_ref().and_then(|r| r.pointer("/sync/workspaceId")).and_then(Value::as_str);
            if let (Some(local), Some(remote)) = (&j.workspace_id, remote_id) {
                if local != remote { return Err("This cloud account contains a different workspace. Disconnect it before continuing.".into()); }
            }
            let id = j.workspace_id.get_or_insert_with(|| remote_id.map(str::to_owned).unwrap_or_else(|| uuid::Uuid::now_v7().to_string()));
            Ok(id.clone())
        })?;
        let mut document = job.document.clone();
        document["sync"]["workspaceId"] = json!(workspace_id);
        let document = if job.task.direction == Direction::Upload {
            let device = app.state::<DataStore>().load_workspace()?["deviceId"]
                .as_str()
                .ok_or("Device id missing")?
                .to_owned();
            transport
                .merge_and_push(provider, &document, &device)
                .await?
        } else {
            let Some(remote) = remote_doc else {
                return Err("No cloud copy exists yet.".into());
            };
            remote
        };
        // Preserve a local recovery copy before importing remote changes.
        let current = snapshot(app)?;
        if cloud_merge::contents(&current) != cloud_merge::contents(&document) {
            self.checkpoint(&current)?;
        }
        self.apply_document(app, &document)?;
        // Pull is read-only remotely, including when Auto is enabled.
        self.capture(app, false)?;
        app.emit("workspace-changed", json!({"source":"cloud-sync"}))
            .map_err(|e| e.to_string())?;
        Ok((document, transport.versions(provider).await?))
    }
    fn apply_document(&self, app: &tauri::AppHandle, document: &Value) -> Result<(), String> {
        validate_document(document)?;
        self.change(|j| {
            j.pending_apply = Some(document.clone());
            Ok(())
        })?;
        self.finish_apply(app)
    }
    fn finish_apply(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let pending = self
            .journal
            .lock()
            .map_err(|_| "Sync journal unavailable")?
            .pending_apply
            .clone();
        if let Some(document) = pending {
            apply_document(app, &document)?;
            self.change(|j| {
                j.pending_apply = None;
                Ok(())
            })?;
        }
        Ok(())
    }
    fn checkpoint(&self, document: &Value) -> Result<(), String> {
        let directory = self.path.with_file_name("recovery");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!("{}.json", cloud_merge::version(document)?));
        // Content-addressed copies do not rotate when another provider echoes the same data.
        if !path.exists() {
            use std::io::Write;
            let temporary = path.with_extension("tmp");
            let mut file = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
            file.write_all(document.to_string().as_bytes())
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(temporary, path).map_err(|e| e.to_string())?;
        }
        let version = cloud_merge::version(document)?;
        self.change(|j| {
            if !j.state.recovery_points.iter().any(|p| p.version == version) {
                j.state.recovery_points.insert(
                    0,
                    RecoveryPoint {
                        id: format!("local:{version}"),
                        version,
                        saved_at: chrono::Utc::now().to_rfc3339(),
                    },
                );
            }
            Ok(())
        })
    }
    pub fn recovery_document(&self, id: &str) -> Result<String, String> {
        let point = self
            .state()?
            .recovery_points
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("Recovery point not found")?;
        if !point.version.starts_with("v1-")
            || point.version.len() != 67
            || !point.version[3..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Invalid recovery version".into());
        }
        std::fs::read_to_string(
            self.path
                .with_file_name("recovery")
                .join(format!("{}.json", point.version)),
        )
        .map_err(|e| e.to_string())
    }
    pub async fn prepare_reset(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let _guard = self.run.lock().await;
        self.finish_apply(app)?;
        self.checkpoint(&snapshot(app)?)?;
        self.change(|j| {
            j.state.automatic = false;
            j.state.syncing = false;
            j.jobs.clear();
            j.state.versions.clear();
            j.observed.clear();
            j.workspace_id = None;
            for p in &mut j.state.providers {
                p.enabled = false;
                p.synced_version = None;
                p.error = None;
                p.last_synced_at = None;
            }
            Ok(())
        })?;
        self.publish(app)
    }
    pub async fn restore(&self, app: &tauri::AppHandle, id: &str) -> Result<SyncState, String> {
        let transport = app.state::<DesktopCloudService<OsSecretVault>>();
        let old = if id.starts_with("local:") {
            serde_json::from_str(&self.recovery_document(id)?)
                .map_err(|e| format!("Invalid recovery file: {e}"))?
        } else {
            let version = self
                .state()?
                .versions
                .into_iter()
                .find(|v| v.id == id)
                .ok_or("Cloud version no longer exists")?;
            let revision = id
                .rsplit(':')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or("Invalid cloud version")?;
            transport
                .version_document(version.provider, revision)
                .await?
        };
        self.restore_document(app, &old).await
    }
    pub async fn restore_document(
        &self,
        app: &tauri::AppHandle,
        old: &Value,
    ) -> Result<SyncState, String> {
        validate_document(old)?;
        let guard = self.run.lock().await;
        self.finish_apply(app)?;
        if self
            .state()?
            .tasks
            .iter()
            .any(|t| t.deliveries.iter().any(|d| d.status != "completed"))
        {
            return Err("Finish pending sync tasks or disconnect the unavailable provider before restoring.".into());
        }
        let transport = app.state::<DesktopCloudService<OsSecretVault>>();
        let current = snapshot(app)?;
        self.checkpoint(&current)?;
        let mut restored = cloud_merge::merge(old, old);
        // A restore is a new generation: stale devices cannot resurrect records absent here.
        restored["sync"] = current["sync"].clone();
        let mut generation = cloud_merge::generation(&current).0;
        for p in self.state()?.providers.into_iter().filter(|p| p.enabled) {
            if let Some(remote) = transport.read_workspace(p.provider).await? {
                self.bind_workspace(&remote.merged_document())?;
                generation = generation.max(cloud_merge::generation(&remote.merged_document()).0);
            }
        }
        restored["sync"]["generation"] =
            json!({"counter":generation+1,"id":uuid::Uuid::now_v7().to_string()});
        // Journal intent before changing local storage; restart replays this exact generation.
        self.change(|j| j.enqueue(restored.clone(), Direction::Upload))?;
        self.apply_document(app, &restored)?;
        self.capture(app, false)?;
        app.emit("workspace-changed", json!({"source":"cloud-restore"}))
            .map_err(|e| e.to_string())?;
        self.publish(app)?;
        drop(guard);
        self.resume(app).await
    }
}
fn delivery_mut<'a>(
    j: &'a mut Journal,
    id: &str,
    provider: CloudProvider,
) -> Result<&'a mut Delivery, String> {
    j.jobs
        .iter_mut()
        .find(|job| job.task.id == id)
        .and_then(|job| {
            job.task
                .deliveries
                .iter_mut()
                .find(|d| d.provider == provider)
        })
        .ok_or("Sync task missing".into())
}
fn supported(provider: CloudProvider) -> bool {
    match provider {
        CloudProvider::GoogleDrive => true,
        #[cfg(target_vendor = "apple")]
        CloudProvider::Icloud => true,
        _ => false,
    }
}
fn snapshot(app: &tauri::AppHandle) -> Result<Value, String> {
    let notes = serde_json::to_value(lms_files::export_notes(app)?).map_err(|e| e.to_string())?;
    serde_json::from_str(&app.state::<DataStore>().export_data(notes)?).map_err(|e| e.to_string())
}
fn validate_document(document: &Value) -> Result<(), String> {
    crate::data::validate_sync_document(document)?;
    let notes: Vec<lms_files::LocalNoteBackup> =
        serde_json::from_value(document["localNotes"].clone()).map_err(|e| e.to_string())?;
    lms_files::validate_notes(&notes)
}
fn apply_document(app: &tauri::AppHandle, document: &Value) -> Result<(), String> {
    let local = snapshot(app)?;
    let merged = cloud_merge::merge(&local, document);
    let notes: Vec<lms_files::LocalNoteBackup> =
        serde_json::from_value(merged["localNotes"].clone()).map_err(|e| e.to_string())?;
    if cloud_merge::generation(&merged) > cloud_merge::generation(&local) {
        lms_files::restore_notes(app, &notes)?;
    } else {
        lms_files::merge_notes(app, &notes)?;
    }
    app.state::<DataStore>().merge_cloud_data(&merged)
}

#[cfg(test)]
mod tests {
    use super::*;
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
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn document(n: u64) -> Value {
        json!({"tables":{"plans":[{"id":"p","rev":n}]},"localNotes":[]})
    }
    #[test]
    fn manual_is_quiet_auto_queues_each_save_and_pull_does_not_echo() {
        let mut j = journal();
        j.observe(document(1), true).unwrap();
        assert!(j.jobs.is_empty());
        j.state.automatic = true;
        j.observe(document(2), true).unwrap();
        j.observe(document(3), true).unwrap();
        j.observe(document(3), true).unwrap();
        assert_eq!(j.jobs.len(), 2);
        j.observe(document(4), false).unwrap();
        assert_eq!(j.jobs.len(), 2);
        assert_eq!(
            j.state.local_version,
            cloud_merge::version(&document(4)).unwrap()
        );
    }
    #[test]
    fn queue_retains_every_change_and_retries_the_same_task() {
        let mut j = journal();
        j.enqueue(document(1), Direction::Upload).unwrap();
        j.enqueue(document(2), Direction::Upload).unwrap();
        j.enqueue(document(2), Direction::Upload).unwrap();
        assert_eq!(j.jobs.len(), 2);
        let id = j.jobs[0].task.id.clone();
        j.jobs[0].task.deliveries[0].status = "running".into();
        let mut restored: Journal =
            serde_json::from_str(&serde_json::to_string(&j).unwrap()).unwrap();
        restored.recover();
        assert_eq!(restored.jobs[0].task.id, id);
        assert_eq!(restored.jobs[0].task.deliveries[0].status, "pending");
        assert_eq!(restored.jobs[0].document, document(1));
    }
    #[test]
    fn acknowledgements_are_independent_and_pull_remains_explicit() {
        let mut j = journal();
        let mut p = j.state.providers[0].clone();
        p.provider = CloudProvider::Icloud;
        j.state.providers.push(p);
        j.state.providers[0].synced_version = Some(cloud_merge::version(&document(1)).unwrap());
        j.enqueue(document(1), Direction::Upload).unwrap();
        assert_eq!(j.jobs[0].task.deliveries.len(), 1);
        assert_eq!(j.jobs[0].task.deliveries[0].provider, CloudProvider::Icloud);
        j.enqueue(document(1), Direction::Pull).unwrap();
        assert_eq!(j.jobs[1].task.deliveries.len(), 2);
        assert!(!j.state.automatic);
    }
    #[test]
    fn journal_commit_survives_restart() {
        let legacy = std::env::temp_dir().join(format!("sync-{}.json", uuid::Uuid::new_v4()));
        let sync = CloudSync::new(legacy.clone(), &[CloudProvider::GoogleDrive]).unwrap();
        sync.change(|j| {
            j.state.automatic = true;
            j.pending_apply = Some(document(2));
            j.enqueue(document(1), Direction::Upload)?;
            j.jobs[0].task.deliveries[0].status = "running".into();
            j.jobs[0].task.deliveries[0].attempts = 1;
            Ok(())
        })
        .unwrap();
        let id = sync.state().unwrap().tasks[0].id.clone();
        let restarted = CloudSync::new(legacy.clone(), &[]).unwrap();
        assert!(restarted.state().unwrap().automatic);
        assert_eq!(restarted.state().unwrap().tasks[0].id, id);
        assert_eq!(
            restarted.state().unwrap().tasks[0].deliveries[0].status,
            "pending"
        );
        assert_eq!(
            restarted.state().unwrap().tasks[0].deliveries[0].attempts,
            1
        );
        assert_eq!(
            restarted.journal.lock().unwrap().pending_apply,
            Some(document(2))
        );
        std::fs::remove_file(legacy.with_extension("sqlite")).unwrap();
    }
}
