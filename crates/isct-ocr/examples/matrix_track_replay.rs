//! Deterministic geometry replay of a recorded camera sequence.
//!
//! `matrix_track_replay FRAME_DIRECTORY [--fps N] [--detect-only] > trace.jsonl`
//!
//! Frames are `*.jpg`/`*.png` in name order. A `frames.jsonl` written by the
//! desktop recorder supplies capture timestamps; otherwise `--fps` (default 30)
//! spaces them evenly. Each frame is reduced to the analysis planes the app
//! sends (luma and R-G at 960 px) and passed to one `MatrixTracker`, exactly as
//! the camera loop does; `--detect-only` gives every frame a fresh tracker.
//! Each output line names the frame, its capture time, the tracker mode, the
//! body corners of every stage that ran, the final grid and the measured
//! costs: `total` is the geometry loop alone, `crop` the 70-cell keyframe cut
//! from the full-resolution luma when the frame qualifies as a keyframe.
//! `scripts/ocr/matrix_track_report.py` turns this into overlays, a
//! capture-to-draw timeline and the summary metrics.
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde_json::json;
    use std::{collections::HashMap, io::Write, path::PathBuf, time::Instant};
    let args: Vec<_> = std::env::args().skip(1).collect();
    let directory = PathBuf::from(args.first().ok_or("pass a frame directory")?);
    let fps: f64 = match args.iter().position(|a| a == "--fps") {
        Some(i) => args.get(i + 1).ok_or("--fps needs a value")?.parse()?,
        None => 30.,
    };
    let detect_only = args.iter().any(|a| a == "--detect-only");
    let mut paths = std::fs::read_dir(&directory)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png"));
    paths.sort();
    // Recorder lines: {"file": "000012.jpg", "frameId": 12, "capturedMs": 1234.5}.
    let mut stamps = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(directory.join("frames.jsonl")) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let value: serde_json::Value = serde_json::from_str(line)?;
            if let (Some(file), Some(ms)) = (value["file"].as_str(), value["capturedMs"].as_f64()) {
                stamps.insert(file.to_owned(), (value["frameId"].as_u64(), ms));
            }
        }
    }
    let origin = paths
        .first()
        .and_then(|p| stamps.get(p.file_name()?.to_str()?))
        .map_or(0., |(_, ms)| *ms);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mut tracker = isct_ocr::MatrixTracker::default();
    for (index, path) in paths.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("bad file name")?;
        let (frame_id, t) = stamps
            .get(name)
            .map_or((index as u64, index as f64 / fps), |s| {
                (s.0.unwrap_or(index as u64), (s.1 - origin) / 1000.)
            });
        let bytes = std::fs::read(path)?;
        let start = Instant::now();
        let planes = isct_ocr::matrix_analysis_image(&bytes)?;
        let planes_ms = start.elapsed().as_secs_f64() * 1000.;
        if detect_only {
            tracker = isct_ocr::MatrixTracker::default();
        }
        let track = tracker.track(&planes.frame(t))?;
        let mut crop_ms = None;
        if let (true, Some(body)) = (track.keyframe, track.body) {
            let full = image::load_from_memory(&bytes)?.to_rgb8();
            let luma: Vec<u8> = full
                .pixels()
                .map(|p| {
                    ((19595 * p[0] as u32 + 38470 * p[1] as u32 + 7471 * p[2] as u32 + 32768) >> 16)
                        as u8
                })
                .collect();
            let start = Instant::now();
            isct_ocr::capture_matrix_keyframe(&luma, full.width(), full.height(), body)?;
            crop_ms = Some(start.elapsed().as_secs_f64() * 1000.);
        }
        let mut stages: serde_json::Map<_, _> = track
            .stages
            .iter()
            .map(|(stage, quad)| ((*stage).to_owned(), json!(quad)))
            .collect();
        if let Some(display) = track.display {
            stages.insert("smoothed".into(), json!(display));
        }
        writeln!(
            output,
            "{}",
            json!({
                "frame": name, "frameId": frame_id, "t": t, "pipeline": "rust-grid",
                "mode": track.mode, "keyframe": track.keyframe, "stages": stages,
                "final": track.body,
                "info": {"residual": track.residual, "reason": track.reason},
                "ms": {"planes": planes_ms, "total": track.elapsed_ms, "crop": crop_ms},
            })
        )?;
    }
    Ok(())
}
#[cfg(not(matrix_ocr))]
fn main() {
    eprintln!("Matrix OCR unavailable for this target");
    std::process::exit(1);
}
