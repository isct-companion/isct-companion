//! Real-time diagnostic camera/file replay through the production streaming scanner.
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use app_core::matrix_scan::{
        MatrixCameraPhase, MatrixCameraScanner, MatrixCaptureFrame, MatrixCaptureGlyph,
        MatrixPoint, MatrixReading,
    };
    use std::{
        path::PathBuf,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    let args: Vec<_> = std::env::args().collect();
    let directory = PathBuf::from(args.get(1).ok_or("frame directory required")?);
    let expected = std::fs::read_to_string(args.get(2).ok_or("expected text required")?)?
        .split_whitespace()
        .collect::<String>();
    if expected.len() != 70 || !expected.bytes().all(|c| c.is_ascii_uppercase()) {
        return Err("expected 70 uppercase letters".into());
    }
    let fps = args.get(3).map_or(Ok(5.), |s| s.parse::<f64>())?;
    if !fps.is_finite() || fps <= 0. {
        return Err("invalid FPS".into());
    }
    let live = args.get(4).is_some_and(|s| s == "live");
    let mut frames = std::fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("capture-"))
                && p.extension().is_some_and(|e| e == "jpg")
        })
        .collect::<Vec<_>>();
    frames.sort();
    if frames.is_empty() && !live {
        return Err("no capture-*.jpg frames".into());
    }
    let scanner = Arc::new(Mutex::new(MatrixCameraScanner::default()));
    let base = Instant::now();
    let id = scanner
        .lock()
        .map_err(|_| "scanner poisoned")?
        .begin_at(base, Duration::from_secs(10));
    let model = app_ocr::MatrixAlphabetService::default();
    let shared = scanner.clone();
    let worker_id = id.clone();
    let worker = std::thread::spawn(move || -> Result<_, String> {
        let execute = || -> Result<_, String> {
            loop {
                let state = shared
                    .lock()
                    .map_err(|e| e.to_string())?
                    .observe(&worker_id, None, Instant::now())
                    .map_err(|e| e.to_string())?;
                if state.phase == MatrixCameraPhase::Complete {
                    return Ok(state);
                }
                let batch = shared
                    .lock()
                    .map_err(|e| e.to_string())?
                    .batch(&worker_id, Instant::now())
                    .map_err(|e| e.to_string())?;
                if let Some(batch) = batch {
                    let readings = model
                        .recognize(&batch.images)
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .map(|r| MatrixReading {
                            text: r.text,
                            confidence: r.confidence,
                        })
                        .collect();
                    shared
                        .lock()
                        .map_err(|e| e.to_string())?
                        .apply(&worker_id, &batch, readings, Instant::now())
                        .map_err(|e| e.to_string())?;
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        };
        let result = execute();
        if result.is_err() {
            if let Ok(mut s) = shared.lock() {
                s.stop(&worker_id);
            }
        }
        result
    });
    let mut timings = vec![];
    let mut last = None;
    let mut modified = None;
    let mut geometry_frames = 0;
    let producer = (|| -> Result<(), Box<dyn std::error::Error>> {
        loop {
            if !scanner
                .lock()
                .map_err(|_| "scanner poisoned")?
                .is_active(&id)
            {
                break;
            }
            let state = scanner.lock().map_err(|_| "scanner poisoned")?.observe(
                &id,
                None,
                Instant::now(),
            )?;
            if state.phase == MatrixCameraPhase::Complete {
                break;
            }
            let path = if live {
                let path = directory.join("latest.jpg");
                let stamp = std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok());
                if stamp.is_none() || stamp == modified {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                modified = stamp;
                path
            } else {
                let index = (base.elapsed().as_secs_f64() * fps).floor() as usize;
                if last == Some(index) || index >= frames.len() {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                last = Some(index);
                frames[index].clone()
            };
            let stage = Instant::now();
            let frame = match app_ocr::capture_matrix_frame(&std::fs::read(path)?) {
                Ok(f) => {
                    geometry_frames += 1;
                    Some(MatrixCaptureFrame {
                        frame_id: geometry_frames as u32,
                        quads: f
                            .quads
                            .into_iter()
                            .map(|q| q.map(|[x, y]| MatrixPoint { x, y }))
                            .collect(),
                        glyphs: f
                            .glyphs
                            .into_iter()
                            .map(|g| MatrixCaptureGlyph {
                                gray: g.gray,
                                contrast: g.contrast,
                                sharpness: g.sharpness,
                                source_size: g.source_size,
                                centering: g.centering,
                                glare: g.glare,
                                acuity: g.acuity,
                            })
                            .collect(),
                    })
                }
                Err(app_ocr::MatrixOcrError::GridNotFound) => None,
                Err(e) => return Err(e.into()),
            };
            timings.push(stage.elapsed().as_secs_f64() * 1000.);
            scanner
                .lock()
                .map_err(|_| "scanner poisoned")?
                .observe(&id, frame, Instant::now())?;
        }
        Ok(())
    })();
    if producer.is_err() {
        scanner.lock().map_err(|_| "scanner poisoned")?.stop(&id);
    }
    let joined = worker.join().map_err(|_| "OCR worker panicked")?;
    producer?;
    let result = joined?;
    let wrong: Vec<_> = result
        .cells
        .iter()
        .zip(expected.bytes())
        .enumerate()
        .filter(|(_, (v, w))| v.text.as_bytes() != [*w])
        .map(|(i, _)| i)
        .collect();
    timings.sort_by(f64::total_cmp);
    let diagnostics = scanner
        .lock()
        .map_err(|_| "scanner poisoned")?
        .diagnostics(&id, Instant::now())?;
    println!(
        "{}",
        serde_json::json!({"outcome":"processed","live":live,"frames":timings.len(),"geometryFrames":geometry_frames,
        "captureMs":result.capture_elapsed_ms,"processingMs":diagnostics.processing_ms,"timedOut":result.timed_out,"readyCells":result.ready_cells,
        "captureFrameMsMedian":timings.get(timings.len()/2),"correct":70-wrong.len(),"wrongCells":wrong,
        "wrongStableCells":wrong.iter().filter(|&&i|result.cells[i].stable).collect::<Vec<_>>(),"stableCells":result.ready_cells,"diagnostics":diagnostics})
    );
    Ok(())
}
#[cfg(not(matrix_ocr))]
fn main() {
    eprintln!("Matrix OCR unavailable for this target");
    std::process::exit(1);
}
