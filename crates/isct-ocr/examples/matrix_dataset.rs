//! Local-only dataset export through the production camera geometry/crop path.
//! No recognition: fixed cell positions are provenance, never predicted labels.
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{io::Write, path::PathBuf};
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or("Pass frame directory")?);
    let output = PathBuf::from(args.next().ok_or("Pass new output directory")?);
    let manual = args
        .next()
        .map(|file| {
            std::fs::read(file)
                .map_err(Box::<dyn std::error::Error>::from)
                .and_then(|bytes| {
                    serde_json::from_slice::<std::collections::BTreeMap<String, [[f64; 2]; 4]>>(
                        &bytes,
                    )
                    .map_err(Into::into)
                })
        })
        .transpose()?
        .unwrap_or_default();
    std::fs::create_dir(&output)?;
    let mut paths = std::fs::read_dir(input)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("capture-"))
            && path
                .extension()
                .is_some_and(|ext| ext == "jpg" || ext == "png")
    });
    paths.sort();
    if paths.is_empty() {
        return Err("No capture-*.jpg/png frames".into());
    }
    let mut manifest = std::io::BufWriter::new(std::fs::File::create(output.join("frames.jsonl"))?);
    for path in paths {
        let name = path
            .file_stem()
            .ok_or("Invalid frame name")?
            .to_string_lossy();
        let bytes = std::fs::read(&path)?;
        let native = isct_ocr::capture_matrix_frame(&bytes);
        let native_quads = match &native {
            Ok(frame) => Some(frame.quads.clone()),
            Err(isct_ocr::MatrixOcrError::GridNotFound) => None,
            Err(error) => return Err(error.to_string().into()),
        };
        let assisted = manual.get(name.as_ref());
        let captured = if let Some(&body) = assisted {
            isct_ocr::capture_matrix_training_frame(&bytes, body)
        } else {
            native
        };
        let report = match captured {
            Ok(frame) => {
                if frame.glyphs.len() != 70 || frame.quads.len() != 70 {
                    return Err("Expected exactly 70 production cells".into());
                }
                let mut cells = Vec::with_capacity(70);
                for (cell, glyph) in frame.glyphs.into_iter().enumerate() {
                    let file = format!("{name}-{cell:02}.png");
                    image::GrayImage::from_raw(96, 96, glyph.gray)
                        .ok_or("Invalid production crop")?
                        .save(output.join(&file))?;
                    cells.push(serde_json::json!({"cell":cell,"file":file,
                        "contrast":glyph.contrast,"sharpness":glyph.sharpness,
                        "sourceSize":glyph.source_size,"centering":glyph.centering,
                        "glare":glyph.glare}));
                }
                serde_json::json!({"frame":name,"outcome":"located",
                    "assisted":assisted.is_some(),"nativeQuads":native_quads,
                    "quads":frame.quads,"cells":cells,"timing":{
                        "decodeMs":frame.timing.decode_ms,"geometryMs":frame.timing.geometry_ms,
                        "cropMs":frame.timing.preprocess_ms,"totalMs":frame.timing.total_ms}})
            }
            Err(isct_ocr::MatrixOcrError::GridNotFound) => {
                serde_json::json!({"frame":name,"outcome":"noGeometry"})
            }
            Err(error) => return Err(error.into()),
        };
        serde_json::to_writer(&mut manifest, &report)?;
        manifest.write_all(b"\n")?;
        manifest.flush()?;
    }
    Ok(())
}

#[cfg(not(matrix_ocr))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Matrix OCR backend unavailable".into())
}
