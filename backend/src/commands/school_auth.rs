//! Credential commands keep potentially interactive OS vault calls off the UI thread.
use crate::{require_local_main, SchoolWebController, SchoolWebResult, WebBounds};
use isct_core::school_auth::{
    SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentialStatus,
    SchoolCredentials,
};
use std::sync::Arc;
use tauri_plugin_clipboard_manager::ClipboardExt;

// Do not share the database command gate: a Keychain prompt must never block
// workspace loading. SchoolAuthService already serializes credential changes.
async fn with_credentials<T: Send + 'static>(
    service: Arc<SchoolAuthService>,
    operation: impl FnOnce(&SchoolAuthService) -> Result<T, SchoolAuthError> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || operation(&service))
        .await
        .map_err(|_| "The credential operation stopped unexpectedly.".to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn school_credential_status(
    webview: tauri::Webview,
    service: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
) -> Result<SchoolCredentialStatus, String> {
    require_local_main(&webview)?;
    with_credentials(service.inner().clone(), |service| service.status()).await
}

#[tauri::command]
pub(crate) async fn save_school_credentials(
    webview: tauri::Webview,
    service: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    with_credentials(service.inner().clone(), move |service| {
        service.save(profile, credentials)
    })
    .await
}

#[tauri::command]
pub(crate) async fn delete_school_credentials(
    webview: tauri::Webview,
    service: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    profile: SchoolAuthProfile,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    with_credentials(service.inner().clone(), move |service| {
        service.delete(profile)
    })
    .await
}

/// Copies the saved Matrix directly from the vault to the system clipboard,
/// so the plaintext never passes through the WebView.
#[tauri::command]
pub(crate) async fn copy_school_matrix(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let text = with_credentials(auth.inner().clone(), |auth| auth.matrix_text()).await?;
    app.clipboard()
        .write_text(text)
        .map_err(|_| "Matrix could not be copied to the clipboard.".to_string())?;
    Ok(true)
}

#[tauri::command]
pub(crate) async fn login_school(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
    bounds: Option<WebBounds>,
) -> Result<SchoolWebResult, String> {
    require_local_main(&webview)?;
    let auth = auth.inner().clone();
    let _ = bounds;
    let controller = controller.inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let credentials = auth
            .load(profile)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Save this school account before signing in.".to_string())?;
        controller.request_login(&handle, profile, credentials)
    })
    .await
    .map_err(|_| "The queued school sign-in stopped unexpectedly.".to_string())?
}

#[tauri::command]
pub(crate) async fn ensure_school_session(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.request_session(&app, profile)
}

#[tauri::command]
pub(crate) async fn test_school_connection(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
) -> Result<SchoolWebResult, String> {
    require_local_main(&webview)?;
    let auth = auth.inner().clone();
    let controller = controller.inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let credentials = auth
            .resolve(profile, credentials)
            .map_err(|error| error.to_string())?;
        controller.request_login(&handle, profile, credentials)
    })
    .await
    .map_err(|_| "The queued school sign-in stopped unexpectedly.".to_string())?
}
