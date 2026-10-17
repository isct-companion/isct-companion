#[cfg(all(matrix_ocr, debug_assertions))]
use crate::lms_files;
#[cfg(matrix_ocr)]
use crate::matrix_ocr;
use crate::require_local_main;
#[cfg(matrix_ocr)]
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeCapabilities {
    school_import: bool,
    school_web: bool,
    notifications: bool,
    background_schedule: bool,
    cloud_sync: bool,
    syllabus_search: bool,
    matrix_ocr: bool,
}

#[cfg(matrix_ocr)]
#[tauri::command]
pub(crate) async fn recognize_matrix_image(
    webview: tauri::Webview,
    service: tauri::State<'_, matrix_ocr::MatrixOcrService>,
    app: tauri::AppHandle,
    image: String,
    on_progress: tauri::ipc::Channel<matrix_ocr::OcrProgress>,
) -> Result<Vec<matrix_ocr::Recognition>, String> {
    require_local_main(&webview)?;
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        let service = service.with_debug_dir(
            lms_files::debug_dir(&app)
                .inspect_err(|error| log::error!("Matrix debug directory unavailable: {error}"))
                .ok(),
        );
        #[cfg(not(debug_assertions))]
        let _ = app;
        // Tauri's JSON transport carries base64; OCR owns the size limit.
        matrix_ocr::check_base64_image_len(image.len()).map_err(|error| error.to_string())?;
        let image = STANDARD
            .decode(image)
            .map_err(|_| "Invalid image encoding".to_string())?;
        service
            .scan(&image, |progress| on_progress.send(progress).is_ok())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|e| format!("Matrix recognition task failed: {e}"))?
}

/// Intel macOS has no prebuilt ONNX Runtime; manual matrix entry still works.
#[cfg(not(matrix_ocr))]
#[tauri::command]
pub(crate) fn recognize_matrix_image(webview: tauri::Webview) -> Result<(), String> {
    require_local_main(&webview)?;
    Err(
        "Scan image is not available on Intel Macs yet. Type or paste the 70 letters instead."
            .into(),
    )
}

#[tauri::command]
pub(crate) fn runtime_capabilities(webview: tauri::Webview) -> Result<RuntimeCapabilities, String> {
    require_local_main(&webview)?;
    Ok(RuntimeCapabilities {
        school_import: true,
        school_web: true,
        notifications: true,
        background_schedule: cfg!(not(mobile)),
        cloud_sync: true,
        syllabus_search: true,
        matrix_ocr: cfg!(matrix_ocr),
    })
}

#[cfg(desktop)]
#[tauri::command]
pub(crate) async fn check_app_update(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    manual: bool,
    download: bool,
) -> Result<crate::desktop_updates::UpdateStatus, String> {
    crate::require_local_main(&webview)?;
    crate::core_ports::runtime(&app)
        .run(crate::desktop_updates::check(app.clone(), manual, download))
        .await
        .map_err(|e| e.to_string())?
}
#[cfg(desktop)]
#[tauri::command]
pub(crate) fn app_update_state(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<crate::desktop_updates::UpdateStatus, String> {
    crate::require_local_main(&webview)?;
    crate::desktop_updates::snapshot(&app)
}
#[cfg(desktop)]
#[tauri::command]
pub(crate) fn install_app_update(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<(), String> {
    crate::require_local_main(&webview)?;
    crate::desktop_updates::request_install(&app)
}

#[tauri::command]
pub(crate) fn application_logs(
    webview: tauri::Webview,
    after: Option<u64>,
) -> Result<isct_core::diagnostics::LogSnapshot, String> {
    require_local_main(&webview)?;
    isct_core::diagnostics::snapshot(after).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn application_error(webview: tauri::Webview, message: String) -> Result<(), String> {
    require_local_main(&webview)?;
    let message: String = message.chars().take(2048).collect();
    isct_core::diagnostics::report("frontend", &message);
    Ok(())
}
