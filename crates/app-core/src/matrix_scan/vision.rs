//! The image capability behind every Matrix scan. Core owns sessions,
//! admission, scheduling and votes; an adapter (app-ocr) owns pixels, grid
//! geometry and the letter model. Desktop, iOS and Android reach it only
//! through `MatrixCameraDriver`, so a still photo and a camera frame are read
//! by the same detector, the same cell cut and the same model.
use super::*;

/// Two row-major planes of one byte per pixel: Rec.601 luma and
/// `clamp(R - G + 128, 0, 255)`, the input of the grid tracker.
#[derive(Clone, Copy)]
pub struct MatrixPlanes<'a> {
    pub width: u32,
    pub height: u32,
    pub luma: &'a [u8],
    pub chroma: &'a [u8],
}
/// A full-resolution Rec.601 luma plane the 70 cells are cut from.
#[derive(Clone, Copy)]
pub struct MatrixLuma<'a> {
    pub width: u32,
    pub height: u32,
    pub luma: &'a [u8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixYuvMatrix {
    Bt601,
    Bt709,
}
/// One native camera frame in 4:2:0 YUV. `u`/`v` may alias one interleaved
/// plane (NV12/NV21): pass the plane twice, the second offset by one byte,
/// with `uv_pixel_stride` 2.
pub struct MatrixYuvFrame<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u8],
    pub y_row_stride: u32,
    pub u: &'a [u8],
    pub v: &'a [u8],
    pub uv_row_stride: u32,
    pub uv_pixel_stride: u32,
    pub matrix: MatrixYuvMatrix,
    pub full_range: bool,
}
/// One picture in the two sizes the pipeline reads: the analysis planes for
/// the grid tracker and the full-resolution luma for the cell cut.
pub struct MatrixPicture {
    pub analysis_width: u32,
    pub analysis_height: u32,
    pub analysis_luma: Vec<u8>,
    pub analysis_chroma: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub luma: Vec<u8>,
}
impl MatrixPicture {
    pub fn planes(&self) -> MatrixPlanes<'_> {
        MatrixPlanes {
            width: self.analysis_width,
            height: self.analysis_height,
            luma: &self.analysis_luma,
            chroma: &self.analysis_chroma,
        }
    }
    pub fn full(&self) -> MatrixLuma<'_> {
        MatrixLuma {
            width: self.width,
            height: self.height,
            luma: &self.luma,
        }
    }
}
/// The tracker's verdict on one frame, in normalized frame coordinates.
pub struct MatrixGridTrack {
    /// Smoothed body corners, for drawing only.
    pub display: Option<[MatrixPoint; 4]>,
    /// Body corners as measured on this frame (A1 top-left, clockwise).
    pub body: Option<[MatrixPoint; 4]>,
    /// The fit on this frame is good enough to cut cells from.
    pub keyframe: bool,
    pub elapsed_ms: f64,
}
/// Whether the cells of a tracked frame may be cut for recognition: a fresh
/// fit whose whole body lies inside the frame. A grid reaching past the edge
/// is still drawn, but its cut-off cells would only feed the model fragments.
/// Live sessions and dataset exports decide keyframes here.
pub fn keyframe_ready(track: &MatrixGridTrack) -> bool {
    track.keyframe
        && track.body.is_some_and(|body| {
            body.iter()
                .all(|p| (0. ..=1.).contains(&p.x) && (0. ..=1.).contains(&p.y))
        })
}
/// The 70 cells cut from one picture with the grid fitted on it.
pub struct MatrixCut {
    pub quads: Vec<[MatrixPoint; 4]>,
    pub glyphs: Vec<MatrixCaptureGlyph>,
}
/// Grid state of one camera session; frames arrive in capture order.
pub trait MatrixGridTracker: Send {
    fn track(
        &mut self,
        planes: MatrixPlanes<'_>,
        time_s: f64,
    ) -> Result<MatrixGridTrack, MatrixScanError>;
}
pub trait MatrixVision: Send + Sync {
    /// Compile-time availability of the model backend.
    fn available(&self) -> bool;
    fn warm_up(&self) -> Result<(), MatrixScanError>;
    fn tracker(&self) -> Box<dyn MatrixGridTracker>;
    /// An encoded photo (PNG, JPEG, WebP), oriented and alpha-composited.
    fn decode(&self, encoded: &[u8]) -> Result<MatrixPicture, MatrixScanError>;
    /// A native camera frame.
    fn convert(&self, frame: &MatrixYuvFrame<'_>) -> Result<MatrixPicture, MatrixScanError>;
    fn cut(
        &self,
        luma: MatrixLuma<'_>,
        body: [MatrixPoint; 4],
    ) -> Result<MatrixCut, MatrixScanError>;
    /// At most ten 96×96 grayscale cells per call.
    fn recognize(&self, images: &[Vec<u8>]) -> Result<Vec<MatrixReading>, MatrixScanError>;
}
