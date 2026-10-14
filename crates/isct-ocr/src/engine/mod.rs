mod contours;
#[cfg(debug_assertions)]
mod debug_crops;
mod geometry;
mod preprocess;
#[cfg(test)]
mod private_parity;
#[cfg(test)]
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
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

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
        let input = Tensor::from_array(([1usize, 3, 48, 320], tensor)).map_err(model_error)?;
        let output = self.session.run(ort::inputs![input]).map_err(model_error)?;
        let Some((_, output)) = output.into_iter().next() else {
            return Err(MatrixOcrError::Model(
                "Unexpected Paddle output shape".into(),
            ));
        };
        let (shape, data) = output.try_extract_tensor::<f32>().map_err(model_error)?;
        if shape.len() != 3 || shape[0] != 1 || shape[2] != self.characters.len() as i64 {
            return Err(MatrixOcrError::Model(
                "Unexpected Paddle output shape".into(),
            ));
        }
        recognition::decode(data, &self.characters)
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
    #[cfg(debug_assertions)]
    pub fn with_debug_dir(mut self, directory: Option<std::path::PathBuf>) -> Self {
        self.1 = directory;
        self
    }
    pub fn scan(
        &self,
        image: &[u8],
        progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<Vec<Recognition>, MatrixOcrError> {
        // Reject overlapping scans instead of accumulating photos and blocking jobs.
        let mut model = match self.0.try_lock() {
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
        if !progress(OcrProgress {
            completed: 0,
            total: MATRIX_CELLS,
        }) {
            return Err(MatrixOcrError::Cancelled);
        }
        let (image, small) = prepare_images(image)?;
        let cells = geometry::locate(&small).ok_or(MatrixOcrError::GridNotFound)?;
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
            let result = if binary.ink < 12 || binary.ink as f64 > 64. * 64. * 0.55 {
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
        Ok(results)
    }
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
        return Err(MatrixOcrError::InvalidImage(
            "Choose a valid image smaller than 20 MB.".into(),
        ));
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
