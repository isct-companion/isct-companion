mod alphabet;
mod contours;
pub use alphabet::{capture_matrix_frame, MatrixAlphabetService};
#[cfg(debug_assertions)]
mod debug_crops;
mod geometry;
mod preprocess;

#[cfg(test)]
#[path = "../tests/engine__private_parity.rs"]
mod private_parity;

#[cfg(test)]
#[path = "../tests/engine.rs"]
mod tests;

use crate::{recognition, MatrixOcrError, OcrProgress, Recognition, MATRIX_CELLS};
use image::{imageops::FilterType, DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use std::{
    io::Cursor,
    sync::{Arc, Mutex, TryLockError},
};

const MODEL: &[u8] = include_bytes!("../../models/paddle/en-ppocrv5.onnx");
const DICTIONARY: &str = include_str!("../../models/paddle/characters.json");
use crate::MAX_IMAGE_BYTES;

struct Paddle {
    session: Session,
    characters: Vec<String>,
}
impl Paddle {
    fn new() -> Result<Self, MatrixOcrError> {
        let session = Session::builder()
            .map_err(model_error)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(model_error)?
            .with_intra_threads(2)
            .map_err(model_error)?
            .with_inter_threads(1)
            .map_err(model_error)?
            .commit_from_memory(MODEL)
            .map_err(model_error)?;
        Ok(Self {
            session,
            characters: serde_json::from_str(DICTIONARY).map_err(model_error)?,
        })
    }
    fn recognize(&mut self, tensor: Vec<f32>) -> Result<Recognition, MatrixOcrError> {
        self.recognize_batch(vec![tensor], 320)?
            .into_iter()
            .next()
            .ok_or_else(|| MatrixOcrError::Model("Missing recognition result".into()))
    }
    /// Keep the narrow fast path, but a weak result gets the established wide
    /// context. The original raw text is retained for the existing review rules.
    fn recognize_glyphs(
        &mut self,
        glyphs: &[&image::GrayImage],
    ) -> Result<(Vec<Recognition>, f64, f64), MatrixOcrError> {
        use std::time::Instant;
        let stage = Instant::now();
        let tensors = glyphs
            .iter()
            .map(|glyph| preprocess::prepare_width(glyph, 64).tensor)
            .collect();
        let mut preprocessing = stage.elapsed().as_secs_f64() * 1000.;
        let stage = Instant::now();
        let mut raw = self.recognize_batch(tensors, 64)?;
        let mut inference = stage.elapsed().as_secs_f64() * 1000.;
        let stage = Instant::now();
        let retry: Vec<_> = raw
            .iter()
            .enumerate()
            .filter(|(_, r)| recognition::accept(&r.text, r.confidence).confidence < 0.8)
            .map(|(i, _)| (i, preprocess::prepare(glyphs[i]).tensor))
            .collect();
        preprocessing += stage.elapsed().as_secs_f64() * 1000.;
        let stage = Instant::now();
        let (positions, tensors): (Vec<_>, Vec<_>) = retry.into_iter().unzip();
        for (i, result) in positions
            .into_iter()
            .zip(self.recognize_batch(tensors, 320)?)
        {
            raw[i] = result;
        }
        inference += stage.elapsed().as_secs_f64() * 1000.;
        Ok((raw, preprocessing, inference))
    }
    fn recognize_cells(
        &mut self,
        binaries: &[preprocess::Binary],
    ) -> Result<(Vec<Recognition>, f64, f64), MatrixOcrError> {
        let active: Vec<_> = binaries
            .iter()
            .enumerate()
            .filter(|(_, b)| b.has_glyph())
            .map(|(i, _)| i)
            .collect();
        let glyphs: Vec<_> = active.iter().map(|&i| &binaries[i].cleaned).collect();
        let (raw, mut preprocessing, mut inference) = self.recognize_glyphs(&glyphs)?;
        let mut accepted = vec![Recognition::default(); binaries.len()];
        for (i, r) in active.into_iter().zip(raw) {
            accepted[i] = recognition::accept(&r.text, r.confidence);
        }
        let positions: Vec<_> = binaries
            .iter()
            .enumerate()
            .filter(|(i, b)| !accepted[*i].text.is_empty() && b.cleaned != b.original)
            .map(|(i, _)| i)
            .collect();
        let originals: Vec<_> = positions.iter().map(|&i| &binaries[i].original).collect();
        let (raw, p, i) = self.recognize_glyphs(&originals)?;
        preprocessing += p;
        inference += i;
        for (i, r) in positions.into_iter().zip(raw) {
            accepted[i] = recognition::review(accepted[i].clone(), &r.text);
        }
        Ok((accepted, preprocessing, inference))
    }
    fn recognize_batch(
        &mut self,
        tensors: Vec<Vec<f32>>,
        width: usize,
    ) -> Result<Vec<Recognition>, MatrixOcrError> {
        let batch = tensors.len();
        if batch == 0 {
            return Ok(vec![]);
        }
        if ![64, 320].contains(&width) || tensors.iter().any(|t| t.len() != 3 * 48 * width) {
            return Err(MatrixOcrError::Model(
                "Invalid recognition batch shape".into(),
            ));
        }
        let input = Tensor::from_array((
            [batch, 3, 48, width],
            tensors.into_iter().flatten().collect::<Vec<_>>(),
        ))
        .map_err(model_error)?;
        let output = self.session.run(ort::inputs![input]).map_err(model_error)?;
        let Some((_, output)) = output.into_iter().next() else {
            return Err(MatrixOcrError::Model(
                "Unexpected Paddle output shape".into(),
            ));
        };
        let (shape, data) = output.try_extract_tensor::<f32>().map_err(model_error)?;
        if shape.len() != 3
            || shape[0] != batch as i64
            || shape[1] <= 0
            || shape[2] != self.characters.len() as i64
        {
            return Err(MatrixOcrError::Model(
                "Unexpected Paddle output shape".into(),
            ));
        }
        let steps = usize::try_from(shape[1]).map_err(model_error)?;
        let stride = steps
            .checked_mul(self.characters.len())
            .ok_or_else(|| MatrixOcrError::Model("Paddle output is too large".into()))?;
        if data.len() / stride != batch || !data.len().is_multiple_of(stride) {
            return Err(MatrixOcrError::Model(
                "Unexpected Paddle output length".into(),
            ));
        }
        data.chunks_exact(stride)
            .map(|sample| recognition::decode(sample, &self.characters))
            .collect()
    }
}
fn model_error(error: impl std::fmt::Display) -> MatrixOcrError {
    MatrixOcrError::Model(error.to_string())
}

/// The mutex serializes scans; the session stays resident, but no photo/cell data does.
#[derive(Clone, Default)]
pub struct MatrixOcrService(
    Arc<Mutex<Option<Paddle>>>,
    #[cfg(debug_assertions)] Option<std::path::PathBuf>,
);
impl MatrixOcrService {
    /// Prepare and validate the optional runtime before accepting a camera scan.
    pub fn warm_up(&self) -> Result<(), MatrixOcrError> {
        let mut model = self.lock_model()?;
        if model.is_none() {
            *model = Some(Paddle::new()?);
        }
        let Some(paddle) = model.as_mut() else {
            return Err(MatrixOcrError::Model(
                "OCR runtime did not initialize".into(),
            ));
        };
        paddle.recognize(vec![0.; 3 * 48 * 320])?;
        Ok(())
    }
    fn lock_model(&self) -> Result<std::sync::MutexGuard<'_, Option<Paddle>>, MatrixOcrError> {
        let model = match self.0.try_lock() {
            Ok(model) => model,
            Err(TryLockError::WouldBlock) => return Err(MatrixOcrError::Busy),
            Err(TryLockError::Poisoned(poisoned)) => {
                // A caller callback may have unwound while this lock was held.
                // Discard the resident session and make the next call retry cleanly.
                let mut model = poisoned.into_inner();
                *model = None;
                self.0.clear_poison();
                return Err(MatrixOcrError::Model(
                    "Matrix scanner recovered after an earlier scan stopped unexpectedly. Try again."
                        .into(),
                ));
            }
        };
        Ok(model)
    }

    #[cfg(debug_assertions)]
    pub fn with_debug_dir(mut self, directory: Option<std::path::PathBuf>) -> Self {
        self.1 = directory;
        self
    }
    /// A fresh frame is decoded/located once; only scheduled cells are prepared.
    /// Batches of ten use a 64 px canvas for a single Latin character.
    pub fn scan_live_frame(
        &self,
        bytes: &[u8],
        indices: &[usize],
        geometry: impl Fn(&[[[f64; 2]; 4]]) -> bool,
        progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<crate::MatrixLiveFrame, MatrixOcrError> {
        use std::time::Instant;
        let start = Instant::now();
        let mut seen = [false; MATRIX_CELLS];
        if indices.is_empty()
            || indices
                .iter()
                .any(|&i| i >= MATRIX_CELLS || std::mem::replace(&mut seen[i], true))
        {
            return Err(MatrixOcrError::InvalidImage(
                "Invalid or duplicated Matrix cell indices".into(),
            ));
        }
        let mut model = self.lock_model()?;
        if !progress(OcrProgress {
            completed: 0,
            total: indices.len(),
        }) {
            return Err(MatrixOcrError::Cancelled);
        }
        let (image, small) = prepare_images(bytes)?;
        let mut timing = crate::MatrixFrameTiming {
            decode_ms: start.elapsed().as_secs_f64() * 1000.,
            ..Default::default()
        };
        let stage = Instant::now();
        let cells = geometry::locate(&small).ok_or(MatrixOcrError::GridNotFound)?;
        let quads = normalized_quads(&cells, &small);
        timing.geometry_ms = stage.elapsed().as_secs_f64() * 1000.;
        if !geometry(&quads) {
            return Err(MatrixOcrError::Cancelled);
        }
        let mut results = Vec::with_capacity(indices.len());
        for chunk in indices.chunks(10) {
            let stage = Instant::now();
            let binaries: Vec<_> = chunk
                .iter()
                .map(|&i| {
                    let corners = cells[i].map(|p| geometry::Point {
                        x: p.x * image.width() as f64 / small.width() as f64,
                        y: p.y * image.height() as f64 / small.height() as f64,
                    });
                    preprocess::binarize(&geometry::crop(&image, &corners))
                })
                .collect();
            timing.preprocess_ms += stage.elapsed().as_secs_f64() * 1000.;
            let stage = Instant::now();
            if binaries.iter().any(preprocess::Binary::has_glyph) && model.is_none() {
                *model = Some(Paddle::new()?);
            }
            timing.inference_ms += stage.elapsed().as_secs_f64() * 1000.;
            let mut accepted = vec![Recognition::default(); chunk.len()];
            if let Some(paddle) = model.as_mut() {
                let (cells, preprocessing, inference) = paddle.recognize_cells(&binaries)?;
                accepted = cells;
                timing.preprocess_ms += preprocessing;
                timing.inference_ms += inference;
            }
            // Thin strokes can disappear in a 64 px crop even with a sharp
            // source. Rescue only weak cells at 128 px instead of slowing all 70.
            let stage = Instant::now();
            let rescue_positions: Vec<_> = (0..chunk.len())
                .filter(|&i| accepted[i].confidence < 0.8 && binaries[i].has_glyph())
                .collect();
            let rescue: Vec<_> = rescue_positions
                .iter()
                .map(|&i| {
                    let corners = cells[chunk[i]].map(|p| geometry::Point {
                        x: p.x * image.width() as f64 / small.width() as f64,
                        y: p.y * image.height() as f64 / small.height() as f64,
                    });
                    preprocess::binarize(&geometry::crop_at(&image, &corners, 128))
                })
                .collect();
            timing.preprocess_ms += stage.elapsed().as_secs_f64() * 1000.;
            if let Some(paddle) = model.as_mut() {
                let (alternatives, preprocessing, inference) = paddle.recognize_cells(&rescue)?;
                timing.preprocess_ms += preprocessing;
                timing.inference_ms += inference;
                for (i, alternative) in rescue_positions.into_iter().zip(alternatives) {
                    if alternative.confidence > accepted[i].confidence {
                        accepted[i] = alternative;
                    }
                }
            }
            results.extend(accepted);
            if !progress(OcrProgress {
                completed: results.len(),
                total: indices.len(),
            }) {
                return Err(MatrixOcrError::Cancelled);
            }
        }
        timing.total_ms = start.elapsed().as_secs_f64() * 1000.;
        Ok(crate::MatrixLiveFrame {
            cells: results,
            quads,
            timing,
        })
    }

    pub fn scan(
        &self,
        image: &[u8],
        progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<Vec<Recognition>, MatrixOcrError> {
        self.scan_frame(image, progress).map(|frame| frame.cells)
    }

    pub fn scan_frame(
        &self,
        image: &[u8],
        progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<crate::MatrixFrame, MatrixOcrError> {
        self.scan_frame_with_geometry(image, |_| true, progress)
    }

    /// Publish geometry before the 70 model inferences so live overlays do not wait.
    pub fn scan_frame_with_geometry(
        &self,
        image: &[u8],
        geometry: impl Fn(&[[[f64; 2]; 4]]) -> bool,
        progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<crate::MatrixFrame, MatrixOcrError> {
        // Reject overlapping scans instead of accumulating photos and blocking jobs.
        let mut model = self.lock_model()?;
        if !progress(OcrProgress {
            completed: 0,
            total: MATRIX_CELLS,
        }) {
            return Err(MatrixOcrError::Cancelled);
        }
        let (image, small) = prepare_images(image)?;
        let cells = geometry::locate(&small).ok_or(MatrixOcrError::GridNotFound)?;
        let quads = normalized_quads(&cells, &small);
        if !geometry(&quads) {
            return Err(MatrixOcrError::Cancelled);
        }
        let mut results = Vec::with_capacity(MATRIX_CELLS);
        #[cfg(debug_assertions)]
        let mut red = vec![];
        for (i, corners) in cells.iter().enumerate() {
            let corners = corners.map(|p| geometry::Point {
                x: p.x * image.width() as f64 / small.width() as f64,
                y: p.y * image.height() as f64 / small.height() as f64,
            });
            let binary = preprocess::binarize(&geometry::crop(&image, &corners));
            let prepared = preprocess::prepare(&binary.cleaned);
            let result = if !binary.has_glyph() {
                Recognition::default()
            } else {
                if model.is_none() {
                    *model = Some(Paddle::new()?);
                }
                let Some(paddle) = model.as_mut() else {
                    return Err(MatrixOcrError::Model(
                        "Matrix OCR model did not initialize".into(),
                    ));
                };
                let raw = paddle.recognize(prepared.tensor)?;
                let mut accepted = recognition::accept(&raw.text, raw.confidence);
                if !accepted.text.is_empty() && binary.cleaned != binary.original {
                    let reference =
                        paddle.recognize(preprocess::prepare(&binary.original).tensor)?;
                    accepted = recognition::review(accepted, &reference.text);
                }
                accepted
            };
            #[cfg(debug_assertions)]
            if self.1.is_some() && recognition::is_red(&result) {
                let mut bytes = Cursor::new(Vec::new());
                match prepared
                    .preview
                    .write_to(&mut bytes, image::ImageFormat::Png)
                {
                    Ok(()) => red.push(debug_crops::Crop::from_png(
                        format!("{}{}.png", (b'A' + (i % 10) as u8) as char, i / 10 + 1),
                        bytes.into_inner(),
                    )),
                    Err(error) => eprintln!("Matrix debug crop encoding failed: {error}"),
                }
            }
            results.push(result);
            if !progress(OcrProgress {
                completed: i + 1,
                total: MATRIX_CELLS,
            }) {
                return Err(MatrixOcrError::Cancelled);
            }
        }
        #[cfg(debug_assertions)]
        if let Some(directory) = self.1.as_ref().filter(|_| !red.is_empty()) {
            // Diagnostic exports must never turn a completed scan into an error.
            if let Err(error) = debug_crops::save(directory, red) {
                eprintln!("Matrix debug export failed: {error}");
            }
        }
        Ok(crate::MatrixFrame {
            cells: results,
            quads,
        })
    }
}

fn normalized_quads(cells: &[[geometry::Point; 4]], image: &RgbImage) -> Vec<[[f64; 2]; 4]> {
    cells
        .iter()
        .map(|corners| {
            corners.map(|p| {
                [
                    (p.x / image.width() as f64).clamp(0., 1.),
                    (p.y / image.height() as f64).clamp(0., 1.),
                ]
            })
        })
        .collect()
}

fn prepare_images(bytes: &[u8]) -> Result<(RgbImage, RgbImage), MatrixOcrError> {
    let image = resize_longest(decode_image(bytes)?, 2200);
    let preview = resize_longest(image.clone(), 600);
    Ok((image, preview))
}

fn resize_longest(image: RgbImage, max: u32) -> RgbImage {
    let longest = image.width().max(image.height());
    if longest <= max {
        return image;
    }
    let scale = max as f64 / longest as f64;
    let width = (image.width() as f64 * scale).round().max(1.) as u32;
    let height = (image.height() as f64 * scale).round().max(1.) as u32;
    image::imageops::resize(&image, width, height, FilterType::Triangle)
}

fn decode_image(bytes: &[u8]) -> Result<RgbImage, MatrixOcrError> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(crate::image_too_large());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| MatrixOcrError::InvalidImage(error.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|error| {
        MatrixOcrError::InvalidImage(format!(
            "Cannot read image (use PNG, JPEG or WebP): {error}"
        ))
    })?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || w as u64 * h as u64 > 40_000_000 {
        return Err(MatrixOcrError::InvalidImage(
            "Image dimensions exceed the 40-megapixel limit.".into(),
        ));
    }
    let orientation = decoder
        .orientation()
        .map_err(|error| MatrixOcrError::InvalidImage(error.to_string()))?;
    let mut decoded = DynamicImage::from_decoder(decoder)
        .map_err(|error| MatrixOcrError::InvalidImage(error.to_string()))?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.to_rgba8();
    let rgb = RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let a = p[3] as f64 / 255.;
        Rgb([0, 1, 2].map(|c| (p[c] as f64 * a + 255. * (1. - a)).round() as u8))
    });
    Ok(rgb)
}
