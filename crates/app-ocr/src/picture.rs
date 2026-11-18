//! Image inputs and buffers owned by the OCR engine, independent of Core.
use crate::MatrixAnalysisImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatrixYuvMatrix {
    Bt601,
    Bt709,
}
/// One native 4:2:0 YUV frame. Interleaved UV planes borrow the same slice
/// with a one-byte offset and a pixel stride of two.
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
/// Full-resolution Rec.601 luma, from which the engine cuts cells.
pub struct MatrixFullLuma {
    pub width: u32,
    pub height: u32,
    pub luma: Vec<u8>,
}
/// One decoded picture at tracking resolution and at cell-cut resolution.
pub struct MatrixPicture {
    pub analysis: MatrixAnalysisImage,
    pub full: MatrixFullLuma,
}
