//! Replay recorded frames through the actual bounded acquisition and offline OCR.
#[cfg(matrix_ocr)]
use isct_core::matrix_scan::{
    MatrixCameraPhase, MatrixCameraScanner, MatrixCaptureFrame, MatrixCaptureGlyph, MatrixPoint,
    MatrixReading,
};
#[cfg(matrix_ocr)]
use std::{
    error::Error,
    path::PathBuf,
    time::{Duration, Instant},
};
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn Error>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("Pass recorded frame directory")?,
    );
    let expected =
        std::fs::read_to_string(std::env::args().nth(2).ok_or("Pass expected text file")?)?
            .chars()
            .filter(|c| c.is_ascii_uppercase())
            .collect::<String>();
    if expected.len() != 70 {
        return Err("Expected exactly 70 letters".into());
    }
    let fps = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "5".into())
        .parse::<f64>()?;
    if !fps.is_finite() || fps <= 0. {
        return Err("Invalid capture frequency".into());
    }
    let live = std::env::args().nth(4).as_deref() == Some("live");
    let mut frames = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("capture-"))
                && p.extension().is_some_and(|e| e == "jpg")
        })
        .collect::<Vec<_>>();
    frames.sort();
    if frames.is_empty() && !live {
        return Err("No capture-*.jpg frames".into());
    }
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin();
    let base = Instant::now();
    let mut capture_ms = vec![];
    let mut sealed = None;
    let mut first_matrix_index = None;
    let mut first_geometry_index = None;
    let mut geometry_frames = 0usize;
    let mut sealed_index = None;
    let mut index = 0usize;
    let mut modified = None;
    eprintln!("READY: waiting for Matrix frames");
    loop {
        let now = if live {
            Instant::now()
        } else {
            base + Duration::from_secs_f64(index as f64 / fps)
        };
        let path = if live {
            let snapshot = scanner.observe(&id, None, now)?;
            if snapshot.phase != MatrixCameraPhase::Capturing {
                sealed = Some((snapshot, now));
                break;
            }
            if base.elapsed() > Duration::from_secs(75) {
                return Err("No Matrix within diagnostic lifetime".into());
            }
            let path = directory.join("latest.jpg");
            let stamp = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok());
            if stamp.is_none() || stamp == modified {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            modified = stamp;
            path
        } else {
            let Some(path) = frames.get(index) else {
                break;
            };
            path.clone()
        };
        let stage = Instant::now();
        let frame = match isct_ocr::capture_matrix_frame(&std::fs::read(path)?) {
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
            Err(error) => return Err(error.into()),
        };
        capture_ms.push(stage.elapsed().as_secs_f64() * 1000.);
        if frame.is_some() {
            first_geometry_index.get_or_insert(index);
            geometry_frames += 1;
        }
        let snapshot = scanner.observe(&id, frame, now)?;
        if snapshot.bounds.is_some() {
            first_matrix_index.get_or_insert(index);
        }
        if snapshot.phase != MatrixCameraPhase::Capturing {
            sealed_index = Some(index);
            sealed = Some((snapshot, now));
            break;
        }
        index += 1;
    }
    let Some((captured, now)) = sealed else {
        // A missed card is an evaluation outcome, not a crashed replay. Keep it
        // in the denominator and distinguish geometry from core admission.
        println!(
            "{}",
            serde_json::json!({
                "outcome": if first_geometry_index.is_none() { "noGeometry" }
                    else if first_matrix_index.is_none() { "notAdmitted" }
                    else { "recordingTooShort" },
                "frames": capture_ms.len(), "geometryFrames": geometry_frames,
                "firstGeometryIndex": first_geometry_index,
                "firstMatrixIndex": first_matrix_index,
                "correct": 0, "stableCells": 0, "wrongStableCells": [],
                "diagnostics": scanner.diagnostics(&id, base + Duration::from_secs_f64(index as f64 / fps))?,
            })
        );
        return Ok(());
    };
    eprintln!(
        "CAPTURE_DONE: {} ms; {} ready cells",
        captured.capture_elapsed_ms, captured.ready_cells
    );
    let model = isct_ocr::MatrixAlphabetService::default();
    let stage = Instant::now();
    let mut seen = vec![0usize; 70];
    while let Some(batch) = scanner.batch(&id)? {
        if let Some(directory) = std::env::var_os("MATRIX_CAPTURE_DUMP") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory)?;
            for (&cell, pixels) in batch.cells.iter().zip(&batch.images) {
                image::GrayImage::from_raw(96, 96, pixels.clone())
                    .ok_or("Invalid gray size")?
                    .save(directory.join(format!("cell-{cell:02}-{}.png", seen[cell])))?;
                seen[cell] += 1;
            }
        }
        let readings = model
            .recognize(&batch.images)?
            .into_iter()
            .map(|r| MatrixReading {
                text: r.text,
                confidence: r.confidence,
            })
            .collect();
        scanner.apply(&id, &batch, readings, now + stage.elapsed())?;
    }
    let processing_ms = stage.elapsed().as_secs_f64() * 1000.;
    let result = scanner.observe(&id, None, now + stage.elapsed())?;
    let wrong = result
        .cells
        .iter()
        .zip(expected.bytes())
        .enumerate()
        .filter(|(_, (actual, wanted))| actual.text.as_bytes() != [*wanted])
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    capture_ms.sort_by(f64::total_cmp);
    let median = capture_ms.get(capture_ms.len() / 2).copied().unwrap_or(0.);
    println!(
        "{}",
        serde_json::json!({"outcome":"processed","frames":capture_ms.len(),"geometryFrames":geometry_frames,"firstGeometryIndex":first_geometry_index,"live":live,"captureMs":captured.capture_elapsed_ms,"firstMatrixIndex":first_matrix_index,"sealedIndex":sealed_index,"timedOut":captured.timed_out,"readyCells":captured.ready_cells,"retainedCandidates":captured.total_samples,"captureFrameMsMedian":median,"processingMs":processing_ms,"correct":70-wrong.len(),"wrongStableCells":wrong.iter().filter(|&&i|result.cells[i].stable).copied().collect::<Vec<_>>(),"wrongCells":wrong,"stableCells":result.cells.iter().filter(|c|c.stable).count(),"diagnostics":scanner.diagnostics(&id,now+stage.elapsed())?})
    );
    Ok(())
}

#[cfg(not(matrix_ocr))]
fn main() {
    eprintln!("Matrix OCR runtime is unavailable for this target/build.");
}
