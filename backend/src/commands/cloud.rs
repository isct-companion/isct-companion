use crate::{
    cloud::{CloudProvider, DesktopCloudService},
    cloud_sync, data, require_local_main,
    school_auth::SchoolAuthService,
    secrets::OsSecretVault,
};
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub(crate) async fn cloud_lifecycle(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    kind: String,
) -> Result<(), String> {
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
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    sync.state()
}

#[tauri::command]
pub(crate) async fn sync_cloud_now(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    direction: cloud_sync::Direction,
) -> Result<cloud_sync::SyncState, String> {
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
) -> Result<cloud_sync::SyncState, String> {
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
) -> Result<String, String> {
    require_local_main(&webview)?;
    sync.recovery_document(&id)
}

#[tauri::command]
pub(crate) async fn restore_cloud_version(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<cloud_sync::SyncState, String> {
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
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    if enabled && provider == CloudProvider::GoogleDrive {
        let service = app.state::<DesktopCloudService<OsSecretVault>>();
        if !service.authorized_providers()?.contains(&provider) {
            let workspace = app
                .state::<data::DataStore>()
                .load_workspace()
                .map_err(|error| error.to_string())?;
            let override_id = workspace
                .pointer("/settings/cloud/clientId")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty());
            #[cfg(desktop)]
            let configured = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
            #[cfg(target_os = "ios")]
            let configured = option_env!("ISCT_GOOGLE_IOS_CLIENT_ID");
            #[cfg(target_os = "android")]
            let configured = option_env!("ISCT_GOOGLE_ANDROID_CLIENT_ID");
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
                    .provider_result(&app, provider, false, Some(error))
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
) -> Result<bool, String> {
    require_local_main(&webview)?;
    if provider == CloudProvider::Icloud {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        return service.connect_icloud().await;
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        return Err("iCloud sync is only available on Apple devices.".into());
    }
    #[cfg(mobile)]
    {
        let _ = redirect_uri;
        if provider != CloudProvider::GoogleDrive {
            return Err("This cloud service is not available on this device.".into());
        }
        let token = tauri_plugin_google_drive_auth::sign_in(&app, &client_id)?;
        service.connect_with_native_token(
            provider,
            &token.client_id,
            token.access_token,
            token.refresh_token,
            token.expires_in,
        )
    }
    #[cfg(desktop)]
    {
        service
            .connect(provider, &client_id, &redirect_uri, move |url| {
                app.opener()
                    .open_url(url, None::<&str>)
                    .map_err(|_| "system browser could not be opened".to_string())
            })
            .await
    }
}

#[tauri::command]
pub(crate) fn cloud_authorization_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<Vec<CloudProvider>, String> {
    require_local_main(&webview)?;
    service.authorized_providers()
}

#[tauri::command]
pub(crate) fn disconnect_cloud(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    #[cfg(target_os = "ios")]
    if provider == CloudProvider::GoogleDrive {
        tauri_plugin_google_drive_auth::sign_out(&app)?;
    }
    #[cfg(not(target_os = "ios"))]
    let _ = app;
    service.disconnect(provider)?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn cloud_credential_sync_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.credential_sync_configured()
}

#[tauri::command]
pub(crate) async fn cloud_credential_backup_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.credential_backup_exists(provider).await
}

#[tauri::command]
pub(crate) fn set_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    password: String,
) -> Result<bool, String> {
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
) -> Result<bool, String> {
    require_local_main(&webview)?;
    if provider == CloudProvider::Icloud {
        return Err("iCloud credentials sync through iCloud Keychain.".into());
    }
    if restore_existing {
        if let Some(credentials) = service.pull_credentials(provider).await? {
            auth.import_sync_payload(&credentials)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
    }
    let credentials = auth
        .export_sync_payload()
        .map_err(|error| error.to_string())?;
    service.back_up_credentials(provider, &credentials).await?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn clear_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.clear_credential_sync_password()?;
    Ok(true)
}
