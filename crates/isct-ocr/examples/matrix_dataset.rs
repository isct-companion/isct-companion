//! Local-only dataset export through the production camera path.
//!
//! `matrix_dataset VIDEO OUT [--cache FRAMES_YUV]`
//!
//! One `MatrixTracker` follows the whole recording exactly as in a camera
//! session (4:2:0 frames through `MatrixVision::convert`), and every frame the
//! session would cut (`keyframe_ready`) is cut by the production `MatrixVision::cut`:
//! the same 96×96 grayscale cells, at the same positions, that Core would
//! queue for the model. Cell positions are the only provenance; labels come
//! later from the card's human-checked truth (`scripts/ocr/matrix_video.py`).
//!
//! `OUT/glyphs.u8` holds the cells, 70 × 9216 bytes per keyframe in row order
//! (A1…J1, A2…). `OUT/frames.jsonl` has one line per video frame: time,
//! whether the grid was located, the measured body corners and tracking cost;
//! keyframe lines add their index into `glyphs.u8` and, per cell, Core's
//! quality measurements, its admission verdict (`matrix_cell_admissible`),
//! the bundled model's reading and the cell's edge acuity
//! `[contrast, sharpness, sourceSize, centering, glare, admissible, text, confidence, acuity]`;
//! `sharp` is Core's verdict on the whole keyframe (`matrix_frame_sharp`).
//! The reading only lets the dataset check indexing and report the current
//! model's errors; it never becomes a label. Outputs are card images.
#[cfg(matrix_ocr)]
#[path = "common/video.rs"]
mod video;

#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use isct_core::matrix_scan::{keyframe_ready, matrix_cell_admissible, matrix_frame_sharp};
    use serde_json::json;
    use std::{io::Write, path::PathBuf};
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(source), Some(out)) = (args.first(), args.get(1)) else {
        return Err("usage: matrix_dataset VIDEO OUT [--cache FRAMES_YUV]".into());
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out)?;
    let cache = args
        .iter()
        .position(|a| a == "--cache")
        .and_then(|i| args.get(i + 1))
        .map_or_else(|| out.join("frames.yuv"), PathBuf::from);
    let mut video = video::Video::open(source.as_ref(), &cache)?;
    let vision = isct_ocr::matrix_vision();
    let mut tracker = vision.tracker();
    let mut glyphs = std::io::BufWriter::new(std::fs::File::create(out.join("glyphs.u8"))?);
    let mut lines = std::io::BufWriter::new(std::fs::File::create(out.join("frames.jsonl"))?);
    let (mut buffer, mut keyframes) = (Vec::new(), 0usize);
    for index in 0..video.times.len() {
        video.read(index, &mut buffer)?;
        let picture = vision.convert(&video.yuv(&buffer))?;
        let track = tracker.track(picture.planes(), video.times[index])?;
        let mut row = json!({"frame": index, "t": video.times[index],
            "located": track.body.is_some(), "trackMs": track.elapsed_ms,
            "body": track.body.map(|b| b.map(|p| [p.x, p.y]))});
        if let (true, Some(body)) = (keyframe_ready(&track), track.body) {
            let cut = vision.cut(picture.full(), body)?;
            let mut readings = Vec::with_capacity(cut.glyphs.len());
            for chunk in cut.glyphs.chunks(10) {
                let images: Vec<_> = chunk.iter().map(|g| g.gray.clone()).collect();
                readings.extend(vision.recognize(&images)?);
            }
            for glyph in &cut.glyphs {
                glyphs.write_all(&glyph.gray)?;
            }
            row["keyframe"] = json!(keyframes);
            row["sharp"] = json!(matrix_frame_sharp(&cut.glyphs));
            row["cells"] = cut
                .glyphs
                .iter()
                .zip(&readings)
                .map(|(g, r)| {
                    json!([
                        g.contrast,
                        g.sharpness,
                        g.source_size,
                        g.centering,
                        g.glare,
                        matrix_cell_admissible(g),
                        r.text,
                        r.confidence,
                        g.acuity
                    ])
                })
                .collect();
            keyframes += 1;
        }
        writeln!(lines, "{row}")?;
        if index % 200 == 0 {
            eprintln!("frame {index}/{}: {keyframes} keyframes", video.times.len());
        }
    }
    glyphs.flush()?;
    lines.flush()?;
    eprintln!(
        "{keyframes} keyframes from {} frames ({:.1} s)",
        video.times.len(),
        video.duration
    );
    Ok(())
}

#[cfg(not(matrix_ocr))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Matrix OCR backend unavailable".into())
}
