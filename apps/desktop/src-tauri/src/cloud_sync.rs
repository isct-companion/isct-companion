//! Native cloud ports; queue, recovery and retry policy live in Core.
use crate::{cloud::DesktopCloudService, secrets::OsSecretVault};
use app_core::cloud_sync::document::{
    BoxFuture, CloudWorkspaceFile, DocumentCloud, DocumentError, WorkspaceTransport,
};
use app_core::cloud_sync::CloudServices;
pub use app_core::cloud_sync::{CloudProvider, Direction, SyncState};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Manager};

/// Reads the managed service on each call so the transport never outlives it.
struct NativeCloud(AppHandle);
impl WorkspaceTransport for NativeCloud {
    fn read<'a>(&'a self, provider: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
        WorkspaceTransport::read(self.service(), provider)
    }
    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> BoxFuture<'a, ()> {
        WorkspaceTransport::write(self.service(), provider, file, expected_revision)
    }
    fn disconnect(&self, provider: CloudProvider) -> Result<(), DocumentError> {
        WorkspaceTransport::disconnect(self.service(), provider)
    }
}
impl NativeCloud {
    fn service(&self) -> &DesktopCloudService<OsSecretVault> {
        self.0.state::<DesktopCloudService<OsSecretVault>>().inner()
    }
}
pub(crate) fn services(app: &AppHandle) -> Result<CloudServices, String> {
    Ok(CloudServices {
        backup: crate::commands::backup::service(app)?,
        transport: Arc::new(DocumentCloud::new(
            NativeCloud(app.clone()),
            std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .ok(),
        )),
        events: Arc::new(crate::core_ports::DesktopEvents(app.clone())),
    })
}

pub struct CloudSync(pub(crate) app_core::cloud_sync::CloudSync);
impl CloudSync {
    pub fn new(legacy: PathBuf, authorized: &[CloudProvider]) -> Result<Self, String> {
        let supported = [
            CloudProvider::GoogleDrive,
            #[cfg(icloud)]
            CloudProvider::Icloud,
        ];
        app_core::cloud_sync::CloudSync::new(legacy, &supported, authorized)
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
                log::error!("Could not journal cloud update: {error}");
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
