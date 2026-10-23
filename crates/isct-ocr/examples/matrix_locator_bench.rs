//! Local-only frame benchmark. Arguments: frame directory; optional OCR flag.
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{io::Write, time::Instant};
    let directory = std::env::args().nth(1).ok_or("pass frame directory")?;
    let recognize = std::env::args().any(|a| a == "--ocr");
    let model = isct_ocr::MatrixAlphabetService::default();
    let mut paths = std::fs::read_dir(directory)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png"));
    paths.sort();
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mode = std::env::var("MATRIX_GRAY").unwrap_or_else(|_| "luma".into());
    let mut config = isct_ocr::MatrixCaptureConfig::default();
    match mode.as_str() {
        "luma" => {}
        "green" => config.locator.gray_weights = [0, 1, 0],
        "blue" => config.locator.gray_weights = [0, 0, 1],
        "mean" => config.locator.gray_weights = [1, 1, 1],
        "red1" => config.locator.red_chroma_weight = 256,
        "red2" => config.locator.red_chroma_weight = 512,
        "red4" => config.locator.red_chroma_weight = 1024,
        _ => return Err("unknown MATRIX_GRAY".into()),
    }
    if let Ok(size) = std::env::var("MATRIX_SEARCH_SIZE") {
        config.locator.search_max_side = size.parse()?;
    }
    config.locator.color_refinement = std::env::var_os("MATRIX_COLOR_REFINE").is_some();
    config.glyph_projection = match std::env::var("MATRIX_GLYPH").as_deref().unwrap_or("luma") {
        "luma" => isct_ocr::MatrixGlyphProjection::Luma,
        "red-chroma" => isct_ocr::MatrixGlyphProjection::RedChroma,
        "paper-distance" => isct_ocr::MatrixGlyphProjection::PaperDistance,
        _ => return Err("unknown MATRIX_GLYPH".into()),
    };
    let hints: std::collections::HashMap<String, [[f64; 2]; 4]> =
        match std::env::var("MATRIX_HINTS") {
            Ok(path) => serde_json::from_slice(&std::fs::read(path)?)?,
            Err(_) => std::collections::HashMap::new(),
        };
    let tracking = std::env::args().any(|a| a == "--track");
    let mut previous = None;
    for path in paths {
        let bytes = std::fs::read(&path)?;
        let start = Instant::now();
        let name = path.file_name().ok_or("missing name")?.to_string_lossy();
        let hint = hints
            .get(name.as_ref())
            .copied()
            .or(if tracking { previous } else { None });
        #[cfg(feature = "training-tools")]
        let direct = std::env::args().any(|a| a == "--direct-hints");
        #[cfg(not(feature = "training-tools"))]
        let direct = false;
        let frame = if direct {
            #[cfg(feature = "training-tools")]
            {
                hint.ok_or(isct_ocr::MatrixOcrError::GridNotFound)
                    .and_then(|q| isct_ocr::capture_matrix_training_frame(&bytes, q))
            }
            #[cfg(not(feature = "training-tools"))]
            {
                Err(isct_ocr::MatrixOcrError::Unavailable)
            }
        } else {
            isct_ocr::capture_matrix_frame_configured(&bytes, hint, config)
        };
        let ms = start.elapsed().as_secs_f64() * 1000.;
        let row = match frame {
            Ok(frame) => {
                previous = Some([
                    frame.quads[0][0],
                    frame.quads[9][1],
                    frame.quads[69][2],
                    frame.quads[60][3],
                ]);
                let inference = Instant::now();
                let readings = if recognize {
                    let mut readings = Vec::new();
                    for batch in frame.glyphs.chunks(10) {
                        readings.extend(model.recognize(
                            &batch.iter().map(|g| g.gray.clone()).collect::<Vec<_>>(),
                        )?);
                    }
                    Some(readings)
                } else {
                    None
                };
                serde_json::json!({"frame":name,"ms":ms,"timing":frame.timing,"quads":frame.quads,"readings":readings,"ocr_ms":inference.elapsed().as_secs_f64()*1000.})
            }
            Err(isct_ocr::MatrixOcrError::GridNotFound) => {
                previous = None;
                serde_json::json!({"frame":name,"ms":ms})
            }
            Err(e) => return Err(e.into()),
        };
        writeln!(output, "{row}")?;
        output.flush()?;
    }
    Ok(())
}
#[cfg(not(matrix_ocr))]
fn main() {
    eprintln!("Matrix OCR unavailable");
    std::process::exit(1);
}
