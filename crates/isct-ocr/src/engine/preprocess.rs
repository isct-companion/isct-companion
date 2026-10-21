use image::{GrayImage, Luma, RgbImage};

pub struct Binary {
    pub cleaned: GrayImage,
    pub original: GrayImage,
    pub ink: usize,
}
impl Binary {
    pub(super) fn has_glyph(&self) -> bool {
        let pixels = self.cleaned.width() as f64 * self.cleaned.height() as f64;
        self.ink as f64 >= pixels * 12. / 4096. && self.ink as f64 <= pixels * 0.55
    }
}
/// Preserve the previous grayscale / unsharp / 15×15 mean-minus-10 pipeline.
pub fn binarize(image: &RgbImage) -> Binary {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let gray: Vec<f32> = image
        .pixels()
        .map(|p| (p[0] as f64 * 0.299 + p[1] as f64 * 0.587 + p[2] as f64 * 0.114) as f32)
        .collect();
    let mut sharp = vec![0f32; w * h];
    let kernel = [1., 2., 1.];
    for y in 0..h {
        for x in 0..w {
            let mut blurred = 0.;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                    let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                    blurred += gray[sy * w + sx] as f64
                        * kernel[(dx + 1) as usize]
                        * kernel[(dy + 1) as usize]
                        / 16.;
                }
            }
            sharp[y * w + x] = (2. * gray[y * w + x] as f64 - blurred).clamp(0., 255.) as f32;
        }
    }
    let stride = w + 1;
    let mut integral = vec![0.; stride * (h + 1)];
    for y in 0..h {
        let mut sum = 0.;
        for x in 0..w {
            sum += sharp[y * w + x] as f64;
            integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + sum;
        }
    }
    let original = GrayImage::from_fn(w as u32, h as u32, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let (left, right, top, bottom) = (
            x.saturating_sub(7),
            (x + 8).min(w),
            y.saturating_sub(7),
            (y + 8).min(h),
        );
        let mean = (integral[bottom * stride + right]
            - integral[top * stride + right]
            - integral[bottom * stride + left]
            + integral[top * stride + left])
            / ((right - left) * (bottom - top)) as f64;
        Luma([if (sharp[y * w + x] as f64) < mean - 10. {
            0
        } else {
            255
        }])
    });
    let (cleaned, ink) = remove_specks(&original);
    Binary {
        cleaned,
        original,
        ink,
    }
}
pub fn remove_specks(image: &GrayImage) -> (GrayImage, usize) {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut cleaned = image.clone();
    let mut visited = vec![false; w * h];
    let mut ink = 0;
    for seed in 0..w * h {
        if visited[seed] || image.as_raw()[seed] >= 128 {
            continue;
        }
        let mut component = vec![seed];
        visited[seed] = true;
        let mut head = 0;
        while head < component.len() {
            let p = component[head];
            head += 1;
            let (x, y) = ((p % w) as i32, (p / w) as i32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let next = ny as usize * w + nx as usize;
                    if !visited[next] && image.as_raw()[next] < 128 {
                        visited[next] = true;
                        component.push(next);
                    }
                }
            }
        }
        if component.len() < 8 {
            for p in component {
                cleaned.as_mut()[p] = 255;
            }
        } else {
            ink += component.len();
        }
    }
    (cleaned, ink)
}
pub struct Prepared {
    pub tensor: Vec<f32>,
    #[cfg(test)]
    pub preview: GrayImage,
}
/// Tight glyph bounds + 8 white pixels; half-pixel bilinear resize, CHW, zero padding.
pub fn prepare(image: &GrayImage) -> Prepared {
    prepare_width(image, 320)
}
pub(super) fn prepare_width(image: &GrayImage, width: usize) -> Prepared {
    let (mut left, mut top, mut right, mut bottom) =
        (image.width() as i32, image.height() as i32, -1, -1);
    for (x, y, p) in image.enumerate_pixels() {
        if p[0] < 128 {
            left = left.min(x as i32);
            top = top.min(y as i32);
            right = right.max(x as i32);
            bottom = bottom.max(y as i32);
        }
    }
    if right < left {
        (left, top, right, bottom) = (0, 0, image.width() as i32 - 1, image.height() as i32 - 1);
    }
    let (w, h) = (right - left + 17, bottom - top + 17);
    let rw = ((48. * w as f64 / h as f64).ceil() as usize).min(width);
    let plane = 48 * width;
    let mut tensor = vec![0.; 3 * plane];
    #[cfg(test)]
    let mut preview = GrayImage::from_pixel(width as u32, 48, Luma([128]));
    let value = |x: i32, y: i32| {
        if x < 8 || y < 8 || x >= w - 8 || y >= h - 8 {
            255.
        } else {
            image.get_pixel((x - 8 + left) as u32, (y - 8 + top) as u32)[0] as f64
        }
    };
    for y in 0..48 {
        for x in 0..rw {
            let sx = ((x as f64 + 0.5) * w as f64 / rw as f64 - 0.5).clamp(0., (w - 1) as f64);
            let sy = ((y as f64 + 0.5) * h as f64 / 48. - 0.5).clamp(0., (h - 1) as f64);
            let (x0, y0) = (sx.floor() as i32, sy.floor() as i32);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (sx - x0 as f64, sy - y0 as f64);
            let pixel = ((value(x0, y0) * (1. - fx) + value(x1, y0) * fx) * (1. - fy)
                + (value(x0, y1) * (1. - fx) + value(x1, y1) * fx) * fy)
                .round() as u8;
            #[cfg(test)]
            preview.put_pixel(x as u32, y as u32, Luma([pixel]));
            // Grayscale channels are identical in BGR and RGB.
            for channel in 0..3 {
                tensor[channel * plane + y * width + x] = (pixel as f64 / 127.5 - 1.) as f32;
            }
        }
    }
    Prepared {
        tensor,
        #[cfg(test)]
        preview,
    }
}
