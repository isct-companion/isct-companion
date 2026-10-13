//! Optional matrix-card OCR with no UI or platform-framework dependencies.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

mod recognition;

#[cfg(matrix_ocr)]
mod engine;

pub use recognition::Recognition;
#[cfg(not(matrix_ocr))]
use std::path::PathBuf;
use thiserror::Error;

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
    #[cfg(debug_assertions)]
    pub fn with_debug_dir(self, _directory: Option<PathBuf>) -> Self {
        self
    }

    pub fn scan(
        &self,
        _data: &str,
        _preview: &str,
        _progress: impl Fn(usize),
    ) -> Result<Vec<Recognition>, MatrixOcrError> {
        Err(MatrixOcrError::Unavailable)
    }
}

#[cfg(all(test, not(matrix_ocr)))]
mod tests {
    use super::*;

    #[test]
    fn disabled_backend_has_the_same_recoverable_interface() {
        let service = MatrixOcrService;
        for _ in 0..2 {
            assert_eq!(
                service.scan("", "", |_| {}),
                Err(MatrixOcrError::Unavailable)
            );
        }
    }
}
