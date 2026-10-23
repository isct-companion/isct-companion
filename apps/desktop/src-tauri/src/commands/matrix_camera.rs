//! Desktop camera transport. Core owns acquisition, its deadline and offline
//! votes; isct-ocr owns the geometry. The webview sends every new video frame
//! as two raw 960 px planes (luma, R-G) over binary IPC and, when Core asks,
//! the full-resolution luma of that same frame for the 70 cells.
#[cfg(matrix_ocr)]
use isct_core::matrix_scan::{
    MatrixCameraPhase, MatrixCaptureFrame, MatrixCaptureGlyph, MatrixPoint, MatrixReading,
    MatrixTrackedFrame,
};
use isct_core::matrix_scan::{
    MatrixCameraScanner, MatrixCameraSnapshot, MatrixCaptureMetrics, MatrixScanError,
};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tauri::{Manager, State};

/// Image-side state of the live session: the tracker and the keyframe Core
/// asked for (its frame id and the grid fitted on that frame).
#[derive(Default)]
struct CameraGeometry {
    session: String,
    tracker: isct_ocr::MatrixTracker,
    keyframe: Option<(u32, [[f64; 2]; 4])>,
}

#[derive(Clone, Default)]
pub(crate) struct CameraScanner(Arc<Mutex<MatrixCameraScanner>>, Arc<Mutex<CameraGeometry>>);
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
    fn geometry<T>(&self, f: impl FnOnce(&mut CameraGeometry) -> T) -> Result<T, MatrixScanError> {
        self.1
            .lock()
            .map(|mut geometry| f(&mut geometry))
            .map_err(|_| {
                MatrixScanError::Backend("Camera geometry unavailable. Restart the app.".into())
            })
    }
}

/// Header of a binary camera request; the body is the raw planes.
#[cfg(matrix_ocr)]
struct FrameHeader {
    session_id: String,
    frame_id: u32,
    width: u32,
    height: u32,
    captured_ms: f64,
}
#[cfg(matrix_ocr)]
fn frame_request<'a>(
    request: &'a tauri::ipc::Request<'_>,
    planes: usize,
) -> Result<(FrameHeader, &'a [u8]), MatrixScanError> {
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| MatrixScanError::InvalidFrame(format!("Missing {name}.")))
    };
    let number = |name: &str| -> Result<f64, MatrixScanError> {
        header(name)?
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.)
            .ok_or_else(|| MatrixScanError::InvalidFrame(format!("Invalid {name}.")))
    };
    let parsed = FrameHeader {
        session_id: header("x-matrix-session")?.to_owned(),
        frame_id: number("x-matrix-frame")? as u32,
        width: number("x-matrix-width")? as u32,
        height: number("x-matrix-height")? as u32,
        captured_ms: number("x-matrix-captured")?,
    };
    let tauri::ipc::InvokeBody::Raw(body) = request.body() else {
        return Err(MatrixScanError::InvalidFrame("Expected raw planes.".into()));
    };
    let pixels = parsed.width as u64 * parsed.height as u64;
    if !(32..=8192).contains(&parsed.width)
        || !(32..=8192).contains(&parsed.height)
        || body.len() as u64 != pixels * planes as u64
    {
        return Err(MatrixScanError::InvalidFrame("Plane size mismatch.".into()));
    }
    Ok((parsed, body))
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
        "[matrix-camera] keyframe_ms={:.1} source_px={:?} contrast={:?} sharpness={:?} centering={:?}",
        frame.timing.total_ms,
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
/// Explicit local opt-in for replay: `ISCT_MATRIX_RECORD_DIR` makes a debug
/// build save every analysed frame's raw planes (`.planes`: luma then R-G)
/// and every keyframe's full-resolution luma (`.full`), indexed with sizes and
/// webview capture times in `frames.jsonl`. These are card images: use a
/// disposable card and keep recordings local.
#[cfg(all(matrix_ocr, debug_assertions))]
fn record_frame(frame: &FrameHeader, kind: &str, bytes: &[u8]) {
    use std::io::Write;
    let Some(root) = std::env::var_os("ISCT_MATRIX_RECORD_DIR") else {
        return;
    };
    let save = || -> std::io::Result<()> {
        let directory = std::path::PathBuf::from(root).join(&frame.session_id);
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        let file = format!("{:06}.{kind}", frame.frame_id);
        std::fs::write(directory.join(&file), bytes)?;
        let line = serde_json::json!({
            "file": file, "kind": kind, "frameId": frame.frame_id,
            "width": frame.width, "height": frame.height, "capturedMs": frame.captured_ms,
        });
        let mut index = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("frames.jsonl"))?;
        writeln!(index, "{line}")
    };
    if let Err(error) = save() {
        log::error!("[matrix-camera] frame recording failed: {error}");
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
    let id = scanner.with(|s| s.begin())?;
    scanner.geometry(|g| {
        *g = CameraGeometry {
            session: id.clone(),
            ..CameraGeometry::default()
        }
    })?;
    Ok(id)
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
/// The fast loop: one frame's 960 px luma and R-G planes. Returns Core's
/// snapshot with the outline fitted on this very frame, and whether Core
/// wants this frame's full resolution as a keyframe.
#[tauri::command]
pub(crate) async fn track_matrix_camera_frame(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    request: tauri::ipc::Request<'_>,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    #[cfg(matrix_ocr)]
    {
        let (frame, body) = frame_request(&request, 2)?;
        let body = body.to_vec();
        let scanner = scanner.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let current =
                scanner.with(|s| s.observe(&frame.session_id, None, Instant::now()))??;
            if current.phase != MatrixCameraPhase::Capturing {
                return Ok(current);
            }
            #[cfg(debug_assertions)]
            record_frame(&frame, "planes", &body);
            let (luma, chroma) = body.split_at(body.len() / 2);
            let track = scanner.geometry(|g| {
                if g.session != frame.session_id {
                    return Err(MatrixScanError::Ended);
                }
                g.tracker
                    .track(&isct_ocr::MatrixAnalysisFrame {
                        width: frame.width,
                        height: frame.height,
                        luma,
                        chroma,
                        time: frame.captured_ms / 1000.,
                    })
                    .map_err(backend)
            })??;
            let snapshot = scanner.with(|s| {
                s.track(
                    &frame.session_id,
                    MatrixTrackedFrame {
                        frame_id: frame.frame_id,
                        bounds: track.display.map(|q| q.map(|[x, y]| MatrixPoint { x, y })),
                        keyframe_ready: track.keyframe,
                        analysis_ms: track.elapsed_ms,
                    },
                    Instant::now(),
                )
            })??;
            let wanted = snapshot.keyframe_frame_id == Some(frame.frame_id);
            scanner.geometry(|g| {
                g.keyframe = track
                    .body
                    .filter(|_| wanted)
                    .map(|body| (frame.frame_id, body));
            })?;
            Ok(snapshot)
        })
        .await
        .map_err(backend)?
    }
    #[cfg(not(matrix_ocr))]
    {
        let _ = (scanner, request);
        Err(backend(isct_ocr::MatrixOcrError::Unavailable))
    }
}

/// A requested keyframe: the full-resolution luma of the frame named in the
/// last snapshot's `keyframeFrameId`, cut with the grid fitted on it.
#[tauri::command]
pub(crate) async fn capture_matrix_keyframe(
    webview: tauri::Webview,
    scanner: State<'_, CameraScanner>,
    request: tauri::ipc::Request<'_>,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    #[cfg(matrix_ocr)]
    {
        let (frame, body) = frame_request(&request, 1)?;
        let body = body.to_vec();
        let scanner = scanner.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let grid = scanner.geometry(|g| {
                (g.session == frame.session_id)
                    .then(|| g.keyframe.take())
                    .flatten()
                    .filter(|(id, _)| *id == frame.frame_id)
            })?;
            // A keyframe superseded by a newer request is dropped, not cut.
            let Some((_, body_quad)) = grid else {
                return scanner.with(|s| s.observe(&frame.session_id, None, Instant::now()))?;
            };
            #[cfg(debug_assertions)]
            record_frame(&frame, "full", &body);
            let cut =
                isct_ocr::capture_matrix_keyframe(&body, frame.width, frame.height, body_quad)
                    .map_err(backend)?;
            #[cfg(debug_assertions)]
            {
                log_frame_quality(&cut);
                save_anonymous_samples(&cut);
            }
            let observed = MatrixCaptureFrame {
                frame_id: frame.frame_id,
                quads: cut
                    .quads
                    .into_iter()
                    .map(|q| q.map(|[x, y]| MatrixPoint { x, y }))
                    .collect(),
                glyphs: cut
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
            };
            scanner.with(|s| s.observe(&frame.session_id, Some(observed), Instant::now()))?
        })
        .await
        .map_err(backend)?
    }
    #[cfg(not(matrix_ocr))]
    {
        let _ = (scanner, request);
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
            let result = (|| {
                loop {
                    let current =
                        scanner.with(|s| s.observe(&session_id, None, Instant::now()))??;
                    if current.phase == MatrixCameraPhase::Complete {
                        return Ok(current);
                    }
                    let Some(batch) = scanner.with(|s| s.batch(&session_id, Instant::now()))??
                    else {
                        // This worker owns the consumer until completion/cancellation. The
                        // session deadline bounds this wait; no lock is held while waiting.
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    };
                    let readings = alphabet
                        .recognize(&batch.images)
                        .map_err(backend)?
                        .into_iter()
                        .map(|r| MatrixReading {
                            text: r.text,
                            confidence: r.confidence,
                        })
                        .collect();
                    let snapshot = scanner
                        .with(|s| s.apply(&session_id, &batch, readings, Instant::now()))??;
                    if snapshot.phase == MatrixCameraPhase::Complete {
                        return Ok(snapshot);
                    }
                    on_progress
                        .send(snapshot)
                        .map_err(|_| MatrixScanError::Ended)?;
                }
            })();
            if result.is_err() {
                scanner.with(|s| s.stop(&session_id))?;
            }
            result
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
