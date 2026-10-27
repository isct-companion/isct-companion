//! Desktop camera transport. `MatrixCameraDriver` (Core) runs the session,
//! the tracker hand-off and the OCR consumer for every shell; this file only
//! moves bytes. The webview sends every new video frame as two raw 960 px
//! planes (luma, R-G) over binary IPC and, when Core asks, the
//! full-resolution luma of that same frame for the 70 cells.
use app_core::matrix_scan::{
    MatrixCameraDriver, MatrixCameraSnapshot, MatrixCaptureMetrics, MatrixLuma, MatrixPlanes,
    MatrixScanError,
};
use std::sync::Arc;
use tauri::{Manager, State};

/// The one Matrix driver of the app: camera sessions and photo scans.
#[derive(Clone)]
pub(crate) struct MatrixScan(pub(crate) Arc<MatrixCameraDriver>);
impl Default for MatrixScan {
    fn default() -> Self {
        let driver = MatrixCameraDriver::new(app_ocr::matrix_vision());
        #[cfg(debug_assertions)]
        let driver = driver.with_inspector(|cut| {
            log_frame_quality(cut);
            save_anonymous_samples(cut);
        });
        Self(Arc::new(driver))
    }
}

/// Header of a binary camera request; the body is the raw planes.
struct FrameHeader {
    session_id: String,
    frame_id: u32,
    width: u32,
    height: u32,
    captured_ms: f64,
}
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
#[cfg(debug_assertions)]
fn log_frame_quality(cut: &app_core::matrix_scan::MatrixCut) {
    // Numeric measurements only: no photographs, glyph pixels or letters.
    let range = |measure: fn(&app_core::matrix_scan::MatrixCaptureGlyph) -> f64| {
        let mut values: Vec<_> = cut.glyphs.iter().map(measure).collect();
        values.sort_by(f64::total_cmp);
        (
            values.first().copied().unwrap_or_default(),
            values.get(values.len() / 2).copied().unwrap_or_default(),
            values.last().copied().unwrap_or_default(),
        )
    };
    log::info!(
        "[matrix-camera] keyframe source_px={:?} contrast={:?} sharpness={:?} centering={:?}",
        range(|g| g.source_size),
        range(|g| g.contrast),
        range(|g| g.sharpness),
        range(|g| g.centering),
    );
}
#[cfg(debug_assertions)]
fn save_anonymous_samples(cut: &app_core::matrix_scan::MatrixCut) {
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
    let mut glyphs: Vec<_> = cut
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
#[cfg(debug_assertions)]
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
    scan: State<'_, MatrixScan>,
) -> Result<String, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    scan.0.begin()
}
#[tauri::command]
pub(crate) fn stop_matrix_camera(
    webview: tauri::Webview,
    scan: State<'_, MatrixScan>,
    session_id: String,
    app: tauri::AppHandle,
    capture: Option<MatrixCaptureMetrics>,
) -> Result<(), MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    if let Some(diagnostics) = scan.0.stop(&session_id) {
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
    scan: State<'_, MatrixScan>,
    session_id: String,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    scan.0.poll(&session_id)
}
/// The fast loop: one frame's 960 px luma and R-G planes.
#[tauri::command]
pub(crate) async fn track_matrix_camera_frame(
    webview: tauri::Webview,
    scan: State<'_, MatrixScan>,
    request: tauri::ipc::Request<'_>,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let (frame, body) = frame_request(&request, 2)?;
    let body = body.to_vec();
    let driver = scan.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        if driver.poll(&frame.session_id)?.phase
            == app_core::matrix_scan::MatrixCameraPhase::Capturing
        {
            record_frame(&frame, "planes", &body);
        }
        let (luma, chroma) = body.split_at(body.len() / 2);
        driver.track(
            &frame.session_id,
            frame.frame_id,
            MatrixPlanes {
                width: frame.width,
                height: frame.height,
                luma,
                chroma,
            },
            frame.captured_ms,
        )
    })
    .await
    .map_err(backend)?
}
/// A requested keyframe: the full-resolution luma of the frame named in the
/// last snapshot's `keyframeFrameId`.
#[tauri::command]
pub(crate) async fn capture_matrix_keyframe(
    webview: tauri::Webview,
    scan: State<'_, MatrixScan>,
    request: tauri::ipc::Request<'_>,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let (frame, body) = frame_request(&request, 1)?;
    let body = body.to_vec();
    let driver = scan.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        record_frame(&frame, "full", &body);
        driver.keyframe(
            &frame.session_id,
            frame.frame_id,
            MatrixLuma {
                width: frame.width,
                height: frame.height,
                luma: &body,
            },
        )
    })
    .await
    .map_err(backend)?
}
#[tauri::command]
pub(crate) async fn process_matrix_camera(
    webview: tauri::Webview,
    scan: State<'_, MatrixScan>,
    session_id: String,
    on_progress: tauri::ipc::Channel<MatrixCameraSnapshot>,
) -> Result<MatrixCameraSnapshot, MatrixScanError> {
    crate::require_local_main(&webview).map_err(backend)?;
    let driver = scan.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        driver.process(&session_id, |snapshot| {
            on_progress.send(snapshot.clone()).is_ok()
        })
    })
    .await
    .map_err(backend)?
}
