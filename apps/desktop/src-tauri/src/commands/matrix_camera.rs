//! Desktop camera transport. Core owns acquisition, its deadline and offline votes.
#[cfg(matrix_ocr)]
use isct_core::matrix_scan::{
    MatrixCameraPhase, MatrixCaptureFrame, MatrixCaptureGlyph, MatrixPoint, MatrixReading,
};
use isct_core::matrix_scan::{
    MatrixCameraScanner, MatrixCameraSnapshot, MatrixCaptureMetrics, MatrixScanError,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{Manager, State};

#[derive(Clone, Default)]
pub(crate) struct CameraScanner(Arc<Mutex<MatrixCameraScanner>>);
impl CameraScanner {
    fn with<T>(&self, f: impl FnOnce(&mut MatrixCameraScanner) -> T) -> Result<T, MatrixScanError> {
        self.0
            .lock()
            .map(|mut scanner| f(&mut scanner))
            .map_err(|_| {
                MatrixScanError::Backend(
                    "Camera scanner state unavailable. Restart the app.".into(),
                )
            })
    }
}
fn backend(error: impl std::fmt::Display) -> MatrixScanError {
    MatrixScanError::Backend(error.to_string())
}
#[tauri::command]
pub(crate) fn begin_matrix_camera(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
) -> Result<String, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    if !isct_ocr::available() {
        return Err(backend(isct_ocr::MatrixOcrError::Unavailable));
    }
    scanner.with(|s| s.begin())
}
#[tauri::command]
pub(crate) fn stop_matrix_camera(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    session_id: String,
    app: tauri::AppHandle,
    capture: Option<MatrixCaptureMetrics>,
) -> Result<(), MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let diagnostics = scanner.with(|s| {
        let report = s.diagnostics(&session_id, Instant::now());
        s.stop(&session_id);
        report
    })?;
    if let Ok(diagnostics) = diagnostics {
        // Aggregate performance only; never serialize candidates or their letters.
        let save = || -> Result<(), Box<dyn std::error::Error>> {
            let directory = app.path().app_cache_dir()?.join("matrix-scan");
            std::fs::create_dir_all(&directory)?;
            let report = serde_json::json!({"sessionId":session_id,"version":app.config().version,"core":diagnostics,"capture":capture});
            let temporary = directory.join(format!("{session_id}.tmp"));
            std::fs::write(&temporary, serde_json::to_vec_pretty(&report)?)?;
            std::fs::rename(temporary, directory.join("latest.json"))?;
            Ok(())
        };
        if let Err(error) = save() {
            log::error!("Matrix performance report unavailable: {error}");
        }
    }
    Ok(())
}
#[tauri::command]
pub(crate) fn poll_matrix_camera(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    session_id: String,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    scanner.with(|s| s.observe(&session_id, None, Instant::now()))?
}
#[tauri::command]
pub(crate) async fn capture_matrix_camera_frame(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    session_id: String,
    image: String,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let scanner = scanner.inner().clone();
    #[cfg(matrix_ocr)]
    {
        use base64::Engine;
        tauri::async_runtime::spawn_blocking(move || {
            let current = scanner.with(|s| s.observe(&session_id, None, Instant::now()))??;
            if current.phase != MatrixCameraPhase::Capturing {
                return Ok(current);
            }
            isct_ocr::check_base64_image_len(image.len()).map_err(backend)?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(image)
                .map_err(backend)?;
            let frame = match isct_ocr::capture_matrix_frame(&bytes) {
                Ok(frame) => Some(MatrixCaptureFrame {
                    quads: frame
                        .quads
                        .into_iter()
                        .map(|q| q.map(|[x, y]| MatrixPoint { x, y }))
                        .collect(),
                    glyphs: frame
                        .glyphs
                        .into_iter()
                        .map(|g| MatrixCaptureGlyph {
                            gray: g.gray,
                            contrast: g.contrast,
                            sharpness: g.sharpness,
                            source_size: g.source_size,
                            centering: g.centering,
                            glare: g.glare,
                        })
                        .collect(),
                }),
                Err(isct_ocr::MatrixOcrError::GridNotFound) => None,
                Err(error) => return Err(backend(error)),
            };
            scanner.with(|s| s.observe(&session_id, frame, Instant::now()))?
        })
        .await
        .map_err(backend)?
    }
    #[cfg(not(matrix_ocr))]
    {
        let _ = (scanner, session_id, image);
        Err(backend(isct_ocr::MatrixOcrError::Unavailable))
    }
}
#[tauri::command]
pub(crate) async fn process_matrix_camera(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    app: tauri::AppHandle,
    session_id: String,
    on_progress: tauri::ipc::Channel<MatrixCameraSnapshot>,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let scanner = scanner.inner().clone();
    #[cfg(matrix_ocr)]
    {
        let alphabet = app
            .state::<isct_ocr::MatrixAlphabetService>()
            .inner()
            .clone();
        tauri::async_runtime::spawn_blocking(move || {
            while let Some(batch) = scanner.with(|s| s.batch(&session_id))?? {
                let readings = alphabet
                    .recognize(&batch.images)
                    .map_err(backend)?
                    .into_iter()
                    .map(|r| MatrixReading {
                        text: r.text,
                        confidence: r.confidence,
                    })
                    .collect();
                let snapshot =
                    scanner.with(|s| s.apply(&session_id, &batch, readings, Instant::now()))??;
                if on_progress.send(snapshot).is_err() {
                    scanner.with(|s| s.stop(&session_id))?;
                    return Err(MatrixScanError::Ended);
                }
            }
            scanner.with(|s| s.observe(&session_id, None, Instant::now()))?
        })
        .await
        .map_err(backend)?
    }
    #[cfg(not(matrix_ocr))]
    {
        let _ = (scanner, session_id, on_progress, app);
        Err(backend(isct_ocr::MatrixOcrError::Unavailable))
    }
}
