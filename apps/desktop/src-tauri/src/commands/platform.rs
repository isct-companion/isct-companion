#[cfg(matrix_ocr)]
use crate::matrix_ocr;
use crate::require_local_main;
#[cfg(matrix_ocr)]
use crate::shell;
use crate::{coded, CommandError};
#[cfg(matrix_ocr)]
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCapabilities {
    school_import: bool,
    school_web: bool,
    notifications: bool,
    notification_unavailable_reason: Option<&'static str>,
    background_schedule: bool,
    cloud_sync: bool,
    matrix_ocr: bool,
}

/// A still photo is read by the same driver as a camera frame.
#[cfg(matrix_ocr)]
#[tauri::command]
pub(crate) async fn recognize_matrix_image(
    webview: tauri::Webview,
    scan: tauri::State<'_, crate::commands::matrix_camera::MatrixScan>,
    image: String,
    on_progress: tauri::ipc::Channel<matrix_ocr::OcrProgress>,
) -> Result<Vec<matrix_ocr::Recognition>, CommandError> {
    require_local_main(&webview)?;
    let driver = scan.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Tauri's JSON transport carries base64; OCR owns the size limit.
        matrix_ocr::check_base64_image_len(image.len()).map_err(shell)?;
        let image = STANDARD
            .decode(image)
            .map_err(|_| "Invalid image encoding".to_string())?;
        let scan = driver
            .scan_photo(&image, |completed, total| {
                on_progress
                    .send(matrix_ocr::OcrProgress { completed, total })
                    .is_ok()
            })
            .map_err(coded)?;
        Ok(scan
            .cells
            .into_iter()
            .map(|vote| matrix_ocr::Recognition {
                text: vote.text,
                confidence: vote.confidence,
            })
            .collect())
    })
    .await
    .map_err(|e| format!("Matrix recognition task failed: {e}"))?
}

/// Intel macOS has no prebuilt ONNX Runtime; manual matrix entry still works.
#[cfg(not(matrix_ocr))]
#[tauri::command]
pub(crate) fn recognize_matrix_image(webview: tauri::Webview) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    Err(
        "Scan image is not available on Intel Macs yet. Type or paste the 70 letters instead."
            .into(),
    )
}

#[tauri::command]
pub(crate) fn runtime_capabilities(
    webview: tauri::Webview,
) -> Result<RuntimeCapabilities, CommandError> {
    require_local_main(&webview)?;
    let notification_unavailable_reason = super::notifications::unavailable_reason();
    Ok(RuntimeCapabilities {
        school_import: true,
        school_web: true,
        notifications: notification_unavailable_reason.is_none(),
        notification_unavailable_reason,
        background_schedule: true,
        cloud_sync: true,
        matrix_ocr: cfg!(matrix_ocr),
    })
}

#[tauri::command]
pub(crate) async fn check_app_update(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    manual: bool,
    download: bool,
) -> Result<crate::desktop_updates::UpdateStatus, CommandError> {
    crate::require_local_main(&webview)?;
    Ok(crate::core_ports::runtime(&app)
        .run(crate::desktop_updates::check(app.clone(), manual, download))
        .await
        .map_err(coded)??)
}
#[tauri::command]
pub(crate) fn app_update_state(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<crate::desktop_updates::UpdateStatus, CommandError> {
    crate::require_local_main(&webview)?;
    Ok(crate::desktop_updates::snapshot(&app)?)
}
#[tauri::command]
pub(crate) fn install_app_update(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<(), CommandError> {
    crate::require_local_main(&webview)?;
    Ok(crate::desktop_updates::request_install(&app)?)
}

#[tauri::command]
pub(crate) fn application_logs(
    webview: tauri::Webview,
    after: Option<u64>,
) -> Result<app_core::diagnostics::LogSnapshot, CommandError> {
    require_local_main(&webview)?;
    app_core::diagnostics::snapshot(after).map_err(coded)
}

fn application_logs_text(app: &tauri::AppHandle) -> Result<String, CommandError> {
    let about = format!(
        "IsctCompanion {} · {} {}",
        app.package_info().version,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    app_core::diagnostics::export_text(&about).map_err(coded)
}

/// The retained log as text, for the Save dialog.
#[tauri::command]
pub(crate) fn export_application_logs(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<String, CommandError> {
    require_local_main(&webview)?;
    application_logs_text(&app)
}

/// Writes from the shell: after an awaited IPC call WebKit no longer treats
/// `navigator.clipboard.writeText` as part of the click and may reject it.
#[tauri::command]
pub(crate) fn copy_application_logs(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<bool, CommandError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    require_local_main(&webview)?;
    app.clipboard()
        .write_text(application_logs_text(&app)?)
        .map_err(|_| "settings.diagnostics.copyLogsFailed".to_string())?;
    Ok(true)
}

#[tauri::command]
pub(crate) fn application_error(
    webview: tauri::Webview,
    message: String,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    let message: String = message.chars().take(2048).collect();
    app_core::diagnostics::report("frontend", &message);
    Ok(())
}
