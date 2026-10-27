//! Optional matrix-card OCR with no UI or platform-framework dependencies.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]

mod recognition;
mod vision;
pub use vision::matrix_vision;

#[cfg(matrix_ocr)]
mod engine;

pub use recognition::Recognition;
use serde::Serialize;
#[cfg(not(matrix_ocr))]
use std::path::PathBuf;
use thiserror::Error;

pub const MATRIX_CELLS: usize = 70;

/// Projection of the original RGB cell into the alphabet model's one channel.
#[derive(Clone, Copy, Debug, Default)]
pub enum MatrixGlyphProjection {
    #[default]
    Luma,
    /// Diagnostic red-opponent projection; neutral paper loses ink contrast.
    RedChroma,
    /// RGB distance below the cell's upper-quartile paper color.
    PaperDistance,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MatrixCaptureConfig {
    pub glyph_projection: MatrixGlyphProjection,
}

/// One camera frame for the geometry loop: two row-major planes of one byte
/// per pixel, Rec.601 luma and `clamp(R - G + 128, 0, 255)`. The R-G plane
/// carries the pink/white row boundaries, which luma alone does not place
/// reliably. `time` is the capture time in seconds, increasing.
pub struct MatrixAnalysisFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub luma: &'a [u8],
    pub chroma: &'a [u8],
    pub time: f64,
}
/// Owned analysis planes, e.g. of a decoded photo or recorded frame.
pub struct MatrixAnalysisImage {
    pub width: u32,
    pub height: u32,
    pub luma: Vec<u8>,
    pub chroma: Vec<u8>,
}
impl MatrixAnalysisImage {
    pub fn frame(&self, time: f64) -> MatrixAnalysisFrame<'_> {
        MatrixAnalysisFrame {
            width: self.width,
            height: self.height,
            luma: &self.luma,
            chroma: &self.chroma,
            time,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatrixTrackMode {
    /// No grid on this frame.
    Lost,
    /// Found from scratch and fitted.
    Detected,
    /// Refitted to the printed lines from the tracked prediction.
    Refitted,
    /// Followed from the previous frame by optical flow.
    Tracked,
}
/// The geometry of one analysed frame, in that frame's normalized coordinates.
pub struct MatrixTrack {
    pub mode: MatrixTrackMode,
    /// Body corners (A1 top-left, clockwise) as measured, for cropping.
    pub body: Option<[[f64; 2]; 4]>,
    /// Smoothed body corners, for drawing only.
    pub display: Option<[[f64; 2]; 4]>,
    /// The grid was fitted on this frame with a low residual: cells may be
    /// cut from the full-resolution version of exactly this frame.
    pub keyframe: bool,
    /// Reprojection residual of this frame's fit, in cells.
    pub residual: Option<f64>,
    /// Gate that rejected this frame's fit, if one ran and failed.
    pub reason: Option<&'static str>,
    /// Body corners of each stage that ran (seed, fit, track), for replay.
    pub stages: Vec<(&'static str, [[f64; 2]; 4])>,
    pub elapsed_ms: f64,
}

/// One frame's readings and normalized cell corners (top-left clockwise).
pub struct MatrixFrame {
    pub cells: Vec<Recognition>,
    pub quads: Vec<[[f64; 2]; 4]>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MatrixFrameTiming {
    pub decode_ms: f64,
    pub geometry_ms: f64,
    pub preprocess_ms: f64,
    pub inference_ms: f64,
    pub total_ms: f64,
}
pub struct MatrixLiveFrame {
    /// Selected readings in the same order as the requested cell indices.
    pub cells: Vec<Recognition>,
    pub quads: Vec<[[f64; 2]; 4]>,
    pub timing: MatrixFrameTiming,
}
/// Capability measurements only; capture admission and lifetime belong to core.
pub struct MatrixCaptureGlyph {
    pub gray: Vec<u8>,
    pub contrast: f64,
    pub sharpness: f64,
    pub source_size: f64,
    pub centering: f64,
    pub glare: f64,
    /// Edge acuity: after a 5-95 % contrast stretch of the 48 px interior,
    /// the median over 4×4 blocks of each block's strongest gradient. Motion
    /// and defocus both widen the strokes' edges and lower it; exposure and
    /// print contrast do not.
    pub acuity: f64,
}
pub struct MatrixCaptureFrame {
    pub quads: Vec<[[f64; 2]; 4]>,
    pub glyphs: Vec<MatrixCaptureGlyph>,
    pub timing: MatrixFrameTiming,
}

/// Compile-time backend availability; model initialization remains lazy.
pub const fn available() -> bool {
    cfg!(matrix_ocr)
}

/// Largest encoded image `scan` accepts.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Reject an oversized image before a shell transport-decodes it: `len` is the
/// byte length of a base64 text payload carrying the encoded image.
pub fn check_base64_image_len(len: usize) -> Result<(), MatrixOcrError> {
    if len > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        Err(image_too_large())
    } else {
        Ok(())
    }
}

pub(crate) fn image_too_large() -> MatrixOcrError {
    MatrixOcrError::InvalidImage("Choose a valid image smaller than 20 MB.".into())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrProgress {
    pub completed: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MatrixOcrError {
    #[error("Matrix scanner is busy. Try again after the current scan.")]
    Busy,
    #[error("{0}")]
    InvalidImage(String),
    #[error(
        "Could not locate all 7 × 10 cells. Include the A–J header and 1–7 row labels in a focused photo."
    )]
    GridNotFound,
    #[error("{0}")]
    Model(String),
    #[error("Matrix scan cancelled.")]
    Cancelled,
    #[error("Matrix scanner is unavailable in this build.")]
    Unavailable,
}

#[cfg(matrix_ocr)]
pub use engine::{
    capture_matrix_frame, capture_matrix_frame_configured, capture_matrix_keyframe,
    matrix_analysis_image, matrix_picture, matrix_picture_from_yuv, MatrixAlphabetService,
    MatrixOcrService, MatrixTracker,
};
#[cfg(all(matrix_ocr, feature = "training-tools"))]
pub use engine::{
    capture_matrix_training_frame, prepare_matrix_alphabet_input, MatrixAlphabetInput,
};

#[cfg(not(matrix_ocr))]
pub fn capture_matrix_frame(_bytes: &[u8]) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    Err(MatrixOcrError::Unavailable)
}
#[cfg(not(matrix_ocr))]
pub fn capture_matrix_frame_configured(
    _bytes: &[u8],
    _config: MatrixCaptureConfig,
) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    Err(MatrixOcrError::Unavailable)
}
#[cfg(not(matrix_ocr))]
pub fn capture_matrix_keyframe(
    _luma: &[u8],
    _width: u32,
    _height: u32,
    _body: [[f64; 2]; 4],
) -> Result<MatrixCaptureFrame, MatrixOcrError> {
    Err(MatrixOcrError::Unavailable)
}
#[cfg(not(matrix_ocr))]
pub fn matrix_picture(
    _bytes: &[u8],
) -> Result<app_core::matrix_scan::MatrixPicture, MatrixOcrError> {
    Err(MatrixOcrError::Unavailable)
}
#[cfg(not(matrix_ocr))]
pub fn matrix_picture_from_yuv(
    _frame: &app_core::matrix_scan::MatrixYuvFrame<'_>,
) -> Result<app_core::matrix_scan::MatrixPicture, MatrixOcrError> {
    Err(MatrixOcrError::Unavailable)
}
#[cfg(not(matrix_ocr))]
pub fn matrix_analysis_image(_bytes: &[u8]) -> Result<MatrixAnalysisImage, MatrixOcrError> {
    Err(MatrixOcrError::Unavailable)
}
#[cfg(not(matrix_ocr))]
#[derive(Default)]
pub struct MatrixTracker(());
#[cfg(not(matrix_ocr))]
impl MatrixTracker {
    pub fn track(
        &mut self,
        _frame: &MatrixAnalysisFrame<'_>,
    ) -> Result<MatrixTrack, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }
}
// The stubs keep a private field like the ONNX types, so callers build both
// with `default()`; a unit struct would invite `Type` literals that only
// compile where `matrix_ocr` is off.
#[cfg(not(matrix_ocr))]
#[derive(Clone, Default)]
pub struct MatrixAlphabetService(());
#[cfg(not(matrix_ocr))]
impl MatrixAlphabetService {
    pub fn recognize(&self, _images: &[Vec<u8>]) -> Result<Vec<Recognition>, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }
}

#[cfg(not(matrix_ocr))]
#[derive(Clone, Default)]
pub struct MatrixOcrService(());

#[cfg(not(matrix_ocr))]
impl MatrixOcrService {
    pub fn warm_up(&self) -> Result<(), MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }

    #[cfg(debug_assertions)]
    pub fn with_debug_dir(self, _directory: Option<PathBuf>) -> Self {
        self
    }

    pub fn scan_live_frame(
        &self,
        _image: &[u8],
        _indices: &[usize],
        _geometry: impl Fn(&[[[f64; 2]; 4]]) -> bool,
        _progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<MatrixLiveFrame, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }

    pub fn scan_frame_with_geometry(
        &self,
        _image: &[u8],
        _geometry: impl Fn(&[[[f64; 2]; 4]]) -> bool,
        _progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<MatrixFrame, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }

    pub fn scan_frame(
        &self,
        _image: &[u8],
        _progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<MatrixFrame, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }

    pub fn scan(
        &self,
        _image: &[u8],
        _progress: impl Fn(OcrProgress) -> bool,
    ) -> Result<Vec<Recognition>, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }
}

#[cfg(test)]
#[path = "tests/crate__limit_tests.rs"]
mod limit_tests;

#[cfg(all(test, not(matrix_ocr)))]
#[path = "tests/crate.rs"]
mod tests;
