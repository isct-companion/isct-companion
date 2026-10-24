//! Live Matrix camera for the native shells. Core's `MatrixCameraDriver`
//! runs the session exactly as on desktop; a phone hands it one YUV frame
//! at a time and draws the state it returns.
#[cfg(not(feature = "ocr"))]
use crate::session::unavailable;
use crate::{SdkError, Session};
#[cfg(feature = "ocr")]
use isct_core::matrix_scan::{
    MatrixCameraDriver, MatrixCameraSnapshot, MatrixScanError, MatrixYuvFrame, MatrixYuvMatrix,
};
use std::sync::Arc;

/// One 4:2:0 camera frame. NV12 (iOS) passes its CbCr plane as `u` and the
/// same plane without its first byte as `v`, with `uv_pixel_stride` 2;
/// Android passes `YUV_420_888`'s planes with their own strides.
#[derive(Clone, Debug, uniffi::Record)]
pub struct MatrixCameraFrame {
    /// Increasing per session; the drawn outline names the frame it fits.
    pub frame_id: u32,
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub y_row_stride: u32,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub uv_row_stride: u32,
    pub uv_pixel_stride: u32,
    /// BT.709 (HD video) rather than BT.601 coefficients.
    pub bt709: bool,
    pub full_range: bool,
    /// Capture time in milliseconds, increasing.
    pub captured_ms: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum MatrixCameraPhase {
    Capturing,
    Processing,
    Complete,
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct MatrixPoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct MatrixCell {
    pub text: String,
    pub confidence: f64,
    /// Confirmed by enough independent observations.
    pub stable: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct MatrixCameraState {
    pub phase: MatrixCameraPhase,
    /// Card outline in the frame's normalized coordinates (A1 top-left,
    /// clockwise), empty when no card is in view.
    pub bounds: Vec<MatrixPoint>,
    pub bounds_frame_id: Option<u32>,
    pub ready_cells: u32,
    pub elapsed_ms: f64,
    pub timed_out: bool,
    /// Row order A1…J1, A2…; blank until a cell has evidence.
    pub cells: Vec<MatrixCell>,
}
#[cfg(feature = "ocr")]
impl From<MatrixCameraSnapshot> for MatrixCameraState {
    fn from(s: MatrixCameraSnapshot) -> Self {
        use isct_core::matrix_scan::MatrixCameraPhase as P;
        Self {
            phase: match s.phase {
                P::Capturing => MatrixCameraPhase::Capturing,
                P::Processing => MatrixCameraPhase::Processing,
                P::Complete => MatrixCameraPhase::Complete,
            },
            bounds: s
                .bounds
                .map(|b| b.iter().map(|p| MatrixPoint { x: p.x, y: p.y }).collect())
                .unwrap_or_default(),
            bounds_frame_id: s.bounds_frame_id,
            ready_cells: s.ready_cells as u32,
            elapsed_ms: s.capture_elapsed_ms,
            timed_out: s.timed_out,
            cells: s
                .cells
                .into_iter()
                .map(|c| MatrixCell {
                    text: c.text,
                    confidence: c.confidence,
                    stable: c.stable,
                })
                .collect(),
        }
    }
}
#[cfg(feature = "ocr")]
pub(crate) fn scan_error(e: MatrixScanError) -> SdkError {
    match e {
        MatrixScanError::Ended => SdkError::Cancelled,
        MatrixScanError::InvalidFrame(_) | MatrixScanError::Unreadable(_) => {
            SdkError::InvalidInput {
                detail: e.to_string(),
            }
        }
        MatrixScanError::Backend(detail) => SdkError::Unavailable { detail },
    }
}

/// One camera session. Its OCR consumer runs on its own thread from
/// `begin_matrix_camera` until the session completes or stops; dropping the
/// object stops it.
#[derive(uniffi::Object)]
pub struct MatrixCameraSession {
    #[cfg(feature = "ocr")]
    driver: Arc<MatrixCameraDriver>,
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    id: String,
}
#[uniffi::export]
impl MatrixCameraSession {
    /// Tracks the frame and, when Core asks for it, reads its cells. Call
    /// from the camera queue, one frame at a time; drop frames meanwhile.
    pub fn submit(&self, frame: MatrixCameraFrame) -> Result<MatrixCameraState, SdkError> {
        #[cfg(feature = "ocr")]
        {
            let v = MatrixYuvFrame {
                width: frame.width,
                height: frame.height,
                y: &frame.y,
                y_row_stride: frame.y_row_stride,
                u: &frame.u,
                v: &frame.v,
                uv_row_stride: frame.uv_row_stride,
                uv_pixel_stride: frame.uv_pixel_stride,
                matrix: if frame.bt709 {
                    MatrixYuvMatrix::Bt709
                } else {
                    MatrixYuvMatrix::Bt601
                },
                full_range: frame.full_range,
            };
            self.driver
                .submit(&self.id, frame.frame_id, &v, frame.captured_ms)
                .map(Into::into)
                .map_err(scan_error)
        }
        #[cfg(not(feature = "ocr"))]
        {
            let _ = frame;
            Err(unavailable("OCR backend"))
        }
    }
    /// A wakeup without a frame: the ten-second budget still advances.
    pub fn poll(&self) -> Result<MatrixCameraState, SdkError> {
        #[cfg(feature = "ocr")]
        {
            self.driver
                .poll(&self.id)
                .map(Into::into)
                .map_err(scan_error)
        }
        #[cfg(not(feature = "ocr"))]
        Err(unavailable("OCR backend"))
    }
    pub fn stop(&self) {
        #[cfg(feature = "ocr")]
        self.driver.stop(&self.id);
    }
}
impl Drop for MatrixCameraSession {
    fn drop(&mut self) {
        #[cfg(feature = "ocr")]
        self.driver.stop(&self.id);
    }
}

#[uniffi::export]
impl Session {
    /// Starts a live scan; the ten-second budget starts now.
    pub fn begin_matrix_camera(&self) -> Result<Arc<MatrixCameraSession>, SdkError> {
        #[cfg(feature = "ocr")]
        {
            let driver = self.inner.matrix.clone();
            let id = driver.begin().map_err(scan_error)?;
            let consumer = (driver.clone(), id.clone());
            std::thread::Builder::new()
                .name("matrix-ocr".into())
                .spawn(move || {
                    // Ends with the session: completion, stop or the deadline.
                    let _ = consumer.0.process(&consumer.1, |_| true);
                })
                .map_err(|e| SdkError::Internal {
                    detail: e.to_string(),
                })?;
            Ok(Arc::new(MatrixCameraSession { driver, id }))
        }
        #[cfg(not(feature = "ocr"))]
        Err(unavailable("OCR backend"))
    }
}
