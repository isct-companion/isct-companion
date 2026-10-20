//! Card-specific illumination measurements; morphology/filtering use imageproc.
use image::{imageops::FilterType, GrayImage, Luma, RgbImage};
use imageproc::{
    filter::gaussian_blur_f32,
    morphology::{grayscale_dilate, Mask},
};

pub(super) struct Illumination {
    pub corrected: Option<GrayImage>,
    pub glare: f64,
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

pub(super) fn geometry_image(image: &RgbImage) -> RgbImage {
    let gray = GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let p = image.get_pixel(x, y);
        Luma([(0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64).round() as u8])
    });
    let small = image::DynamicImage::ImageLuma8(gray.clone())
        .thumbnail(160, 160)
        .to_luma8();
    let background = gaussian_blur_f32(&grayscale_dilate(&small, &Mask::square(7)), 3.);
    image::DynamicImage::ImageLuma8(correct(&gray, &background, 220., 6.)).to_rgb8()
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
        Some(correct(gray, &background, 210., 4.))
    } else {
        None
    };
    Illumination { corrected, glare }
}
