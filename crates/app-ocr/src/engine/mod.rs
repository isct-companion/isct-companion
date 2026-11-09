mod alphabet;
pub use alphabet::{capture_matrix_frame, capture_matrix_keyframe, MatrixAlphabetService};
#[cfg(feature = "training-tools")]
pub use alphabet::{prepare_matrix_alphabet_input, MatrixAlphabetInput};
mod geometry;
mod grid;
mod illumination;
mod letters;
mod picture;
pub use picture::{matrix_analysis_from_yuv, matrix_full_luma_from_yuv, matrix_picture};
mod vendor;

#[cfg(test)]
#[path = "../tests/engine.rs"]
mod tests;

use crate::MatrixOcrError;
use image::{DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};
use std::io::Cursor;

use crate::MAX_IMAGE_BYTES;

/// One still image, detected from scratch: the 70 cell quads, normalized.
fn locate(image: &RgbImage) -> Option<Vec<[[f64; 2]; 4]>> {
    let planes = grid::Planes::from_rgb(image)?;
    let (_, h, _) = grid::detect(&planes)?;
    let body = grid::normalize(grid::body(&h), &planes);
    grid::cells(body, 1., 1.)
}

/// The analysis planes of an encoded image (replay, photos).
pub fn matrix_analysis_image(bytes: &[u8]) -> Result<crate::MatrixAnalysisImage, MatrixOcrError> {
    let planes = grid::Planes::from_rgb(&decode_image(bytes)?)
        .ok_or_else(|| MatrixOcrError::InvalidImage("Image is too small.".into()))?;
    Ok(crate::MatrixAnalysisImage {
        width: planes.luma.width(),
        height: planes.luma.height(),
        chroma: planes.chroma.clone(),
        luma: planes.luma.into_raw(),
    })
}

/// Camera geometry state for one session: detect, fit and track.
#[derive(Default)]
pub struct MatrixTracker(grid::Tracker);
impl MatrixTracker {
    /// Analyses the next frame of the session. Frames must arrive in capture
    /// order; a size change starts over.
    pub fn track(
        &mut self,
        frame: &crate::MatrixAnalysisFrame<'_>,
    ) -> Result<crate::MatrixTrack, MatrixOcrError> {
        let start = std::time::Instant::now();
        let luma = image::GrayImage::from_raw(frame.width, frame.height, frame.luma.to_vec());
        let planes = luma
            .filter(|_| frame.chroma.len() == frame.luma.len() && frame.time.is_finite())
            .and_then(|luma| grid::Planes::new(luma, frame.chroma.to_vec()))
            .ok_or_else(|| MatrixOcrError::InvalidImage("Invalid analysis planes.".into()))?;
        let step = self.0.step(&planes, frame.time);
        let normalize = |q: [[f64; 2]; 4]| grid::normalize(q, &planes);
        Ok(crate::MatrixTrack {
            mode: match step.mode {
                grid::TrackMode::Lost => crate::MatrixTrackMode::Lost,
                grid::TrackMode::Detected => crate::MatrixTrackMode::Detected,
                grid::TrackMode::Refitted => crate::MatrixTrackMode::Refitted,
                grid::TrackMode::Tracked => crate::MatrixTrackMode::Tracked,
            },
            body: step.grid.map(|g| normalize(grid::body(&g))),
            display: step.display.map(normalize),
            keyframe: step.keyframe,
            residual: step.fit.as_ref().map(|f| f.residual),
            reason: step.fit.as_ref().and_then(|f| f.reason),
            stages: step
                .stages
                .iter()
                .map(|(name, g)| (*name, normalize(grid::body(g))))
                .collect(),
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
        })
    }
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
