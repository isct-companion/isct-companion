use super::{geometry, illumination, model_error, normalized_quads, prepare_images};
use crate::{MatrixCaptureFrame, MatrixCaptureGlyph, MatrixOcrError, Recognition};
use image::{imageops::FilterType, GrayImage, Rgb, RgbImage};
use imageproc::geometric_transformations::{warp_into, Border, Interpolation, Projection};
use ort::{session::Session, value::Tensor};
use std::sync::{Arc, Mutex, TryLockError};

const SIZE: usize = 96;
const MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/matrix-alphabet.onnx"));

/// Pixel-center convention and grayscale coefficients match PIL/TorchVision.
pub(super) fn rectified_gray(
    image: &RgbImage,
    quad: [[f64; 2]; 4],
) -> Result<GrayImage, MatrixOcrError> {
    let corners = quad.map(|[x, y]| geometry::Point {
        x: x * image.width() as f64,
        y: y * image.height() as f64,
    });
    let destination = [(0f32, 0f32), (95., 0.), (95., 95.), (0., 95.)];
    let source = destination.map(|(x, y)| {
        let p = geometry::project(
            &corners,
            0.09 + (x as f64 + 0.5) / 95. * 0.82,
            0.09 + (y as f64 + 0.5) / 95. * 0.82,
        );
        ((p.x - 0.5) as f32, (p.y - 0.5) as f32)
    });
    let projection =
        Projection::from_control_points(source, destination).ok_or(MatrixOcrError::GridNotFound)?;
    let mut crop = RgbImage::new(SIZE as u32, SIZE as u32);
    warp_into(
        image,
        projection,
        Interpolation::Bilinear,
        Border::Constant(Rgb([255; 3])),
        &mut crop,
    );
    // image::grayscale uses Rec.709, while training uses Rec.601.
    Ok(GrayImage::from_fn(SIZE as u32, SIZE as u32, |x, y| {
        let p = crop.get_pixel(x, y);
        image::Luma([
            ((19595 * p[0] as u32 + 38470 * p[1] as u32 + 7471 * p[2] as u32 + 32768) >> 16) as u8,
        ])
    }))
}
pub fn capture_matrix_frame(bytes: &[u8]) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    let start = std::time::Instant::now();
    let (image, small) = prepare_images(bytes)?;
    let decode_ms = start.elapsed().as_secs_f64() * 1000.;
    let stage = std::time::Instant::now();
    let cells = geometry::locate_capture(&small).ok_or(MatrixOcrError::GridNotFound)?;
    let quads = normalized_quads(&cells, &small);
    let geometry_ms = stage.elapsed().as_secs_f64() * 1000.;
    let stage = std::time::Instant::now();
    let glyphs = quads
        .iter()
        .map(|&quad| {
            let gray = rectified_gray(&image, quad)?;
            let light = illumination::analyze(&gray);
            let measured = image::imageops::resize(
                light.corrected.as_ref().unwrap_or(&gray),
                32,
                32,
                FilterType::Triangle,
            );
            let mean = measured.pixels().map(|p| p[0] as f64).sum::<f64>() / 1024.;
            let contrast = (measured
                .pixels()
                .map(|p| (p[0] as f64 - mean).powi(2))
                .sum::<f64>()
                / 1024.)
                .sqrt();
            let deviation = |points: Vec<f64>| {
                let mean = points.iter().sum::<f64>() / points.len() as f64;
                (points.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / points.len() as f64)
                    .sqrt()
            };
            let interior = deviation(
                (4..28)
                    .flat_map(|y| (4..28).map(move |x| (x, y)))
                    .map(|(x, y)| measured.get_pixel(x, y)[0] as f64)
                    .collect(),
            );
            let border = deviation(
                (0..32)
                    .flat_map(|y| (0..32).map(move |x| (x, y)))
                    .filter(|&(x, y)| !(4..28).contains(&x) || !(4..28).contains(&y))
                    .map(|(x, y)| measured.get_pixel(x, y)[0] as f64)
                    .collect(),
            );
            let centering = interior / (interior + border).max(1.);
            let laplacian = imageproc::filter::laplacian_filter(&measured);
            let sharpness = (1..31)
                .flat_map(|y| (1..31).map(move |x| (x, y)))
                .map(|(x, y)| (laplacian.get_pixel(x, y)[0] as f64).powi(2))
                .sum::<f64>()
                / 900.;
            let distance = |a: [f64; 2], b: [f64; 2]| {
                ((a[0] - b[0]) * image.width() as f64).hypot((a[1] - b[1]) * image.height() as f64)
            };
            let source_size = (0..4)
                .map(|i| distance(quad[i], quad[(i + 1) % 4]))
                .fold(f64::INFINITY, f64::min)
                * 0.82;
            Ok(MatrixCaptureGlyph {
                gray: gray.into_raw(),
                contrast,
                sharpness,
                source_size,
                centering,
                glare: light.glare,
            })
        })
        .collect::<Result<Vec<_>, MatrixOcrError>>()?;
    Ok(MatrixCaptureFrame {
        quads,
        glyphs,
        timing: crate::MatrixFrameTiming {
            decode_ms,
            geometry_ms,
            preprocess_ms: stage.elapsed().as_secs_f64() * 1000.,
            total_ms: start.elapsed().as_secs_f64() * 1000.,
            ..Default::default()
        },
    })
}

#[derive(Clone, Default)]
pub struct MatrixAlphabetService(Arc<Mutex<Option<Session>>>);
impl MatrixAlphabetService {
    pub fn recognize(&self, images: &[Vec<u8>]) -> Result<Vec<Recognition>, MatrixOcrError> {
        let mut raw = self.recognize_raw(images)?;
        let corrections = images
            .iter()
            .enumerate()
            .filter_map(|(i, pixels)| {
                let gray = GrayImage::from_raw(SIZE as u32, SIZE as u32, pixels.clone())?;
                illumination::analyze(&gray)
                    .corrected
                    .map(|image| (i, image.into_raw()))
            })
            .collect::<Vec<_>>();
        let (positions, images): (Vec<_>, Vec<_>) = corrections.into_iter().unzip();
        for (i, corrected) in positions.into_iter().zip(self.recognize_raw(&images)?) {
            // Two renderings of one observation remain ONE vote. A confident
            // disagreement is review evidence, never a second independent vote.
            if raw[i].confidence >= 0.8
                && corrected.confidence >= 0.8
                && raw[i].text != corrected.text
            {
                raw[i].confidence = 0.79;
            } else if corrected.confidence > raw[i].confidence {
                raw[i] = corrected;
            }
        }
        Ok(raw)
    }
    fn recognize_raw(&self, images: &[Vec<u8>]) -> Result<Vec<Recognition>, MatrixOcrError> {
        if images.is_empty() {
            return Ok(vec![]);
        }
        if images.len() > 10 || images.iter().any(|image| image.len() != SIZE * SIZE) {
            return Err(MatrixOcrError::InvalidImage(
                "Invalid grayscale candidate batch.".into(),
            ));
        }
        let mut resident = match self.0.try_lock() {
            Ok(resident) => resident,
            Err(TryLockError::WouldBlock) => return Err(MatrixOcrError::Busy),
            Err(TryLockError::Poisoned(poisoned)) => {
                *poisoned.into_inner() = None;
                self.0.clear_poison();
                return Err(MatrixOcrError::Model(
                    "Alphabet scanner recovered after an interrupted scan. Try again.".into(),
                ));
            }
        };
        if resident.is_none() {
            *resident = Some(
                Session::builder()
                    .map_err(model_error)?
                    .with_intra_threads(2)
                    .map_err(model_error)?
                    .with_inter_threads(1)
                    .map_err(model_error)?
                    .commit_from_memory(MODEL)
                    .map_err(model_error)?,
            );
        }
        let session = resident
            .as_mut()
            .ok_or_else(|| MatrixOcrError::Model("Alphabet model did not initialize.".into()))?;
        let input = Tensor::from_array((
            [images.len(), 1, SIZE, SIZE],
            images
                .iter()
                .flatten()
                .map(|p| *p as f32 / 127.5 - 1.)
                .collect::<Vec<_>>(),
        ))
        .map_err(model_error)?;
        let output = session.run(ort::inputs![input]).map_err(model_error)?;
        let Some((_, output)) = output.into_iter().next() else {
            return Err(MatrixOcrError::Model("Missing alphabet output.".into()));
        };
        let (shape, data) = output.try_extract_tensor::<f32>().map_err(model_error)?;
        if shape.as_ref() != [images.len() as i64, 27]
            || data.len() != images.len() * 27
            || data.iter().any(|v| !v.is_finite())
        {
            return Err(MatrixOcrError::Model("Invalid alphabet output.".into()));
        }
        data.as_chunks::<27>()
            .0
            .iter()
            .map(|row| {
                let (winner, &max) = row
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .ok_or_else(|| MatrixOcrError::Model("Empty alphabet output.".into()))?;
                let sum = row.iter().map(|v| (*v - max).exp()).sum::<f32>();
                Ok(Recognition {
                    text: if winner == 26 {
                        String::new()
                    } else {
                        ((b'A' + winner as u8) as char).to_string()
                    },
                    confidence: 1. / sum as f64,
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "../tests/engine__alphabet.rs"]
mod tests;
