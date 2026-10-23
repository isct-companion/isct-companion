//! Credential commands keep potentially interactive OS vault calls off the UI thread.
use crate::{
    require_local_main,
    school_web::{SchoolWebController, SchoolWebResult, WebBounds},
};
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
pub(crate) fn begin_school_otp_setup(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
) -> Result<u64, isct_core::school_auth::OtpSetupError> {
    require_local_main(&webview).map_err(|_| isct_core::school_auth::OtpSetupError::Unavailable)?;
    controller.engine.begin_school_otp_setup()
}

#[tauri::command]
pub(crate) fn cancel_school_otp_setup(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    operation_id: u64,
) -> Result<(), isct_core::school_auth::OtpSetupError> {
    require_local_main(&webview).map_err(|_| isct_core::school_auth::OtpSetupError::Unavailable)?;
    controller.engine.cancel_school_otp_setup(operation_id);
    Ok(())
}

#[tauri::command]
pub(crate) async fn complete_school_otp_setup(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    operation_id: u64,
    reset: bool,
) -> Result<(), isct_core::school_auth::OtpSetupError> {
    use isct_core::school_auth::OtpSetupError;
    require_local_main(&webview).map_err(|_| OtpSetupError::Unavailable)?;
    let engine = controller.engine.clone();
    crate::core_ports::runtime(&app)
        .run(async move { engine.complete_school_otp_setup(operation_id, reset).await })
        .await
        .map_err(|_| OtpSetupError::Unavailable)?
}

#[tauri::command]
pub(crate) async fn school_otp_setup(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    operation_id: u64,
) -> Result<isct_core::school_auth::SchoolOtpSetup, isct_core::school_auth::OtpSetupError> {
    use isct_core::school_auth::OtpSetupError;
    require_local_main(&webview).map_err(|_| OtpSetupError::Unavailable)?;
    let engine = controller.engine.clone();
    crate::core_ports::runtime(&app)
        .run(async move { engine.school_otp_setup(operation_id).await })
        .await
        .map_err(|_| OtpSetupError::Unavailable)?
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
    app: tauri::AppHandle,
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let core = crate::core_ports::core(&app);
    tauri::async_runtime::spawn_blocking(move || core.save_credentials(profile, credentials))
        .await
        .map_err(|_| "The credential operation stopped unexpectedly.".to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn delete_school_credentials(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    profile: SchoolAuthProfile,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let core = crate::core_ports::core(&app);
    tauri::async_runtime::spawn_blocking(move || core.delete_credentials(profile))
        .await
        .map_err(|_| "The credential operation stopped unexpectedly.".to_string())?
        .map_err(|error| error.to_string())
}

/// Export the current draft or saved OTP setup directly to the OS clipboard.
#[tauri::command]
pub(crate) async fn copy_school_otp(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, Arc<SchoolAuthService>>,
    setup: Option<String>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let text = with_credentials(auth.inner().clone(), move |auth| {
        auth.otp_setup_text(setup.as_deref())
    })
    .await?;
    app.clipboard()
        .write_text(text)
        .map_err(|_| "OTP setup could not be copied to the clipboard.".to_string())?;
    Ok(true)
}

/// The same export as a QR code. The WebView receives only the SVG, and
/// only while the user has the QR dialog open.
#[tauri::command]
pub(crate) async fn school_otp_qr(
    webview: tauri::Webview,
    auth: tauri::State<'_, Arc<SchoolAuthService>>,
    setup: Option<String>,
) -> Result<String, String> {
    require_local_main(&webview)?;
    with_credentials(auth.inner().clone(), move |auth| {
        auth.otp_setup_qr(setup.as_deref())
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
    let _ = bounds;
    let credentials = with_credentials(auth.inner().clone(), move |auth| auth.load(profile))
        .await?
        .ok_or_else(|| "Save this school account before signing in.".to_string())?;
    let engine = controller.engine.clone();
    crate::core_ports::runtime(&app)
        .run(async move { engine.login(profile, credentials).await })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn school_session_snapshot(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
) -> Result<isct_core::school_browser::SchoolSessionSnapshot, String> {
    require_local_main(&webview)?;
    controller
        .engine
        .session_snapshot()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn school_session_lifecycle(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    kind: isct_core::school_browser::SchoolSessionLifecycleKind,
    visible: bool,
) -> Result<(), String> {
    require_local_main(&webview)?;
    let engine = controller.engine.clone();
    crate::core_ports::runtime(&app)
        .run(async move {
            engine
                .lifecycle(isct_core::school_browser::SchoolSessionLifecycle { kind, visible })
                .await
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
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
    let credentials = with_credentials(auth.inner().clone(), move |auth| {
        auth.resolve(profile, credentials)
    })
    .await?;
    let engine = controller.engine.clone();
    crate::core_ports::runtime(&app)
        .run(async move { engine.login(profile, credentials).await })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Import is local parsing only; saving still requires the credential intent.
#[tauri::command]
pub(crate) fn import_school_otp(
    webview: tauri::Webview,
    payload: String,
) -> Result<Vec<isct_core::school_auth::OtpImportAccount>, isct_core::school_auth::OtpImportError> {
    require_local_main(&webview).map_err(|_| isct_core::school_auth::OtpImportError::Invalid)?;
    isct_core::school_auth::import_otp(&payload)
}
