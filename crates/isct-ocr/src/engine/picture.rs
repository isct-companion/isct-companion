//! One picture builder for every input. A photo and a native camera frame
//! both become RGB, then the same two sizes the desktop webview sends: the
//! 960 px analysis planes and the full-resolution luma (capped like a
//! desktop keyframe), with the same Rec.601 and R−G rounding.
use super::{decode_image, grid};
use crate::MatrixOcrError;
use image::{imageops::FilterType, Rgb, RgbImage};
use isct_core::matrix_scan::{MatrixPicture, MatrixYuvFrame, MatrixYuvMatrix};

/// Long side of the luma the cells are cut from (`KEYFRAME_SIDE` in the
/// desktop camera transport).
pub(crate) const KEYFRAME_SIDE: u32 = 2200;

fn scaled(image: &RgbImage, side: u32) -> Option<RgbImage> {
    let scale = side as f64 / image.width().max(image.height()) as f64;
    (scale < 1.).then(|| {
        image::imageops::resize(
            image,
            (image.width() as f64 * scale).round().max(1.) as u32,
            (image.height() as f64 * scale).round().max(1.) as u32,
            FilterType::Triangle,
        )
    })
}

fn picture(image: &RgbImage) -> Result<MatrixPicture, MatrixOcrError> {
    if image.width() < 32 || image.height() < 32 {
        return Err(MatrixOcrError::InvalidImage("Image is too small.".into()));
    }
    // The analysis planes are reduced from the capped picture, as the
    // webview draws its small canvas from the full one.
    let capped = scaled(image, KEYFRAME_SIDE);
    let full = capped.as_ref().unwrap_or(image);
    let reduced = scaled(full, grid::WORK_SIDE);
    let small = reduced.as_ref().unwrap_or(full);
    Ok(MatrixPicture {
        analysis_width: small.width(),
        analysis_height: small.height(),
        analysis_luma: small
            .pixels()
            .map(|p| grid::luma(p[0], p[1], p[2]))
            .collect(),
        analysis_chroma: small.pixels().map(|p| grid::chroma(p[0], p[1])).collect(),
        width: full.width(),
        height: full.height(),
        luma: full
            .pixels()
            .map(|p| grid::luma(p[0], p[1], p[2]))
            .collect(),
    })
}

/// An encoded photo, oriented and alpha-composited.
pub fn matrix_picture(bytes: &[u8]) -> Result<MatrixPicture, MatrixOcrError> {
    picture(&decode_image(bytes)?)
}

/// A native 4:2:0 camera frame, converted to RGB with its own matrix/range.
pub fn matrix_picture_from_yuv(
    frame: &MatrixYuvFrame<'_>,
) -> Result<MatrixPicture, MatrixOcrError> {
    picture(&yuv_to_rgb(frame)?)
}

pub(crate) fn yuv_to_rgb(frame: &MatrixYuvFrame<'_>) -> Result<RgbImage, MatrixOcrError> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let (ys, uvs, ps) = (
        frame.y_row_stride as usize,
        frame.uv_row_stride as usize,
        frame.uv_pixel_stride as usize,
    );
    let chroma_end =
        |plane: &[u8]| h > 0 && w > 0 && plane.len() > ((h - 1) / 2) * uvs + ((w - 1) / 2) * ps;
    if !(32..=8192).contains(&w)
        || !(32..=8192).contains(&h)
        || ys < w
        || ps == 0
        || frame.y.len() < (h - 1) * ys + w
        || !chroma_end(frame.u)
        || !chroma_end(frame.v)
    {
        return Err(MatrixOcrError::InvalidImage(
            "Invalid camera frame planes.".into(),
        ));
    }
    let (kr, kgu, kgv, kb) = match frame.matrix {
        MatrixYuvMatrix::Bt601 => (1.402, 0.344136, 0.714136, 1.772),
        MatrixYuvMatrix::Bt709 => (1.5748, 0.187324, 0.468124, 1.8556),
    };
    let (y_scale, y_offset, c_scale) = if frame.full_range {
        (1., 0., 1.)
    } else {
        (255. / 219., 16., 255. / 224.)
    };
    let clamp = |v: f64| v.round().clamp(0., 255.) as u8;
    Ok(RgbImage::from_fn(frame.width, frame.height, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let luma = (frame.y[y * ys + x] as f64 - y_offset) * y_scale;
        let c = (y / 2) * uvs + (x / 2) * ps;
        let cb = (frame.u[c] as f64 - 128.) * c_scale;
        let cr = (frame.v[c] as f64 - 128.) * c_scale;
        Rgb([
            clamp(luma + kr * cr),
            clamp(luma - kgu * cb - kgv * cr),
            clamp(luma + kb * cb),
        ])
    }))
}
