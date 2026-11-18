//! Core's image capability, implemented by this crate. Desktop and the
//! native SDK build their `MatrixCameraDriver` from `matrix_vision()`, so no
//! shell wires the tracker, the cut or the model itself.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use app_core::matrix_scan::{
    MatrixAnalysis, MatrixCaptureGlyph, MatrixCut, MatrixFullLuma, MatrixGridTrack,
    MatrixGridTracker, MatrixLuma, MatrixPicture, MatrixPlanes, MatrixPoint, MatrixReading,
    MatrixScanError, MatrixVision, MatrixYuvFrame,
};
use app_ocr::{MatrixAlphabetService, MatrixAnalysisFrame, MatrixOcrError, MatrixTracker};
use std::sync::Arc;

fn scan_error(error: MatrixOcrError) -> MatrixScanError {
    match error {
        MatrixOcrError::GridNotFound | MatrixOcrError::InvalidImage(_) => {
            MatrixScanError::Unreadable(error.to_string())
        }
        MatrixOcrError::Cancelled => MatrixScanError::Ended,
        MatrixOcrError::Unavailable => MatrixScanError::Unavailable,
        other => MatrixScanError::Backend(other.to_string()),
    }
}
fn point([x, y]: [f64; 2]) -> MatrixPoint {
    MatrixPoint { x, y }
}

// Borrow camera planes and move output buffers: the boundary copies no pixels.
fn yuv_frame<'a>(frame: &MatrixYuvFrame<'a>) -> app_ocr::MatrixYuvFrame<'a> {
    app_ocr::MatrixYuvFrame {
        width: frame.width,
        height: frame.height,
        y: frame.y,
        y_row_stride: frame.y_row_stride,
        u: frame.u,
        v: frame.v,
        uv_row_stride: frame.uv_row_stride,
        uv_pixel_stride: frame.uv_pixel_stride,
        matrix: match frame.matrix {
            app_core::matrix_scan::MatrixYuvMatrix::Bt601 => app_ocr::MatrixYuvMatrix::Bt601,
            app_core::matrix_scan::MatrixYuvMatrix::Bt709 => app_ocr::MatrixYuvMatrix::Bt709,
        },
        full_range: frame.full_range,
    }
}
fn analysis(image: app_ocr::MatrixAnalysisImage) -> MatrixAnalysis {
    MatrixAnalysis {
        width: image.width,
        height: image.height,
        luma: image.luma,
        chroma: image.chroma,
    }
}
fn full_luma(image: app_ocr::MatrixFullLuma) -> MatrixFullLuma {
    MatrixFullLuma {
        width: image.width,
        height: image.height,
        luma: image.luma,
    }
}

struct Tracker(MatrixTracker);
impl MatrixGridTracker for Tracker {
    fn track(
        &mut self,
        planes: MatrixPlanes<'_>,
        time_s: f64,
    ) -> Result<MatrixGridTrack, MatrixScanError> {
        let track = self
            .0
            .track(&MatrixAnalysisFrame {
                width: planes.width,
                height: planes.height,
                luma: planes.luma,
                chroma: planes.chroma,
                time: time_s,
            })
            .map_err(scan_error)?;
        Ok(MatrixGridTrack {
            display: track.display.map(|q| q.map(point)),
            body: track.body.map(|q| q.map(point)),
            keyframe: track.keyframe,
            elapsed_ms: track.elapsed_ms,
        })
    }
}

#[derive(Default)]
struct Vision {
    alphabet: MatrixAlphabetService,
}
impl MatrixVision for Vision {
    fn available(&self) -> bool {
        app_ocr::available()
    }
    fn warm_up(&self) -> Result<(), MatrixScanError> {
        // A blank cell still runs the model once, so the session is resident.
        self.alphabet
            .recognize(&[vec![255; 96 * 96]])
            .map(|_| ())
            .map_err(scan_error)
    }
    fn tracker(&self) -> Box<dyn MatrixGridTracker> {
        Box::new(Tracker(MatrixTracker::default()))
    }
    fn decode(&self, encoded: &[u8]) -> Result<MatrixPicture, MatrixScanError> {
        let picture = app_ocr::matrix_picture(encoded).map_err(scan_error)?;
        Ok(MatrixPicture {
            analysis: analysis(picture.analysis),
            full: full_luma(picture.full),
        })
    }
    fn analyse(&self, frame: &MatrixYuvFrame<'_>) -> Result<MatrixAnalysis, MatrixScanError> {
        app_ocr::matrix_analysis_from_yuv(&yuv_frame(frame))
            .map(analysis)
            .map_err(scan_error)
    }
    fn full_luma(&self, frame: &MatrixYuvFrame<'_>) -> Result<MatrixFullLuma, MatrixScanError> {
        app_ocr::matrix_full_luma_from_yuv(&yuv_frame(frame))
            .map(full_luma)
            .map_err(scan_error)
    }
    fn cut(
        &self,
        luma: MatrixLuma<'_>,
        body: [MatrixPoint; 4],
    ) -> Result<MatrixCut, MatrixScanError> {
        let cut = app_ocr::capture_matrix_keyframe(
            luma.luma,
            luma.width,
            luma.height,
            body.map(|p| [p.x, p.y]),
        )
        .map_err(scan_error)?;
        Ok(MatrixCut {
            quads: cut.quads.into_iter().map(|q| q.map(point)).collect(),
            glyphs: cut
                .glyphs
                .into_iter()
                .map(|g| MatrixCaptureGlyph {
                    gray: g.gray,
                    contrast: g.contrast,
                    sharpness: g.sharpness,
                    source_size: g.source_size,
                    centering: g.centering,
                    glare: g.glare,
                    acuity: g.acuity,
                })
                .collect(),
        })
    }
    fn recognize(&self, images: &[Vec<u8>]) -> Result<Vec<MatrixReading>, MatrixScanError> {
        Ok(self
            .alphabet
            .recognize(images)
            .map_err(scan_error)?
            .into_iter()
            .map(|r| MatrixReading {
                text: r.text,
                confidence: r.confidence,
            })
            .collect())
    }
}

/// The capability every shell hands to `MatrixCameraDriver::new`.
pub fn matrix_vision() -> Arc<dyn MatrixVision> {
    Arc::new(Vision::default())
}

#[cfg(all(test, matrix_ocr))]
#[path = "tests/vision.rs"]
mod tests;

#[cfg(all(test, not(matrix_ocr)))]
#[path = "tests/unavailable.rs"]
mod unavailable_tests;
