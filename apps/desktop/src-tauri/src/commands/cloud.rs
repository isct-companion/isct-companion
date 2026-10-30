use crate::{
    cloud::{CloudProvider, DesktopCloudService},
    cloud_sync, data, require_local_main,
    school_auth::SchoolAuthService,
    secrets::OsSecretVault,
};
use crate::{coded, CommandError};
use app_core::cloud_sync::{CloudFuture, CloudSignIn, CloudSyncError};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

/// The Drive sign-in Core asks for before turning Drive on: the system
/// browser, returning to a loopback redirect.
struct DesktopSignIn(tauri::AppHandle);

impl CloudSignIn for DesktopSignIn {
    fn signed_in(&self, _: CloudProvider) -> Result<bool, CloudSyncError> {
        self.0
            .state::<DesktopCloudService<OsSecretVault>>()
            .drive_signed_in()
            .map_err(|error| CloudSyncError::Transport(error.to_string()))
    }

    fn sign_in(&self, _: CloudProvider) -> CloudFuture<'_, ()> {
        Box::pin(async move {
            let app = &self.0;
            let workspace = app
                .state::<data::DataStore>()
                .load_fields(&[])
                .map_err(|error| CloudSyncError::Storage(error.to_string()))?;
            let override_id = workspace
                .pointer("/settings/cloud/clientId")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty());
            let configured = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
            let client_id = override_id.or(configured).unwrap_or("").to_string();
            let service = app.state::<DesktopCloudService<OsSecretVault>>();
            service
                .connect(&client_id, "http://127.0.0.1:42813/oauth/callback", |url| {
                    app.opener()
                        .open_url(url, None::<&str>)
                        .map_err(|_| "system browser could not be opened".to_string())
                })
                .await
                .map_err(CloudSyncError::Transport)
        })
    }
}

#[tauri::command]
pub(crate) async fn cloud_lifecycle(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    kind: String,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    if matches!(kind.as_str(), "foreground" | "background") {
        cloud_sync::CloudSync::changed(&app).await;
        app.state::<cloud_sync::CloudSync>().resume(&app).await?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn cloud_sync_state(
    webview: tauri::Webview,
    sync: tauri::State<'_, cloud_sync::CloudSync>,
) -> Result<cloud_sync::SyncState, CommandError> {
    require_local_main(&webview)?;
    sync.state()
}

#[tauri::command]
pub(crate) async fn sync_cloud_now(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    direction: cloud_sync::Direction,
) -> Result<cloud_sync::SyncState, CommandError> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .sync(&app, direction)
        .await
}

#[tauri::command]
pub(crate) async fn set_cloud_sync_automatic(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    automatic: bool,
) -> Result<cloud_sync::SyncState, CommandError> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .set_automatic(&app, automatic)
        .await
}

#[tauri::command]
pub(crate) fn cloud_recovery_document(
    webview: tauri::Webview,
    sync: tauri::State<'_, cloud_sync::CloudSync>,
    id: String,
) -> Result<String, CommandError> {
    require_local_main(&webview)?;
    sync.recovery_document(&id)
}

#[tauri::command]
pub(crate) async fn restore_cloud_version(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<cloud_sync::SyncState, CommandError> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .restore(&app, &id)
        .await
}

#[tauri::command]
pub(crate) async fn set_cloud_sync_provider(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    provider: CloudProvider,
    enabled: bool,
) -> Result<cloud_sync::SyncState, CommandError> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .set_provider(&app, provider, enabled, &DesktopSignIn(app.clone()))
        .await?;
    cloud_sync::CloudSync::changed(&app).await;
    app.state::<cloud_sync::CloudSync>().state()
}

#[tauri::command]
pub(crate) fn cloud_credential_sync_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    service.credential_backup().configured().map_err(coded)
}

#[tauri::command]
pub(crate) async fn cloud_credential_backup_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    service.credential_backup().exists().await.map_err(coded)
}

/// Restores an existing backup with `password` on a device that has none
/// yet; otherwise uploads this device's logins under `password`, a new one.
#[tauri::command]
pub(crate) async fn enable_cloud_credential_sync(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    auth: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    password: String,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    service
        .credential_backup()
        .enable(&password, auth.inner())
        .await
        .map_err(coded)?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn clear_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    service.credential_backup().disable().map_err(coded)?;
    Ok(true)
}
