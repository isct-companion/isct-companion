//! One picture builder for every input. A photo becomes RGB, then the same
//! two sizes the desktop webview sends: the 960 px analysis planes and the
//! full-resolution luma (capped like a desktop keyframe), with the same
//! Rec.601 and R−G rounding. A native camera frame produces the two
//! separately: analysis planes on every frame, the full luma only for the
//! keyframes Core asks for.
use super::{decode_image, grid};
use crate::MatrixOcrError;
use app_core::matrix_scan::{
    MatrixAnalysis, MatrixFullLuma, MatrixPicture, MatrixYuvFrame, MatrixYuvMatrix,
};
use image::{imageops::FilterType, GrayImage, Luma, RgbImage};

/// Long side of the luma the cells are cut from (`KEYFRAME_SIDE` in the
/// desktop camera transport).
pub(crate) const KEYFRAME_SIDE: u32 = 2200;

fn scaled_size(width: u32, height: u32, side: u32) -> Option<(u32, u32)> {
    let scale = side as f64 / width.max(height) as f64;
    (scale < 1.).then(|| {
        (
            (width as f64 * scale).round().max(1.) as u32,
            (height as f64 * scale).round().max(1.) as u32,
        )
    })
}

fn scaled(image: &RgbImage, side: u32) -> Option<RgbImage> {
    scaled_size(image.width(), image.height(), side)
        .map(|(w, h)| image::imageops::resize(image, w, h, FilterType::Triangle))
}

fn analysis(image: &RgbImage) -> MatrixAnalysis {
    MatrixAnalysis {
        width: image.width(),
        height: image.height(),
        luma: image
            .pixels()
            .map(|p| grid::luma(p[0], p[1], p[2]))
            .collect(),
        chroma: image.pixels().map(|p| grid::chroma(p[0], p[1])).collect(),
    }
}

/// The Rec.601 luma plane cells are cut from.
pub(super) fn rec601(image: &RgbImage) -> GrayImage {
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let p = image.get_pixel(x, y);
        Luma([grid::luma(p[0], p[1], p[2])])
    })
}

fn full_luma(image: &RgbImage) -> MatrixFullLuma {
    MatrixFullLuma {
        width: image.width(),
        height: image.height(),
        luma: rec601(image).into_raw(),
    }
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
    Ok(MatrixPicture {
        analysis: analysis(reduced.as_ref().unwrap_or(full)),
        full: full_luma(full),
    })
}

/// An encoded photo, oriented and alpha-composited.
pub fn matrix_picture(bytes: &[u8]) -> Result<MatrixPicture, MatrixOcrError> {
    picture(&decode_image(bytes)?)
}

/// `f64::round` (half away from zero) for |x| < 2^52, without the libm call
/// it compiles to on baseline x86-64 and without branches: truncation and the
/// remainder are exact in that range, so the result is the same value.
#[inline(always)]
fn round(x: f64) -> f64 {
    let whole = x as i64 as f64;
    let rest = x - whole;
    whole + f64::from(u8::from(rest >= 0.5)) - f64::from(u8::from(rest <= -0.5))
}

#[inline(always)]
fn clamp(v: f64) -> u8 {
    round(v).clamp(0., 255.) as u8
}

/// A validated 4:2:0 frame with its conversion tabulated. Red depends only on
/// (Y, V) and blue only on (Y, U), so both are stored per code pair; green is
/// evaluated per pixel. Every entry is the f64 expression of the per-pixel
/// conversion, so the tables change speed, not one output byte.
struct Yuv<'a> {
    frame: &'a MatrixYuvFrame<'a>,
    luma: [f64; 256],
    green_u: [f64; 256],
    green_v: [f64; 256],
    /// Indexed by `Y << 8 | V`.
    red: Vec<u8>,
    /// Indexed by `Y << 8 | U`.
    blue: Vec<u8>,
}
impl<'a> Yuv<'a> {
    fn new(frame: &'a MatrixYuvFrame<'a>) -> Result<Self, MatrixOcrError> {
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
        let table =
            |f: &dyn Fn(f64) -> f64| -> [f64; 256] { std::array::from_fn(|code| f(code as f64)) };
        let luma = table(&|y| (y - y_offset) * y_scale);
        let (red, blue) = (
            table(&|v| kr * ((v - 128.) * c_scale)),
            table(&|u| kb * ((u - 128.) * c_scale)),
        );
        let pairs = |term: &[f64; 256]| -> Vec<u8> {
            (0..65536)
                .map(|i| clamp(luma[i >> 8] + term[i & 255]))
                .collect()
        };
        Ok(Self {
            frame,
            luma,
            green_u: table(&|u| kgu * ((u - 128.) * c_scale)),
            green_v: table(&|v| kgv * ((v - 128.) * c_scale)),
            red: pairs(&red),
            blue: pairs(&blue),
        })
    }
    #[inline(always)]
    fn rgb(&self, y: u8, u: u8, v: u8) -> [u8; 3] {
        let (y, u, v) = (y as usize, u as usize, v as usize);
        [
            self.red[y << 8 | v],
            clamp(self.luma[y] - self.green_u[u] - self.green_v[v]),
            self.blue[y << 8 | u],
        ]
    }
    /// Calls `pixel` with every converted pixel in row order.
    fn each(&self, mut pixel: impl FnMut([u8; 3])) {
        let f = self.frame;
        let (ys, uvs, ps) = (
            f.y_row_stride as usize,
            f.uv_row_stride as usize,
            f.uv_pixel_stride as usize,
        );
        for y in 0..f.height as usize {
            let row = &f.y[y * ys..y * ys + f.width as usize];
            let chroma = (y / 2) * uvs;
            for (x, &luma) in row.iter().enumerate() {
                let c = chroma + (x / 2) * ps;
                pixel(self.rgb(luma, f.u[c], f.v[c]));
            }
        }
    }
    fn to_rgb(&self) -> RgbImage {
        let mut data =
            Vec::with_capacity(self.frame.width as usize * self.frame.height as usize * 3);
        self.each(|p| data.extend_from_slice(&p));
        RgbImage::from_raw(self.frame.width, self.frame.height, data).unwrap_or_default()
    }
    /// One chroma plane at its own (half) resolution, without the pixel stride.
    fn chroma_plane(&self, plane: &[u8]) -> (u32, u32, Vec<u8>) {
        let f = self.frame;
        let (cw, ch) = (f.width.div_ceil(2), f.height.div_ceil(2));
        let (uvs, ps) = (f.uv_row_stride as usize, f.uv_pixel_stride as usize);
        let mut data = Vec::with_capacity(cw as usize * ch as usize);
        for y in 0..ch as usize {
            let row = &plane[y * uvs..];
            data.extend((0..cw as usize).map(|x| row[x * ps]));
        }
        (cw, ch, data)
    }
}

/// One 8-bit plane resampled with a triangle (bilinear) filter. The typed
/// call instantiates only the one-channel kernels; the untyped one compiles
/// every pixel format in (about 2 MB of the Android library).
fn resample(
    (width, height, data): (u32, u32, Vec<u8>),
    to: (u32, u32),
) -> Result<Vec<u8>, MatrixOcrError> {
    use fast_image_resize::{
        images::{TypedImage, TypedImageRef},
        pixels::U8,
        FilterType, ResizeAlg, ResizeOptions,
    };
    if (width, height) == to {
        return Ok(data);
    }
    let failed = |e: &dyn std::fmt::Display| MatrixOcrError::InvalidImage(e.to_string());
    let source = TypedImageRef::<U8>::from_buffer(width, height, &data).map_err(|e| failed(&e))?;
    let mut target = TypedImage::<U8>::new(to.0, to.1);
    fast_image_resize::Resizer::new()
        .resize_typed(
            &source,
            &mut target,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear)),
        )
        .map_err(|e| failed(&e))?;
    Ok(target.pixels().iter().map(|p| p.0).collect())
}

/// The analysis planes of a native camera frame. Luma and chroma are reduced
/// to the analysis size first and converted there, so the full frame is never
/// converted to RGB on the fast loop; the tracker reads luma steps and the
/// sign of R−G, which this order preserves up to rounding.
pub fn matrix_analysis_from_yuv(
    frame: &MatrixYuvFrame<'_>,
) -> Result<MatrixAnalysis, MatrixOcrError> {
    let yuv = Yuv::new(frame)?;
    let (w, h) = (frame.width, frame.height);
    let (cw, ch) = scaled_size(w, h, KEYFRAME_SIDE).unwrap_or((w, h));
    let size = scaled_size(cw, ch, grid::WORK_SIDE).unwrap_or((cw, ch));
    let ys = frame.y_row_stride as usize;
    let mut luma = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h as usize {
        luma.extend_from_slice(&frame.y[y * ys..y * ys + w as usize]);
    }
    let luma = resample((w, h, luma), size)?;
    let u = resample(yuv.chroma_plane(frame.u), size)?;
    let v = resample(yuv.chroma_plane(frame.v), size)?;
    let pixels = luma.len();
    let (mut analysis_luma, mut chroma) = (Vec::with_capacity(pixels), Vec::with_capacity(pixels));
    for ((y, u), v) in luma.iter().zip(&u).zip(&v) {
        let [r, g, b] = yuv.rgb(*y, *u, *v);
        analysis_luma.push(grid::luma(r, g, b));
        chroma.push(grid::chroma(r, g));
    }
    Ok(MatrixAnalysis {
        width: size.0,
        height: size.1,
        luma: analysis_luma,
        chroma,
    })
}

/// The full-resolution luma of a native camera frame, the plane cells are cut
/// from: the Rec.601 luma of each converted pixel, byte for byte what the
/// training exports were cut from. A frame beyond the keyframe size is
/// converted, reduced and measured like a photo.
pub fn matrix_full_luma_from_yuv(
    frame: &MatrixYuvFrame<'_>,
) -> Result<MatrixFullLuma, MatrixOcrError> {
    let yuv = Yuv::new(frame)?;
    if frame.width.max(frame.height) > KEYFRAME_SIDE {
        let rgb = yuv.to_rgb();
        return Ok(full_luma(&scaled(&rgb, KEYFRAME_SIDE).unwrap_or(rgb)));
    }
    let (w, ys, uvs, ps) = (
        frame.width as usize,
        frame.y_row_stride as usize,
        frame.uv_row_stride as usize,
        frame.uv_pixel_stride as usize,
    );
    let mut luma = Vec::with_capacity(w * frame.height as usize);
    for y in 0..frame.height as usize {
        let row = &frame.y[y * ys..y * ys + w];
        let (u_row, v_row) = (&frame.u[(y / 2) * uvs..], &frame.v[(y / 2) * uvs..]);
        luma.extend(row.iter().enumerate().map(|(x, &code)| {
            let [r, g, b] = yuv.rgb(code, u_row[(x / 2) * ps], v_row[(x / 2) * ps]);
            grid::luma(r, g, b)
        }));
    }
    Ok(MatrixFullLuma {
        width: frame.width,
        height: frame.height,
        luma,
    })
}
