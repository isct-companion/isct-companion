//! Native cloud ports; queue, recovery and retry policy live in Core.
use crate::{cloud::DesktopCloudService, secrets::OsSecretVault};
use isct_core::cloud_sync::{
    CloudFuture, CloudObserver, CloudServices, CloudSyncError, CloudTransport,
};
pub use isct_core::cloud_sync::{CloudProvider, CloudVersion, Direction, SyncState};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Emitter, Manager};

struct NativeCloud(AppHandle);
impl CloudTransport for NativeCloud {
    fn disconnect(&self, provider: CloudProvider) -> Result<(), CloudSyncError> {
        self.0
            .state::<DesktopCloudService<OsSecretVault>>()
            .disconnect(provider)
            .map_err(CloudSyncError::Transport)
    }
    fn read_workspace<'a>(&'a self, provider: CloudProvider) -> CloudFuture<'a, Option<Value>> {
        Box::pin(async move {
            let transport = self.0.state::<DesktopCloudService<OsSecretVault>>();
            #[cfg(target_os = "android")]
            if provider == CloudProvider::GoogleDrive {
                transport
                    .refresh_android(&self.0)
                    .map_err(CloudSyncError::Transport)?;
            }
            transport
                .read_workspace(provider)
                .await
                .map(|value| value.map(|file| file.merged_document()))
                .map_err(CloudSyncError::Transport)
        })
    }
    fn merge_and_push<'a>(
        &'a self,
        provider: CloudProvider,
        document: &'a Value,
        device: &'a str,
    ) -> CloudFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .state::<DesktopCloudService<OsSecretVault>>()
                .merge_and_push(provider, document, device)
                .await
                .map_err(CloudSyncError::Transport)
        })
    }
    fn versions<'a>(&'a self, provider: CloudProvider) -> CloudFuture<'a, Vec<CloudVersion>> {
        Box::pin(async move {
            self.0
                .state::<DesktopCloudService<OsSecretVault>>()
                .versions(provider)
                .await
                .map_err(CloudSyncError::Transport)
        })
    }
    fn version_document<'a>(
        &'a self,
        provider: CloudProvider,
        revision: u64,
    ) -> CloudFuture<'a, Value> {
        Box::pin(async move {
            self.0
                .state::<DesktopCloudService<OsSecretVault>>()
                .version_document(provider, revision)
                .await
                .map_err(CloudSyncError::Transport)
        })
    }
}
struct NativeObserver(AppHandle);
impl CloudObserver for NativeObserver {
    fn state_changed(&self, state: SyncState) -> Result<(), CloudSyncError> {
        self.0
            .emit("cloud-sync-state", state)
            .map_err(|error| CloudSyncError::Event(error.to_string()))
    }
    fn workspace_changed(&self, source: &'static str) -> Result<(), CloudSyncError> {
        self.0
            .emit("workspace-changed", serde_json::json!({"source":source}))
            .map_err(|error| CloudSyncError::Event(error.to_string()))
    }
}
fn services(app: &AppHandle) -> Result<CloudServices, String> {
    Ok(CloudServices {
        backup: crate::commands::backup::service(app)?,
        transport: Arc::new(NativeCloud(app.clone())),
        observer: Arc::new(NativeObserver(app.clone())),
    })
}

pub struct CloudSync(pub(crate) isct_core::cloud_sync::CloudSync);
impl CloudSync {
    pub fn new(legacy: PathBuf, authorized: &[CloudProvider]) -> Result<Self, String> {
        let supported = [
            CloudProvider::GoogleDrive,
            #[cfg(target_vendor = "apple")]
            CloudProvider::Icloud,
        ];
        isct_core::cloud_sync::CloudSync::new(legacy, &supported, authorized)
            .map(Self)
            .map_err(|error| error.to_string())
    }
    pub fn state(&self) -> Result<SyncState, String> {
        self.0.state().map_err(|error| error.to_string())
    }
    pub fn recovery_document(&self, id: &str) -> Result<String, String> {
        self.0
            .recovery_document(id)
            .map_err(|error| error.to_string())
    }
    pub async fn changed(app: &AppHandle) {
        if let Some(sync) = app.try_state::<Self>() {
            let result = match services(app) {
                Ok(ports) => sync
                    .0
                    .changed(ports)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                eprintln!("Could not journal cloud update: {error}");
            }
        }
    }
    pub fn tick(app: &AppHandle) {
        if let Some(sync) = app.try_state::<Self>() {
            if let Err(error) = services(app)
                .and_then(|ports| sync.0.tick(ports).map_err(|error| error.to_string()))
            {
                eprintln!("Cloud queue: {error}");
            }
        }
    }
    pub async fn provider_result(
        &self,
        app: &AppHandle,
        provider: CloudProvider,
        enabled: bool,
        error: Option<String>,
    ) -> Result<SyncState, String> {
        self.0
            .provider_result(services(app)?, provider, enabled, error)
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn set_provider(
        &self,
        app: &AppHandle,
        provider: CloudProvider,
        enabled: bool,
    ) -> Result<SyncState, String> {
        self.0
            .set_enabled(services(app)?, provider, enabled)
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn set_automatic(
        &self,
        app: &AppHandle,
        automatic: bool,
    ) -> Result<SyncState, String> {
        self.0
            .set_automatic(services(app)?, automatic)
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn sync(&self, app: &AppHandle, direction: Direction) -> Result<SyncState, String> {
        self.0
            .sync(services(app)?, direction)
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn resume(&self, app: &AppHandle) -> Result<SyncState, String> {
        self.0
            .resume(services(app)?)
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn prepare_reset(&self, app: &AppHandle) -> Result<(), String> {
        self.0
            .prepare_reset(services(app)?)
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn restore(&self, app: &AppHandle, id: &str) -> Result<SyncState, String> {
        self.0
            .restore(services(app)?, id.to_owned())
            .await
            .map_err(|error| error.to_string())
    }
    pub async fn restore_document(
        &self,
        app: &AppHandle,
        document: &Value,
    ) -> Result<SyncState, String> {
        self.0
            .restore_document(services(app)?, document.clone())
            .await
            .map_err(|error| error.to_string())
    }
}
