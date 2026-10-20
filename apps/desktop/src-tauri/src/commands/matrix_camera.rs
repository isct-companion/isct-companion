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
#[cfg(all(matrix_ocr, debug_assertions))]
fn log_frame_quality(frame: &isct_ocr::MatrixCaptureFrame) {
    // Numeric measurements only: no photographs, glyph pixels or letters.
    let range = |measure: fn(&isct_ocr::MatrixCaptureGlyph) -> f64| {
        let mut values: Vec<_> = frame.glyphs.iter().map(measure).collect();
        values.sort_by(f64::total_cmp);
        (
            values.first().copied().unwrap_or_default(),
            values.get(values.len() / 2).copied().unwrap_or_default(),
            values.last().copied().unwrap_or_default(),
        )
    };
    log::info!(
        "[matrix-camera] frame_ms={:.1} decode_ms={:.1} geometry_ms={:.1} glyph_ms={:.1} source_px={:?} contrast={:?} sharpness={:?} centering={:?}",
        frame.timing.total_ms,
        frame.timing.decode_ms,
        frame.timing.geometry_ms,
        frame.timing.preprocess_ms,
        range(|g| g.source_size),
        range(|g| g.contrast),
        range(|g| g.sharpness),
        range(|g| g.centering),
    );
}
#[cfg(all(matrix_ocr, debug_assertions))]
fn save_anonymous_samples(frame: &isct_ocr::MatrixCaptureFrame) {
    use rand::seq::SliceRandom;
    use std::{
        io::Write,
        sync::atomic::{AtomicBool, Ordering},
    };
    static SAVED: AtomicBool = AtomicBool::new(false);
    let Some(directory) = std::env::var_os("ISCT_MATRIX_GLYPH_SAMPLES") else {
        return;
    };
    if SAVED.load(Ordering::Relaxed) {
        return;
    }
    let mut glyphs: Vec<_> = frame
        .glyphs
        .iter()
        .filter(|g| {
            g.source_size >= 25. && g.centering >= 0.5 && g.contrast >= 12. && g.sharpness >= 10.
        })
        .collect();
    if glyphs.len() < 60 || SAVED.swap(true, Ordering::Relaxed) {
        return;
    }
    // Explicit local opt-in, once per process. Discard cell positions before
    // writing at most twelve shuffled raw 96×96 samples; never retain a card.
    glyphs.shuffle(&mut rand::rng());
    glyphs.truncate(12);
    let save = || -> std::io::Result<()> {
        let directory = std::path::PathBuf::from(directory);
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        for glyph in glyphs {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            options
                .open(directory.join(format!("{}.gray", uuid::Uuid::new_v4())))?
                .write_all(&glyph.gray)?;
        }
        Ok(())
    };
    match save() {
        Ok(()) => log::info!("[matrix-camera] saved twelve anonymous glyph samples"),
        Err(error) => log::error!("[matrix-camera] anonymous sample export failed: {error}"),
    }
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
            let captured = isct_ocr::capture_matrix_frame(&bytes);
            #[cfg(debug_assertions)]
            if let Ok(frame) = &captured {
                log_frame_quality(frame);
                save_anonymous_samples(frame);
            }
            let frame = match captured {
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
