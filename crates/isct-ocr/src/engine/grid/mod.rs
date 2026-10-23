//! Matrix card geometry: detect, fit, track. One homography maps the printed
//! grid model to the image; every other output (body corners, 70 cell quads,
//! the drawn outline) is derived from it.
//!
//! Model: cell units, u along the 10 columns, v along the 7 rows, A1 top-left
//! at (0, 0). The card prints a dark header band above row 1, dark/grey row
//! labels left of column A, and alternating pink (rows 1, 3, 5, 7) and white
//! rows with no lines between columns. The offline OpenCV reference
//! (`scripts/ocr/matrix_cv_reference.py`) documents every step and threshold;
//! this module follows it and is compared with it frame by frame.
mod detect;
mod fit;
mod track;

pub(crate) use detect::detect;
pub(crate) use track::{TrackMode, Tracker};

use super::vendor::opencv_homography::Homography;
use image::{GrayImage, RgbImage};

pub(crate) const COLS: f64 = 10.;
pub(crate) const ROWS: f64 = 7.;
/// Long side of the analysed image, in pixels.
pub(crate) const WORK_SIDE: u32 = 960;

/// The two planes the geometry reads: Rec.601 luma and R−G stored as +128.
pub(crate) struct Planes {
    pub(crate) luma: GrayImage,
    pub(crate) chroma: Vec<u8>,
}
impl Planes {
    pub(crate) fn new(mut luma: GrayImage, mut chroma: Vec<u8>) -> Option<Self> {
        if luma.width() < 32
            || luma.height() < 32
            || chroma.len() != luma.width() as usize * luma.height() as usize
        {
            return None;
        }
        // Exposure: lift a dim frame so its bright paper reaches about 220.
        // The step floors are absolute, and a dim card's pink rows would
        // otherwise differ from white by less than the R-G floor.
        let mut histogram = [0usize; 256];
        for &p in luma.as_raw() {
            histogram[p as usize] += 1;
        }
        let target = luma.as_raw().len() * 98 / 100;
        let (mut count, mut p98) = (0, 255);
        for (level, &n) in histogram.iter().enumerate() {
            count += n;
            if count >= target {
                p98 = level;
                break;
            }
        }
        if p98 < 200 {
            let gain = (220. / p98.max(1) as f64).min(6.);
            for p in luma.iter_mut() {
                *p = (*p as f64 * gain).round().min(255.) as u8;
            }
            for c in chroma.iter_mut() {
                *c = (128. + (*c as f64 - 128.) * gain).round().clamp(0., 255.) as u8;
            }
        }
        Some(Self { luma, chroma })
    }
    /// Planes of an RGB image scaled to the analysis size.
    pub(crate) fn from_rgb(image: &RgbImage) -> Option<Self> {
        let scale = WORK_SIDE as f64 / image.width().max(image.height()) as f64;
        let small;
        let source = if scale < 1. {
            small = image::imageops::resize(
                image,
                (image.width() as f64 * scale).round().max(1.) as u32,
                (image.height() as f64 * scale).round().max(1.) as u32,
                image::imageops::FilterType::Triangle,
            );
            &small
        } else {
            image
        };
        let luma = GrayImage::from_fn(source.width(), source.height(), |x, y| {
            let p = source.get_pixel(x, y);
            image::Luma([luma(p[0], p[1], p[2])])
        });
        let chroma = source.pixels().map(|p| chroma(p[0], p[1])).collect();
        Self::new(luma, chroma)
    }
    pub(crate) fn width(&self) -> f64 {
        self.luma.width() as f64
    }
    pub(crate) fn height(&self) -> f64 {
        self.luma.height() as f64
    }
    fn plane(&self, channel: Channel) -> (&[u8], f64) {
        match channel {
            Channel::Luma => (self.luma.as_raw(), 0.),
            Channel::Chroma => (&self.chroma, 128.),
        }
    }
    /// Bilinear sample, edge pixels replicated (`cv::remap`, BORDER_REPLICATE).
    pub(crate) fn sample(&self, channel: Channel, p: [f64; 2]) -> f64 {
        let (data, offset) = self.plane(channel);
        let (w, h) = (self.luma.width() as usize, self.luma.height() as usize);
        if !p[0].is_finite() || !p[1].is_finite() {
            return 0.;
        }
        let x = p[0].clamp(0., (w - 1) as f64);
        let y = p[1].clamp(0., (h - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = (x - x0 as f64, y - y0 as f64);
        let at = |x: usize, y: usize| data[y * w + x] as f64;
        let top = at(x0, y0) * (1. - fx) + at(x1, y0) * fx;
        let bottom = at(x0, y1) * (1. - fx) + at(x1, y1) * fx;
        top * (1. - fy) + bottom * fy - offset
    }
}

/// Rec.601 luma with the training pipeline's integer rounding.
pub(crate) fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((19595 * r as u32 + 38470 * g as u32 + 7471 * b as u32 + 32768) >> 16) as u8
}
/// Pink rows are positive in R−G; stored with +128 and clamped.
pub(crate) fn chroma(r: u8, g: u8) -> u8 {
    (r as i32 - g as i32 + 128).clamp(0, 255) as u8
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Channel {
    Luma,
    Chroma,
}

pub(crate) fn apply(h: &Homography, [u, v]: [f64; 2]) -> [f64; 2] {
    let w = h[6] * u + h[7] * v + h[8];
    [
        (h[0] * u + h[1] * v + h[2]) / w,
        (h[3] * u + h[4] * v + h[5]) / w,
    ]
}

pub(crate) fn multiply(a: &Homography, b: &Homography) -> Homography {
    let mut out = [0.; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    out
}

/// Analysis pixels (centres at integers, as sampled here) to normalized image
/// coordinates (0 and 1 at the outer pixel edges), which the crop code and
/// every other resolution of the same frame share.
pub(crate) fn normalize(quad: [[f64; 2]; 4], planes: &Planes) -> [[f64; 2]; 4] {
    quad.map(|[x, y]| [(x + 0.5) / planes.width(), (y + 0.5) / planes.height()])
}

/// Body corners A1 top-left, clockwise, in image pixels.
pub(crate) fn body(h: &Homography) -> [[f64; 2]; 4] {
    [[0., 0.], [COLS, 0.], [COLS, ROWS], [0., ROWS]].map(|p| apply(h, p))
}

pub(crate) fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Mean column pitch in pixels.
pub(crate) fn cell_px(h: &Homography) -> f64 {
    let b = body(h);
    (distance(b[1], b[0]) + distance(b[2], b[3])) / (2. * COLS)
}

/// Homography from four model points to four image points.
pub(crate) fn from_quad(model: [[f64; 2]; 4], image: [[f64; 2]; 4]) -> Option<Homography> {
    // An exact four-point solve is the minimal case of the vendored DLT.
    super::vendor::opencv_homography::find_homography(&model, &image, 1., 1, 0.99).map(|(h, _)| h)
}

/// Body quad (normalized, A1 top-left clockwise) to the 70 cell quads in
/// the coordinates of an image of `width` x `height`.
pub(crate) fn cells(body: [[f64; 2]; 4], width: f64, height: f64) -> Option<Vec<[[f64; 2]; 4]>> {
    let image = body.map(|[x, y]| [x * width, y * height]);
    let h = from_quad([[0., 0.], [COLS, 0.], [COLS, ROWS], [0., ROWS]], image)?;
    Some(
        (0..70)
            .map(|i| {
                let (c, r) = ((i % 10) as f64, (i / 10) as f64);
                [[c, r], [c + 1., r], [c + 1., r + 1.], [c, r + 1.]].map(|p| apply(&h, p))
            })
            .collect(),
    )
}

#[cfg(test)]
fn invert(h: &Homography) -> Option<Homography> {
    let m = nalgebra::Matrix3::from_row_slice(h).try_inverse()?;
    Some(std::array::from_fn(|i| m[(i / 3, i % 3)]))
}

#[cfg(test)]
#[path = "../../tests/engine__grid.rs"]
mod tests;
