//! Deterministic geometry replay of a recorded camera sequence.
//!
//! `matrix_track_replay FRAME_DIRECTORY [--fps N] [--detect-only] > trace.jsonl`
//!
//! Input is either a desktop recording (`ISCT_MATRIX_RECORD_DIR`): raw
//! `.planes` (luma then R-G, exactly what the app analysed) and `.full`
//! keyframe luma, indexed with sizes and capture times by `frames.jsonl`; or
//! a directory of `*.jpg`/`*.png` frames in name order, spaced by `--fps`
//! (default 30) and reduced to the same 960 px planes. Every frame goes
//! through one `MatrixTracker`, as in the camera loop; `--detect-only` gives
//! each frame a fresh tracker.
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
    // A recording: frames.jsonl lines {"file", "kind", "frameId", "width",
    // "height", "capturedMs"} for "planes" and "full" files.
    struct Recorded {
        file: PathBuf,
        frame_id: u64,
        width: u32,
        height: u32,
        captured_ms: f64,
    }
    let mut recorded: Vec<Recorded> = Vec::new();
    let mut keyframes: HashMap<u64, Recorded> = HashMap::new();
    if let Ok(text) = std::fs::read_to_string(directory.join("frames.jsonl")) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line)?;
            let entry = Recorded {
                file: directory.join(v["file"].as_str().ok_or("file")?),
                frame_id: v["frameId"].as_u64().ok_or("frameId")?,
                width: v["width"].as_u64().ok_or("width")? as u32,
                height: v["height"].as_u64().ok_or("height")? as u32,
                captured_ms: v["capturedMs"].as_f64().ok_or("capturedMs")?,
            };
            match v["kind"].as_str() {
                Some("planes") => recorded.push(entry),
                Some("full") => {
                    keyframes.insert(entry.frame_id, entry);
                }
                _ => {}
            }
        }
    }
    let mut paths = std::fs::read_dir(&directory)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png"));
    paths.sort();
    if !recorded.is_empty() {
        paths = recorded.iter().map(|r| r.file.clone()).collect();
    }
    let origin = recorded.first().map_or(0., |r| r.captured_ms);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mut tracker = isct_ocr::MatrixTracker::default();
    for (index, path) in paths.iter().enumerate() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("bad file name")?;
        let start = Instant::now();
        let (frame_id, t, planes, full) = match recorded.get(index) {
            Some(r) => {
                let bytes = std::fs::read(&r.file)?;
                let (luma, chroma) = bytes.split_at(bytes.len() / 2);
                let planes = isct_ocr::MatrixAnalysisImage {
                    width: r.width,
                    height: r.height,
                    luma: luma.to_vec(),
                    chroma: chroma.to_vec(),
                };
                let full = match keyframes.get(&r.frame_id) {
                    Some(k) => Some((std::fs::read(&k.file)?, k.width, k.height)),
                    None => None,
                };
                (r.frame_id, (r.captured_ms - origin) / 1000., planes, full)
            }
            None => {
                let bytes = std::fs::read(path)?;
                let planes = isct_ocr::matrix_analysis_image(&bytes)?;
                let rgb = image::load_from_memory(&bytes)?.to_rgb8();
                let luma: Vec<u8> = rgb
                    .pixels()
                    .map(|p| {
                        ((19595 * p[0] as u32 + 38470 * p[1] as u32 + 7471 * p[2] as u32 + 32768)
                            >> 16) as u8
                    })
                    .collect();
                let full = Some((luma, rgb.width(), rgb.height()));
                (index as u64, index as f64 / fps, planes, full)
            }
        };
        let planes_ms = start.elapsed().as_secs_f64() * 1000.;
        if detect_only {
            tracker = isct_ocr::MatrixTracker::default();
        }
        let track = tracker.track(&planes.frame(t))?;
        let mut crop_ms = None;
        if let (true, Some(body), Some((luma, width, height))) = (track.keyframe, track.body, &full)
        {
            let start = Instant::now();
            isct_ocr::capture_matrix_keyframe(luma, *width, *height, body)?;
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
