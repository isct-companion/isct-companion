use crate::{
    cloud::{CloudProvider, DesktopCloudService},
    cloud_sync, data, require_local_main,
    school_auth::SchoolAuthService,
    secrets::OsSecretVault,
};
use crate::{coded, CommandError};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

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
    if enabled && provider == CloudProvider::GoogleDrive {
        let service = app.state::<DesktopCloudService<OsSecretVault>>();
        if !service.authorized_providers()?.contains(&provider) {
            let workspace = app
                .state::<data::DataStore>()
                .load_fields(&[])
                .map_err(coded)?;
            let override_id = workspace
                .pointer("/settings/cloud/clientId")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty());
            let configured = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
            let client_id = override_id.or(configured).unwrap_or("").to_string();
            if let Err(error) = connect_cloud(
                webview,
                app.clone(),
                service,
                provider,
                client_id,
                "http://127.0.0.1:42813/oauth/callback".into(),
            )
            .await
            {
                return app
                    .state::<cloud_sync::CloudSync>()
                    .provider_result(&app, provider, false, Some(error.to_string()))
                    .await;
            }
        }
    }
    app.state::<cloud_sync::CloudSync>()
        .set_provider(&app, provider, enabled)
        .await?;
    cloud_sync::CloudSync::changed(&app).await;
    app.state::<cloud_sync::CloudSync>().state()
}

#[tauri::command]
pub(crate) async fn connect_cloud(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
    client_id: String,
    redirect_uri: String,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    if provider == CloudProvider::Icloud {
        #[cfg(target_os = "macos")]
        return Ok(service.connect_icloud().await?);
        #[cfg(not(target_os = "macos"))]
        return Err("iCloud sync is only available on Apple devices.".into());
    }
    Ok(service
        .connect(provider, &client_id, &redirect_uri, move |url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|_| "system browser could not be opened".to_string())
        })
        .await?)
}

#[tauri::command]
pub(crate) fn cloud_authorization_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<Vec<CloudProvider>, CommandError> {
    require_local_main(&webview)?;
    Ok(service.authorized_providers()?)
}

#[tauri::command]
pub(crate) fn disconnect_cloud(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    let _ = app;
    service.disconnect(provider)?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn cloud_credential_sync_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(service.credential_sync_configured()?)
}

#[tauri::command]
pub(crate) async fn cloud_credential_backup_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(service.credential_backup_exists(provider).await?)
}

#[tauri::command]
pub(crate) fn set_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    password: String,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    service.set_credential_sync_password(&password)?;
    Ok(true)
}

#[tauri::command]
pub(crate) async fn sync_cloud_credentials(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    auth: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    provider: CloudProvider,
    restore_existing: bool,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    if provider == CloudProvider::Icloud {
        return Err("iCloud credentials sync through iCloud Keychain.".into());
    }
    if restore_existing {
        if let Some(credentials) = service.pull_credentials(provider).await? {
            auth.import_sync_payload(&credentials).map_err(coded)?;
            return Ok(true);
        }
    }
    let credentials = auth.export_sync_payload().map_err(coded)?;
    service.back_up_credentials(provider, &credentials).await?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn clear_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    service.clear_credential_sync_password()?;
    Ok(true)
}
