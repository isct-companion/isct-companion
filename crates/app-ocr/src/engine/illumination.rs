//! Card-specific illumination measurements; morphology/filtering use imageproc.
use image::{imageops::FilterType, GrayImage, Luma};
use imageproc::{
    filter::gaussian_blur_f32,
    morphology::{grayscale_dilate, Mask},
};

pub(super) struct Illumination {
    pub corrected: Option<GrayImage>,
    pub glare: f64,
    pub paper: f64,
    clipped: f64,
}
impl Illumination {
    pub fn glare_against(&self, paper: f64) -> f64 {
        // A reflection can saturate an entire cell's paper field, hiding its
        // local gradient. Compare with the card too; uniformly white cards
        // still have no relative brightness excess.
        self.glare
            .max(self.clipped * ((self.paper - paper - 20.) / 80.).clamp(0., 1.))
    }
}

fn correct(gray: &GrayImage, background: &GrayImage, target: f64, max_gain: f64) -> GrayImage {
    let field = image::imageops::resize(
        background,
        gray.width(),
        gray.height(),
        FilterType::Triangle,
    );
    GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let gain = (target / field.get_pixel(x, y)[0].max(1) as f64).clamp(0.75, max_gain);
        Luma([(gray.get_pixel(x, y)[0] as f64 * gain).round().min(255.) as u8])
    })
}

pub(super) fn analyze(gray: &GrayImage) -> Illumination {
    // Estimate paper brightness at a lower resolution, excluding narrow ink.
    // The max filter preserves a shadow boundary without treating a letter's
    // dark strokes as illumination. A bounded gain avoids lifting black noise.
    let small = image::imageops::resize(gray, 32, 32, FilterType::Triangle);
    let background = gaussian_blur_f32(&grayscale_dilate(&small, &Mask::square(3)), 1.2);
    let mut levels = background.as_raw().clone();
    levels.sort_unstable();
    let low = levels[levels.len() / 10] as f64;
    let high = levels[levels.len() * 9 / 10] as f64;
    let clipped =
        gray.as_raw().iter().filter(|&&p| p >= 248).count() as f64 / gray.as_raw().len() as f64;
    // A uniformly white card is not glare. Require a clipped region AND a
    // substantial change in estimated paper brightness within the same cell.
    let glare = clipped * ((high - low - 20.) / 80.).clamp(0., 1.);
    let corrected = if high < 155. || high - low > 35. {
        // A card at 15% exposure needs about 6.5x gain to restore paper.
        // Admission still measures stroke texture and rejects saturated glare.
        Some(correct(gray, &background, 210., 8.))
    } else {
        None
    };
    Illumination {
        corrected,
        glare,
        paper: high,
        clipped,
    }
}
