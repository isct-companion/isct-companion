mod geometry;
mod preprocess;
mod recognition;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod private_parity;

use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
pub use recognition::Recognition;
use std::{
    io::Cursor,
    sync::{Arc, Mutex},
};

const MODEL: &[u8] = include_bytes!("../../models/paddle/en-ppocrv5.onnx");
const DICTIONARY: &str = include_str!("../../models/paddle/characters.json");
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

struct Paddle {
    session: Session,
    characters: Vec<String>,
}
impl Paddle {
    fn new() -> Result<Self, String> {
        let session = Session::builder()
            .map_err(err)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(err)?
            .with_intra_threads(2)
            .map_err(err)?
            .with_inter_threads(1)
            .map_err(err)?
            .commit_from_memory(MODEL)
            .map_err(err)?;
        Ok(Self {
            session,
            characters: serde_json::from_str(DICTIONARY).map_err(err)?,
        })
    }
    fn recognize(&mut self, tensor: Vec<f32>) -> Result<Recognition, String> {
        let input = Tensor::from_array(([1usize, 3, 48, 320], tensor)).map_err(err)?;
        let output = self.session.run(ort::inputs![input]).map_err(err)?;
        let (shape, data) = output[0].try_extract_tensor::<f32>().map_err(err)?;
        if shape.len() != 3 || shape[0] != 1 || shape[2] != self.characters.len() as i64 {
            return Err("Unexpected Paddle output shape".into());
        }
        recognition::decode(data, &self.characters)
    }
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// The mutex serializes scans; the session stays resident, but no photo/cell data does.
#[derive(Clone, Default)]
pub struct MatrixOcrService(Arc<Mutex<Option<Paddle>>>);
impl MatrixOcrService {
    pub fn scan(
        &self,
        data: &str,
        preview: &str,
        progress: impl Fn(usize),
    ) -> Result<Vec<Recognition>, String> {
        // Reject overlapping scans instead of accumulating photos and blocking jobs.
        let mut model = self
            .0
            .try_lock()
            .map_err(|_| "Matrix scanner is busy. Try again after the current scan.")?;
        let image = decode_canvas(data, 2200)?;
        let small = decode_canvas(preview, 600)?;
        let scale = (600. / image.width().max(image.height()) as f64).min(1.);
        let expected = (
            (image.width() as f64 * scale).round().max(1.) as u32,
            (image.height() as f64 * scale).round().max(1.) as u32,
        );
        if small.dimensions() != expected {
            return Err("Matrix preview dimensions do not match the source.".into());
        }
        let cells=geometry::locate(&small).ok_or("Could not locate all 7 × 10 cells. Include the A–J header and 1–7 row labels in a focused photo.")?;
        let mut results = Vec::with_capacity(70);
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
                let paddle = model.as_mut().unwrap();
                let raw = paddle.recognize(prepared.tensor)?;
                let mut accepted = recognition::accept(&raw.text, raw.confidence);
                if !accepted.text.is_empty() && binary.cleaned != binary.original {
                    let reference =
                        paddle.recognize(preprocess::prepare(&binary.original).tensor)?;
                    accepted = recognition::review(accepted, &reference.text);
                }
                accepted
            };
            if recognition::is_red(&result) {
                let mut bytes = Cursor::new(Vec::new());
                prepared
                    .preview
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .map_err(err)?;
                red.push(crate::matrix_red_crops::Crop::from_png(
                    format!("{}{}.png", (b'A' + (i % 10) as u8) as char, i / 10 + 1),
                    bytes.into_inner(),
                ));
            }
            results.push(result);
            progress(i + 1);
        }
        if !red.is_empty() {
            crate::matrix_red_crops::save(red)?;
        }
        Ok(results)
    }
}
// Decode only: using image::resize here changes Canvas pixels and OCR geometry.
fn decode_canvas(data: &str, max: u32) -> Result<RgbImage, String> {
    if data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err("Prepared image exceeds the size limit.".into());
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| "Invalid image encoding")?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Expected a prepared PNG image.".into());
    }
    let image = decode_image(&bytes)?;
    if image.width().max(image.height()) > max {
        return Err("Prepared image dimensions exceed the size limit.".into());
    }
    Ok(image)
}
fn decode_image(bytes: &[u8]) -> Result<RgbImage, String> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err("Choose a valid image smaller than 20 MB.".into());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(err)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12000);
    limits.max_image_height = Some(12000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("Cannot read image (use PNG, JPEG or WebP): {e}"))?;
    let (w, h) = decoder.dimensions();
    if w == 0 || h == 0 || w as u64 * h as u64 > 40_000_000 {
        return Err("Image dimensions exceed the 40-megapixel limit.".into());
    }
    let orientation = decoder.orientation().map_err(err)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(err)?;
    decoded.apply_orientation(orientation);
    let rgba = decoded.to_rgba8();
    let rgb = RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y);
        let a = p[3] as f64 / 255.;
        Rgb([0, 1, 2].map(|c| (p[c] as f64 * a + 255. * (1. - a)).round() as u8))
    });
    Ok(rgb)
}
