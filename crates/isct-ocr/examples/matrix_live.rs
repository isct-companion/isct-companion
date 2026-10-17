//! Local camera diagnostic: a capture adapter atomically replaces latest.jpg.
//! Keep source frames local; print selected cells and measured scores for debugging.
#[cfg(matrix_ocr)]
#[allow(dead_code)]
#[path = "../../isct-core/src/matrix_scan.rs"]
mod matrix_scan;
#[cfg(matrix_ocr)]
use isct_ocr::{MatrixOcrError, MatrixOcrService};
#[cfg(matrix_ocr)]
use matrix_scan::{MatrixPoint, MatrixReading, MatrixScanTiming, MatrixScanner};
#[cfg(matrix_ocr)]
use ort::{session::Session, value::Tensor};
// Reuse the actual geometry implementation in this diagnostic; the candidate
// classifier remains external until its live accuracy has been measured.
#[cfg(matrix_ocr)]
#[allow(dead_code)]
#[path = "../src/engine/contours.rs"]
mod contours;
#[cfg(matrix_ocr)]
#[allow(dead_code)]
#[path = "../src/engine/geometry.rs"]
mod geometry;
#[cfg(matrix_ocr)]
fn rectified_gray(
    image: &image::RgbImage,
    quad: [[f64; 2]; 4],
) -> Result<image::GrayImage, Box<dyn Error>> {
    use imageproc::geometric_transformations::{warp_into, Interpolation, Projection};
    let corners = quad.map(|[x, y]| geometry::Point {
        x: x * image.width() as f64,
        y: y * image.height() as f64,
    });
    let destination = [(0f32, 0f32), (95., 0.), (95., 95.), (0., 95.)];
    // Match PIL's pixel-center convention and bilinear training input.
    let source = destination.map(|(x, y)| {
        let p = geometry::project(
            &corners,
            0.09 + (x as f64 + 0.5) / 95. * 0.82,
            0.09 + (y as f64 + 0.5) / 95. * 0.82,
        );
        ((p.x - 0.5) as f32, (p.y - 0.5) as f32)
    });
    let projection = Projection::from_control_points(source, destination)
        .ok_or("Cannot rectify classifier cell")?;
    let mut crop = image::RgbImage::new(96, 96);
    warp_into(
        image,
        projection,
        Interpolation::Bilinear,
        imageproc::geometric_transformations::Border::Constant(image::Rgb([255, 255, 255])),
        &mut crop,
    );
    // image::grayscale uses Rec.709; these coefficients match PIL/TorchVision.
    Ok(image::GrayImage::from_fn(96, 96, |x, y| {
        let p = crop.get_pixel(x, y);
        image::Luma([
            ((19595 * p[0] as u32 + 38470 * p[1] as u32 + 7471 * p[2] as u32 + 32768) >> 16) as u8,
        ])
    }))
}
#[cfg(matrix_ocr)]
fn alphabet_frame(
    service: &MatrixOcrService,
    session: &mut Session,
    bytes: &[u8],
    indices: &[usize],
) -> Result<isct_ocr::MatrixLiveFrame, Box<dyn Error>> {
    let start = Instant::now();
    let quads = std::cell::RefCell::new(None);
    // Early cancellation obtains production geometry without running Paddle.
    let result = service.scan_live_frame(
        bytes,
        indices,
        |value| {
            *quads.borrow_mut() = Some(value.to_vec());
            false
        },
        |_| true,
    );
    if !matches!(result, Err(MatrixOcrError::Cancelled)) {
        result?;
        return Err("Missing geometry cancellation".into());
    }
    let quads = quads.into_inner().ok_or("No matrix geometry")?;
    let image = image::load_from_memory(bytes)?.to_rgb8();
    let geometry_ms = start.elapsed().as_secs_f64() * 1000.;
    let stage = Instant::now();
    let dump = std::env::var_os("MATRIX_INPUT_DUMP").map(PathBuf::from);
    if let Some(path) = &dump {
        std::fs::create_dir_all(path)?;
    }
    let tensors: Vec<Vec<f32>> = indices
        .iter()
        .map(|&i| -> Result<Vec<f32>, Box<dyn Error>> {
            let gray = rectified_gray(&image, quads[i])?;
            if let Some(path) = &dump {
                if let Err(error) = gray.save(path.join(format!("cell-{i:02}.png"))) {
                    eprintln!("Input dump failed: {error}");
                }
            }
            Ok(gray.pixels().map(|p| p[0] as f32 / 127.5 - 1.).collect())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let preprocess_ms = stage.elapsed().as_secs_f64() * 1000.;
    let stage = Instant::now();
    let mut cells = Vec::new();
    for batch in tensors.chunks(10) {
        let input = Tensor::from_array((
            [batch.len(), 1, 96, 96],
            batch.iter().flatten().copied().collect::<Vec<_>>(),
        ))?;
        let output = session.run(ort::inputs![input])?;
        let (shape, data) = output[0].try_extract_tensor::<f32>()?;
        if shape.as_ref() != [batch.len() as i64, 27] {
            return Err("Invalid alphabet output shape".into());
        }
        for row in data.chunks_exact(27) {
            if row.iter().any(|v| !v.is_finite()) {
                return Err("Invalid alphabet logits".into());
            }
            let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let sum: f32 = row.iter().map(|v| (v - max).exp()).sum();
            let winner = row
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i)
                .ok_or("No alphabet output")?;
            cells.push(isct_ocr::Recognition {
                text: if winner == 26 {
                    String::new()
                } else {
                    ((b'A' + winner as u8) as char).to_string()
                },
                confidence: ((row[winner] - max).exp() / sum) as f64,
            });
        }
    }
    let inference_ms = stage.elapsed().as_secs_f64() * 1000.;
    Ok(isct_ocr::MatrixLiveFrame {
        cells,
        quads,
        timing: isct_ocr::MatrixFrameTiming {
            geometry_ms,
            preprocess_ms,
            inference_ms,
            total_ms: start.elapsed().as_secs_f64() * 1000.,
            ..Default::default()
        },
    })
}
#[cfg(matrix_ocr)]
use std::{
    error::Error,
    path::PathBuf,
    time::{Duration, Instant},
};
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn Error>> {
    let directory = PathBuf::from(std::env::args().nth(1).ok_or("Pass capture directory")?);
    let service = MatrixOcrService::default();
    let mut alphabet = std::env::args()
        .nth(2)
        .map(|path| -> Result<Session, Box<dyn Error>> {
            Ok(Session::builder()?
                .with_intra_threads(2)?
                .with_inter_threads(1)?
                .commit_from_file(path)?)
        })
        .transpose()?;
    if let Some(session) = alphabet.as_mut() {
        let _ = session.run(ort::inputs![Tensor::from_array((
            [10, 1, 96, 96],
            vec![0f32; 10 * 96 * 96]
        ))?])?;
    } else {
        service.warm_up()?;
    }
    if let Some(image_path) = std::env::args().nth(4) {
        let session = alphabet
            .as_mut()
            .ok_or("Static classifier probe requires model")?;
        let frame = alphabet_frame(
            &service,
            session,
            &std::fs::read(image_path)?,
            &(0..70).collect::<Vec<_>>(),
        )?;
        println!(
            "{}",
            serde_json::json!({"cells":frame.cells,"quads":frame.quads,"timing":frame.timing})
        );
        return Ok(());
    }
    eprintln!("READY: model warmed; waiting for fresh camera frames");
    let mut scanner = if alphabet.is_some() {
        MatrixScanner::full_frame()
    } else {
        MatrixScanner::default()
    };
    let id = scanner.begin();
    let mut previous = None;
    let mut started: Option<Instant> = None;
    let lifetime = Instant::now();
    let mut first_readable: Option<Instant> = None;
    let mut saved_frames = 0;
    let timeout = std::env::args()
        .nth(3)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(12);
    while lifetime.elapsed() < Duration::from_secs(90) {
        if started.is_some_and(|start| start.elapsed() > Duration::from_secs(timeout)) {
            eprintln!(
                "TIMEOUT: {} stable cells",
                scanner.diagnostics(&id)?.stable_cells
            );
            break;
        }
        let path = directory.join("latest.jpg");
        let Ok(metadata) = std::fs::metadata(&path) else {
            std::thread::sleep(Duration::from_millis(30));
            continue;
        };
        let modified = metadata.modified()?;
        if previous == Some(modified) {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        previous = Some(modified);
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let plan = scanner.plan(&id)?;
        let frame_started = Instant::now();
        let frame = match if let Some(session) = alphabet.as_mut() {
            alphabet_frame(&service, session, &bytes, &plan.cells).map_err(|error| {
                error
                    .downcast::<MatrixOcrError>()
                    .map(|e| *e)
                    .unwrap_or_else(|e| MatrixOcrError::Model(e.to_string()))
            })
        } else {
            service.scan_live_frame(&bytes, &plan.cells, |_| true, |_| true)
        } {
            Ok(frame) => frame,
            Err(MatrixOcrError::GridNotFound) => {
                scanner.observe(&id, &plan, None, MatrixScanTiming::default())?;
                continue;
            }
            Err(MatrixOcrError::InvalidImage(_)) => continue,
            Err(error) => return Err(error.into()),
        };
        let start = *started.get_or_insert(frame_started);
        let frame_number = scanner.diagnostics(&id)?.frames;
        let output:Vec<_>=plan.cells.iter().zip(&frame.cells).map(|(i,c)|serde_json::json!({"cell":format!("{}{}",(b'A'+(i%10) as u8) as char,i/10+1),"text":c.text,"confidence":c.confidence})).collect();
        std::fs::write(
            directory.join(format!("geometry-{frame_number:03}.json")),
            serde_json::to_vec(&frame.quads)?,
        )?;
        let t = frame.timing;
        let snapshot = scanner.observe(
            &id,
            &plan,
            Some((
                frame
                    .quads
                    .into_iter()
                    .map(|q| q.map(|[x, y]| MatrixPoint { x, y }))
                    .collect(),
                frame
                    .cells
                    .into_iter()
                    .map(|r| MatrixReading {
                        text: r.text,
                        confidence: r.confidence,
                    })
                    .collect(),
            )),
            MatrixScanTiming {
                decode_ms: t.decode_ms,
                geometry_ms: t.geometry_ms,
                preprocess_ms: t.preprocess_ms,
                inference_ms: t.inference_ms,
                total_ms: t.total_ms,
            },
        )?;
        if snapshot.sampled_cells > 0 {
            first_readable.get_or_insert(frame_started);
            // Bound source retention by readable frames, so waiting on an empty
            // camera cannot consume the whole diagnostic allowance.
            if saved_frames < 80 {
                std::fs::write(
                    directory.join(format!("frame-{frame_number:03}.jpg")),
                    &bytes,
                )?;
                saved_frames += 1;
            }
        }
        println!(
            "{}",
            serde_json::json!({"frame":frame_number+1,"elapsedMs":start.elapsed().as_secs_f64()*1000.,"phase":snapshot.phase,"accepted":snapshot.sampled_cells>0,"complete":snapshot.complete,"readableElapsedMs":first_readable.map(|s|s.elapsed().as_secs_f64()*1000.),"stable":snapshot.stable_cells,"totalMs":t.total_ms,"inferenceMs":t.inference_ms,"results":output,"votes":snapshot.cells})
        );
        if snapshot.complete {
            eprintln!("COMPLETE {} ms", start.elapsed().as_millis());
            break;
        }
        if start.elapsed() > Duration::from_secs(timeout) {
            eprintln!("TIMEOUT: {} stable cells", snapshot.stable_cells);
            break;
        }
    }
    std::fs::write(
        directory.join("diagnostics.json"),
        serde_json::to_vec_pretty(&scanner.diagnostics(&id)?)?,
    )?;
    Ok(())
}

#[cfg(not(matrix_ocr))]
fn main() {
    eprintln!("Matrix OCR runtime is unavailable for this target/build.");
}
