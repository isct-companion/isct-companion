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

#[cfg(matrix_ocr)]
mod engine;

pub use recognition::Recognition;
use serde::Serialize;
#[cfg(not(matrix_ocr))]
use std::path::PathBuf;
use thiserror::Error;

pub const MATRIX_CELLS: usize = 70;

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
pub use engine::MatrixOcrService;

#[cfg(not(matrix_ocr))]
#[derive(Clone, Default)]
pub struct MatrixOcrService;

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
