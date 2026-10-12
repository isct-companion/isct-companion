//! Device-owned automatic sync orchestration. All providers share one merge.
use crate::{
    cloud::{CloudProvider, DesktopCloudService},
    cloud_merge,
    data::DataStore,
    lms_files,
    secrets::OsSecretVault,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    pub provider: CloudProvider,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVersion {
    pub id: String,
    pub provider: CloudProvider,
    pub saved_at: String,
    pub device: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub syncing: bool,
    pub providers: Vec<ProviderState>,
    pub versions: Vec<CloudVersion>,
}
struct Runtime {
    state: SyncState,
    observed: Option<Value>,
    changed: Option<Instant>,
    last_run: Option<Instant>,
}
impl Runtime {
    fn observe(&mut self, content: Value, now: Instant) {
        if self.observed.as_ref() != Some(&content) {
            self.observed = Some(content);
            self.changed = Some(now);
        }
    }
    fn due(&self, now: Instant) -> bool {
        !self.state.syncing
            && (self
                .changed
                .is_some_and(|at| now.duration_since(at) >= Duration::from_secs(30))
                || self
                    .last_run
                    .is_none_or(|at| now.duration_since(at) >= Duration::from_secs(300)))
    }
}
pub struct CloudSync {
    path: PathBuf,
    runtime: Mutex<Runtime>,
    run: tokio::sync::Mutex<()>,
    persistence: Mutex<()>,
}
impl CloudSync {
    pub fn new(path: PathBuf, authorized: &[CloudProvider]) -> Result<Self, String> {
        let mut state: SyncState = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Invalid cloud sync configuration: {e}"))?
        } else {
            let mut providers = vec![ProviderState {
                provider: CloudProvider::GoogleDrive,
                enabled: authorized.contains(&CloudProvider::GoogleDrive),
                last_synced_at: None,
                error: None,
            }];
            #[cfg(target_vendor = "apple")]
            providers.insert(
                0,
                ProviderState {
                    provider: CloudProvider::Icloud,
                    enabled: true,
                    last_synced_at: None,
                    error: None,
                },
            );
            SyncState {
                providers,
                ..Default::default()
            }
        };
        state.syncing = false;
        state.providers.retain(|p| supported(p.provider));
        let service = Self {
            path,
            runtime: Mutex::new(Runtime {
                state,
                observed: None,
                changed: None,
                last_run: None,
            }),
            run: tokio::sync::Mutex::new(()),
            persistence: Mutex::new(()),
        };
        service.persist()?;
        Ok(service)
    }
    fn persist(&self) -> Result<(), String> {
        let _guard = self
            .persistence
            .lock()
            .map_err(|_| "Sync configuration unavailable")?;
        let state = self.state()?;
        let temp = self.path.with_extension("tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec(&state).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temp, &self.path).map_err(|e| e.to_string())
    }
    pub fn state(&self) -> Result<SyncState, String> {
        Ok(self
            .runtime
            .lock()
            .map_err(|_| "Sync state unavailable")?
            .state
            .clone())
    }
    fn publish(&self, app: &tauri::AppHandle) -> Result<(), String> {
        self.persist()?;
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
        {
            let mut runtime = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
            let row = runtime
                .state
                .providers
                .iter_mut()
                .find(|p| p.provider == provider)
                .ok_or("Unsupported sync provider")?;
            row.enabled = enabled;
            row.error = error;
        }
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
        let transport = app.state::<DesktopCloudService<OsSecretVault>>();
        if !enabled {
            transport.disconnect(provider)?;
        }
        // OAuth is performed by the command before enabling Drive.
        self.provider_result(app, provider, enabled, None)
    }
    pub fn start(app: tauri::AppHandle) {
        tauri::async_runtime::spawn(async move {
            let _ = app.state::<CloudSync>().sync(&app).await;
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let sync = app.state::<CloudSync>();
                match snapshot(&app) {
                    Ok(document) => {
                        let due = {
                            let mut r = match sync.runtime.lock() {
                                Ok(r) => r,
                                Err(_) => continue,
                            };
                            let content = cloud_merge::contents(&document);
                            let now = Instant::now();
                            r.observe(content, now);
                            r.due(now)
                        };
                        if due {
                            let _ = sync.sync(&app).await;
                        }
                    }
                    Err(_) => (),
                }
            }
        });
    }
    pub async fn sync(&self, app: &tauri::AppHandle) -> Result<SyncState, String> {
        // An overlapping request waits for the existing run's published state.
        let guard = match self.run.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                let guard = self.run.lock().await;
                let current = cloud_merge::contents(&snapshot(app)?);
                let unchanged = {
                    let runtime = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
                    runtime.observed.as_ref() == Some(&current)
                };
                if unchanged {
                    return self.state();
                }
                guard
            }
        };
        self.runtime
            .lock()
            .map_err(|_| "Sync state unavailable")?
            .state
            .syncing = true;
        let _ = self.publish(app);
        let result = self.sync_inner(app).await;
        {
            let mut r = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
            r.state.syncing = false;
            r.last_run = Some(Instant::now());
            if let Err(error) = &result {
                for p in &mut r.state.providers {
                    if p.enabled {
                        p.error = Some(error.clone());
                    }
                }
            }
        }
        let published = self.publish(app);
        drop(guard);
        published?;
        self.state()
    }
    async fn sync_inner(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let transport = app.state::<DesktopCloudService<OsSecretVault>>();
        let enabled: Vec<_> = self
            .state()?
            .providers
            .into_iter()
            .filter(|p| p.enabled)
            .map(|p| p.provider)
            .collect();
        let mut merged = snapshot(app)?;
        let mut ready = Vec::new();
        for provider in enabled {
            #[cfg(target_os = "android")]
            if provider == CloudProvider::GoogleDrive {
                if let Err(error) = transport.refresh_android(app) {
                    self.provider_result(app, provider, true, Some(error))?;
                    continue;
                }
            }
            match transport.read_workspace(provider).await {
                Ok(remote) => {
                    if let Some(remote) = remote {
                        merged = cloud_merge::merge(&merged, &remote.merged_document());
                    }
                    ready.push(provider);
                }
                Err(error) => {
                    self.provider_result(app, provider, true, Some(error))?;
                }
            }
        }
        // Include changes made locally while cloud reads were in flight.
        merged = cloud_merge::merge(&snapshot(app)?, &merged);
        let store = app.state::<DataStore>();
        store.merge_cloud_data(&merged)?;
        merge_notes(app, &merged)?;
        let document = snapshot(app)?;
        let device = store.load_workspace()?["deviceId"]
            .as_str()
            .ok_or("Device id missing")?
            .to_owned();
        for provider in ready {
            let outcome = async {
                let pushed = transport
                    .merge_and_push(provider, &document, &device)
                    .await?;
                store.merge_cloud_data(&pushed)?;
                merge_notes(app, &pushed)?;
                if provider == CloudProvider::GoogleDrive
                    && transport.credential_sync_configured()?
                {
                    let auth = app.state::<crate::SchoolAuthService<OsSecretVault>>();
                    transport
                        .back_up_credentials(provider, &auth.export_sync_payload()?)
                        .await?;
                }
                transport.versions(provider).await
            }
            .await;
            let mut r = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
            let p = r
                .state
                .providers
                .iter_mut()
                .find(|p| p.provider == provider)
                .unwrap();
            match outcome {
                Ok(versions) => {
                    p.error = None;
                    p.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
                    r.state.versions.retain(|v| v.provider != provider);
                    r.state.versions.extend(versions);
                }
                Err(error) => p.error = Some(error),
            }
        }
        {
            let mut r = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
            r.state.versions.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
            r.observed = Some(cloud_merge::contents(&document));
            r.changed = None;
        }
        app.emit(
            "workspace-changed",
            serde_json::json!({"source":"cloud-sync"}),
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub async fn restore(&self, app: &tauri::AppHandle, id: &str) -> Result<SyncState, String> {
        let _guard = self.run.lock().await;
        self.runtime
            .lock()
            .map_err(|_| "Sync state unavailable")?
            .state
            .syncing = true;
        let _ = self.publish(app);
        let result = self.restore_inner(app, id).await;
        {
            let mut r = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
            r.state.syncing = false;
            if let Err(error) = result {
                for p in &mut r.state.providers {
                    if p.enabled {
                        p.error = Some(error.clone());
                    }
                }
            }
        }
        self.publish(app)?;
        self.state()
    }
    async fn restore_inner(&self, app: &tauri::AppHandle, id: &str) -> Result<SyncState, String> {
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
        let transport = app.state::<DesktopCloudService<OsSecretVault>>();
        let old = transport
            .version_document(version.provider, revision)
            .await?;
        let current = snapshot(app)?;
        let device = app.state::<DataStore>().load_workspace()?["deviceId"]
            .as_str()
            .ok_or("Device id missing")?
            .to_owned();
        let restored = restore_document(&current, &old, &device);
        app.state::<DataStore>().merge_cloud_data(&restored)?;
        merge_notes(app, &restored)?;
        // Push directly: re-merging the replaced cloud copy here would undo
        // rollback. The restored rows carry fresh clocks for all later runs.
        for provider in self
            .state()?
            .providers
            .into_iter()
            .filter(|p| p.enabled)
            .map(|p| p.provider)
        {
            let result = transport
                .back_up_workspace(provider, &restored.to_string(), &device, true)
                .await;
            self.provider_result(app, provider, true, result.err())?;
            if let Ok(versions) = transport.versions(provider).await {
                let mut r = self.runtime.lock().map_err(|_| "Sync state unavailable")?;
                r.state.versions.retain(|v| v.provider != provider);
                r.state.versions.extend(versions);
            }
        }
        app.emit(
            "workspace-changed",
            serde_json::json!({"source":"cloud-restore"}),
        )
        .map_err(|e| e.to_string())?;
        self.publish(app)?;
        self.state()
    }
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
fn merge_notes(app: &tauri::AppHandle, document: &Value) -> Result<(), String> {
    let local = snapshot(app)?;
    let merged = cloud_merge::merge(&local, document);
    let notes: Vec<lms_files::LocalNoteBackup> =
        serde_json::from_value(merged["localNotes"].clone()).map_err(|e| e.to_string())?;
    lms_files::merge_notes(app, &notes)
}
fn restore_document(current: &Value, old: &Value, device: &str) -> Value {
    let mut restored = cloud_merge::merge(current, old);
    let at = chrono::Utc::now().to_rfc3339();
    for (name, rows) in restored["tables"].as_object_mut().unwrap() {
        let key = if name == "settings" { "key" } else { "id" };
        for row in rows.as_array_mut().unwrap() {
            if let Some(prior) = old["tables"][name]
                .as_array()
                .into_iter()
                .flatten()
                .find(|v| v[key] == row[key])
            {
                *row = prior.clone();
            } else {
                row["deletedAt"] = Value::String(at.clone());
            }
            row["updatedAt"] = Value::String(at.clone());
            row["updatedBy"] = Value::String(device.into());
            row["rev"] = Value::from(row["rev"].as_u64().unwrap_or(0) + 1);
        }
    }
    for row in restored["localNotes"].as_array_mut().unwrap() {
        if let Some(prior) = old["localNotes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| v["id"] == row["id"])
        {
            *row = prior.clone();
        } else {
            row["deleted"] = Value::Bool(true);
            row["deletedAt"] = Value::String(at.clone());
        }
        row["updatedAt"] = Value::String(at.clone());
        row["updatedBy"] = Value::String(device.into());
    }
    restored
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn debounce_follows_the_last_change_and_folds_inflight_requests() {
        let now = Instant::now();
        let mut r = Runtime {
            state: SyncState::default(),
            observed: None,
            changed: None,
            last_run: Some(now),
        };
        r.observe(json!({"record":1}), now);
        assert!(!r.due(now + Duration::from_secs(29)));
        r.observe(json!({"record":2}), now + Duration::from_secs(20));
        assert!(!r.due(now + Duration::from_secs(49)));
        assert!(r.due(now + Duration::from_secs(50)));
        r.state.syncing = true;
        assert!(!r.due(now + Duration::from_secs(300)));
        r.state.syncing = false;
        r.changed = None;
        assert!(r.due(now + Duration::from_secs(300)));
    }

    #[test]
    fn restoring_reverts_edits_and_tombstones_later_additions() {
        let old = json!({"tables":{"plans":[{"id":"a","title":"old","updatedAt":"2026-01-01T00:00:00Z","rev":1,"updatedBy":"a"}]},"localNotes":[]});
        let current = json!({"tables":{"plans":[{"id":"a","title":"new","updatedAt":"2026-01-02T00:00:00Z","rev":2,"updatedBy":"b"},{"id":"b","title":"later","updatedAt":"2026-01-02T00:00:00Z","rev":1,"updatedBy":"b"}]},"localNotes":[]});
        let restored = restore_document(&current, &old, "device-c");
        assert_eq!(restored["tables"]["plans"][0]["title"], "old");
        assert!(restored["tables"]["plans"][1]["deletedAt"].is_string());
        assert_eq!(
            cloud_merge::contents(&restored),
            cloud_merge::contents(&cloud_merge::merge(&current, &restored))
        );
    }
    #[test]
    fn provider_configuration_is_local_and_survives_restart() {
        let path = std::env::temp_dir().join(format!("sync-config-{}.json", uuid::Uuid::new_v4()));
        let sync = CloudSync::new(path.clone(), &[CloudProvider::GoogleDrive]).unwrap();
        assert!(
            sync.state()
                .unwrap()
                .providers
                .iter()
                .find(|p| p.provider == CloudProvider::GoogleDrive)
                .unwrap()
                .enabled
        );
        let next = CloudSync::new(path.clone(), &[]).unwrap();
        assert!(
            next.state()
                .unwrap()
                .providers
                .iter()
                .find(|p| p.provider == CloudProvider::GoogleDrive)
                .unwrap()
                .enabled
        );
        std::fs::remove_file(path).unwrap();
    }
}
