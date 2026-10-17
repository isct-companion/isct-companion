//! Desktop transport only: OCR capability feeds the shared core scan session.
use isct_core::matrix_scan::{
    MatrixCaptureMetrics, MatrixScanError, MatrixScanOutline, MatrixScanSnapshot, MatrixScanTiming,
    MatrixScanner,
};
#[cfg(matrix_ocr)]
use isct_core::matrix_scan::{MatrixPoint, MatrixReading};
use std::sync::{Arc, Mutex};
use tauri::{Manager, State};

#[derive(Clone, Default)]
pub(crate) struct CameraScanner(Arc<Mutex<MatrixScanner>>);
impl CameraScanner {
    fn with<T>(&self, f: impl FnOnce(&mut MatrixScanner) -> T) -> Result<T, MatrixScanError> {
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
        let report = s.diagnostics(&session_id);
        s.stop(&session_id);
        report
    })?;
    if let Ok(diagnostics) = diagnostics {
        // Performance metadata only: no recognized letters, credentials or images.
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
            eprintln!("Matrix performance report unavailable: {error}");
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn scan_matrix_camera_frame(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    app: tauri::AppHandle,
    session_id: String,
    image: String,
    on_geometry: tauri::ipc::Channel<MatrixScanOutline>,
) -> Result<MatrixScanSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let scanner = scanner.inner().clone();
    #[cfg(matrix_ocr)]
    {
        use base64::Engine;
        let ocr = app.state::<isct_ocr::MatrixOcrService>().inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            if !scanner.with(|s| s.is_active(&session_id))? {
                return Err(MatrixScanError::Ended);
            }
            let plan = scanner.with(|s| s.plan(&session_id))??;
            isct_ocr::check_base64_image_len(image.len()).map_err(backend)?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(image)
                .map_err(backend)?;
            // No debug photo export for live frames. Cancellation is observed at
            // every cell; closing the UI invalidates any late completion.
            let mut timing = MatrixScanTiming::default();
            let frame = match ocr.scan_live_frame(
                &bytes,
                &plan.cells,
                |quads| {
                    let points: Vec<_> = quads
                        .iter()
                        .map(|quad| quad.map(|[x, y]| MatrixPoint { x, y }))
                        .collect();
                    scanner
                        .with(|s| s.outline(&session_id, &points))
                        .ok()
                        .and_then(Result::ok)
                        .is_some_and(|outline| on_geometry.send(outline).is_ok())
                },
                |_| scanner.with(|s| s.is_active(&session_id)).unwrap_or(false),
            ) {
                Ok(frame) => {
                    timing = MatrixScanTiming {
                        decode_ms: frame.timing.decode_ms,
                        geometry_ms: frame.timing.geometry_ms,
                        preprocess_ms: frame.timing.preprocess_ms,
                        inference_ms: frame.timing.inference_ms,
                        total_ms: frame.timing.total_ms,
                    };
                    Some((
                        frame
                            .quads
                            .into_iter()
                            .map(|quad| quad.map(|[x, y]| MatrixPoint { x, y }))
                            .collect(),
                        frame
                            .cells
                            .into_iter()
                            .map(|cell| MatrixReading {
                                text: cell.text,
                                confidence: cell.confidence,
                            })
                            .collect(),
                    ))
                }
                Err(isct_ocr::MatrixOcrError::GridNotFound) => None,
                Err(error) => return Err(backend(error)),
            };
            scanner.with(|s| s.observe(&session_id, &plan, frame, timing))?
        })
        .await
        .map_err(backend)?
    }
    #[cfg(not(matrix_ocr))]
    {
        let _ = (scanner, app, session_id, image, on_geometry);
        Err(backend(isct_ocr::MatrixOcrError::Unavailable))
    }
}
