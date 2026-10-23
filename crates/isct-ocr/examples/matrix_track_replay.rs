//! Deterministic geometry replay of a recorded camera sequence.
//!
//! `matrix_track_replay FRAME_DIRECTORY [--fps N] > trace.jsonl`
//!
//! Frames are `*.jpg`/`*.png` in name order. A `frames.jsonl` written by the
//! desktop recorder supplies capture timestamps; otherwise `--fps` (default 30)
//! spaces them evenly. Each output line names the frame, its capture time, the
//! body corners of every geometry stage that ran, the final grid body and the
//! measured stage durations. `scripts/ocr/matrix_track_report.py` turns this
//! into overlays, a capture-to-draw timeline and the summary metrics.
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use serde_json::json;
    use std::{collections::HashMap, io::Write, path::PathBuf};
    let args: Vec<_> = std::env::args().skip(1).collect();
    let directory = PathBuf::from(args.first().ok_or("pass a frame directory")?);
    let fps: f64 = match args.iter().position(|a| a == "--fps") {
        Some(i) => args.get(i + 1).ok_or("--fps needs a value")?.parse()?,
        None => 30.,
    };
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
    let mut previous = None;
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
        let start = std::time::Instant::now();
        let trace = isct_ocr::trace_matrix_frame(&bytes, previous);
        let total_ms = start.elapsed().as_secs_f64() * 1000.;
        let stages: serde_json::Map<_, _> = trace
            .stages
            .iter()
            .map(|(stage, quad)| ((*stage).to_owned(), json!(quad)))
            .collect();
        let (body, timing) = match &trace.frame {
            Ok(frame) => {
                let q = &frame.quads;
                let body = [q[0][0], q[9][1], q[69][2], q[60][3]];
                (Some(body), Some(&frame.timing))
            }
            Err(isct_ocr::MatrixOcrError::GridNotFound) => (None, None),
            Err(error) => return Err(error.to_string().into()),
        };
        previous = body;
        writeln!(
            output,
            "{}",
            json!({
                "frame": name, "frameId": frame_id, "t": t, "pipeline": "rust-current",
                "stages": stages, "final": body,
                "ms": {
                    "decode": timing.map(|t| t.decode_ms),
                    "geometry": timing.map(|t| t.geometry_ms),
                    "crop": timing.map(|t| t.preprocess_ms),
                    "total": total_ms,
                },
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
