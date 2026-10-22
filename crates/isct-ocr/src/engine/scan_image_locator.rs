//! Grayscale reference-line localization for Matrix still images and camera frames.
//! The alternating label strip seeds orientation; two-stage measured transitions
//! recover the body before equal-column and label-seam row splitting.
//! imageproc supplies the image primitives; this card-specific label pattern and
//! its shared perspective constraints need a dedicated fit.
use super::geometry;
#[path = "scan_image_locator/reference.rs"]
mod reference;
use image::{GrayImage, Luma, RgbImage};
use imageproc::{
    geometric_transformations::{rotate_about_center_no_crop, Border, Interpolation},
    region_labelling::{connected_components, Connectivity},
};

const ROWS: usize = 7;
const COLS: usize = 10;
#[derive(Clone, Copy)]
struct Blob {
    area: u32,
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    sum_x: f64,
    sum_y: f64,
}

impl Blob {
    fn empty() -> Self {
        Self {
            area: 0,
            min_x: u32::MAX,
            max_x: 0,
            min_y: u32::MAX,
            max_y: 0,
            sum_x: 0.,
            sum_y: 0.,
        }
    }

    fn center(self) -> geometry::Point {
        geometry::Point {
            x: self.sum_x / self.area as f64,
            y: self.sum_y / self.area as f64,
        }
    }

    fn size(self) -> f64 {
        ((self.max_x - self.min_x + 1) * (self.max_y - self.min_y + 1)) as f64
    }

    fn height_along(self, axis: geometry::Point) -> f64 {
        let width = (self.max_x - self.min_x + 1) as f64;
        let height = (self.max_y - self.min_y + 1) as f64;
        axis.x.abs() * width + axis.y.abs() * height
    }
}

#[derive(Clone, Copy)]
struct Stripe {
    top: geometry::Point,
    first: geometry::Point,
    middle: geometry::Point,
    bottom: geometry::Point,
    score: f64,
}

pub(super) fn locate(image: &RgbImage) -> Option<Vec<[geometry::Point; 4]>> {
    if image.width() < 32 || image.height() < 32 {
        return None;
    }

    // Keep the decoded source at full resolution. Rotation is only a temporary
    // measurement view; the returned quads are mapped back to source pixels.
    let gray = grayscale(image);
    for stripe in stripe_candidates(&gray) {
        let theta = (stripe.bottom.x - stripe.top.x).atan2(stripe.bottom.y - stripe.top.y) as f32;
        let upright = rotate_to_axis(&gray, theta);
        let centers = [stripe.top, stripe.first, stripe.middle, stripe.bottom]
            .map(|point| rotate_point(point, theta, gray.dimensions(), upright.dimensions()));
        let Some(rectified) = reference::seed(&upright, centers) else {
            continue;
        };
        let body = rectified
            .body
            .map(|point| unrotate_point(point, theta, gray.dimensions(), upright.dimensions()));
        return Some(refine_grid_cells(&gray, &body));
    }
    None
}
fn stripe_candidates(image: &GrayImage) -> Vec<Stripe> {
    let dark = GrayImage::from_fn(image.width(), image.height(), |x, y| {
        Luma([u8::from(image.get_pixel(x, y)[0] <= 90) * 255])
    });
    // Thin printed grid lines can join distinct dark labels into one component.
    // Opening only this proposal mask preserves the source grayscale measurements.
    let dark = imageproc::morphology::open(&dark, imageproc::distance_transform::Norm::LInf, 1);
    let labels = connected_components(&dark, Connectivity::Four, Luma([0]));
    let count = labels.as_raw().iter().copied().max().unwrap_or(0) as usize + 1;
    let mut components = vec![Blob::empty(); count];
    for (x, y, label) in labels.enumerate_pixels() {
        let id = label[0] as usize;
        if id == 0 {
            continue;
        }
        let blob = &mut components[id];
        blob.area += 1;
        blob.min_x = blob.min_x.min(x);
        blob.max_x = blob.max_x.max(x);
        blob.min_y = blob.min_y.min(y);
        blob.max_y = blob.max_y.max(y);
        blob.sum_x += x as f64;
        blob.sum_y += y as f64;
    }
    let mut blobs = components
        .into_iter()
        .filter(|blob| {
            if blob.area < 12 {
                return false;
            }
            let width = (blob.max_x - blob.min_x + 1) as f64;
            let height = (blob.max_y - blob.min_y + 1) as f64;
            (0.5..=2.0).contains(&(width / height)) && blob.area as f64 / blob.size() >= 0.35
        })
        .collect::<Vec<_>>();
    blobs.sort_by_key(|blob| std::cmp::Reverse(blob.area));
    blobs.truncate(350);
    let mut stripes = Vec::new();
    for (i, first) in blobs.iter().enumerate() {
        for (j, second) in blobs.iter().enumerate() {
            if i == j || !(0.5..=2.0).contains(&(first.area as f64 / second.area as f64)) {
                continue;
            }
            let (a, b) = (first.center(), second.center());
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let distance = dx.hypot(dy);
            let side = ((first.size().sqrt() + second.size().sqrt()) * 0.5).max(1.);
            if !(1.0..=3.3).contains(&(distance / side)) {
                continue;
            }
            let near = |target: geometry::Point| {
                blobs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, blob)| {
                        if index == i
                            || index == j
                            || !(0.5..=2.0).contains(&(first.area as f64 / blob.area as f64))
                        {
                            return None;
                        }
                        let center = blob.center();
                        let error = (center.x - target.x).hypot(center.y - target.y);
                        (error < distance * 0.27).then_some((index, error))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            };
            let Some((third_index, error3)) = near(geometry::Point {
                x: b.x + dx,
                y: b.y + dy,
            }) else {
                continue;
            };
            let axis = geometry::Point {
                x: dx / distance,
                y: dy / distance,
            };
            // Perspective may change each label's height gradually. Reject
            // abrupt jumps, rather than requiring three identical heights.
            let heights = [
                first.height_along(axis),
                second.height_along(axis),
                blobs[third_index].height_along(axis),
            ];
            let smallest = heights.iter().copied().fold(f64::INFINITY, f64::min);
            let largest = heights.iter().copied().fold(0., f64::max);
            let height_growth_1 = (heights[1] / heights[0]).ln();
            let height_growth_2 = (heights[2] / heights[1]).ln();
            if smallest < 3.
                || largest / smallest > 1.8
                || (height_growth_1 - height_growth_2).abs() > 0.3 + 1. / smallest
            {
                continue;
            }
            let bottom = blobs[third_index].center();
            let next_distance = (bottom.x - b.x).hypot(bottom.y - b.y);
            let first_step = (distance / next_distance).clamp(0.8, 1.25);
            let top = geometry::Point {
                x: a.x - dx * first_step,
                y: a.y - dy * first_step,
            };
            let cross = geometry::Point {
                x: -dy / distance,
                y: dx / distance,
            };
            // Sample across each label so a white numeral in its middle does
            // not hide the dark fill. Relative contrast rejects wood texture
            // with three similarly placed dark flecks but no gray/black rhythm.
            let tone = |center: geometry::Point, dark: bool| {
                let mut values = [-0.3, -0.15, 0., 0.15, 0.3]
                    .into_iter()
                    .filter_map(|shift| {
                        sample(
                            image,
                            geometry::Point {
                                x: center.x + cross.x * side * shift,
                                y: center.y + cross.y * side * shift,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                if values.len() < 3 {
                    return None;
                }
                values.sort_by(f64::total_cmp);
                Some(
                    values[if dark {
                        values.len() / 4
                    } else {
                        values.len() / 2
                    }],
                )
            };
            let dark_centers = [top, a, b, bottom];
            let Some(dark_tones) = dark_centers
                .map(|center| tone(center, true))
                .into_iter()
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            if dark_tones[0] > 110. {
                continue;
            }
            let mut weakest_contrast = f64::INFINITY;
            for step in 0..3 {
                let before = dark_centers[step];
                let after = dark_centers[step + 1];
                let between = |fraction: f64| geometry::Point {
                    x: before.x + (after.x - before.x) * fraction,
                    y: before.y + (after.y - before.y) * fraction,
                };
                let center = geometry::Point {
                    x: (before.x + after.x) * 0.5,
                    y: (before.y + after.y) * 0.5,
                };
                let Some(gray_tone) = tone(center, false) else {
                    weakest_contrast = 0.;
                    break;
                };
                let dark_ceiling = dark_tones[step].max(dark_tones[step + 1]);
                weakest_contrast = weakest_contrast.min(gray_tone - dark_ceiling);
                // The gray band has to persist across the interval, with dark
                // fill on both sides. Isolated flecks in wood fail this rhythm.
                for fraction in [0.15, 0.35, 0.65, 0.85] {
                    let should_be_dark = fraction < 0.2 || fraction > 0.8;
                    let Some(value) = tone(between(fraction), should_be_dark) else {
                        weakest_contrast = 0.;
                        break;
                    };
                    let contrast = if should_be_dark {
                        gray_tone - value
                    } else {
                        value - dark_ceiling
                    };
                    weakest_contrast = weakest_contrast.min(contrast);
                }
            }
            if weakest_contrast < 20. {
                continue;
            }
            let score = first.area.min(second.area).min(blobs[third_index].area) as f64
                * (weakest_contrast / 100.).clamp(0.5, 2.)
                / (largest / smallest)
                - error3 * side;
            stripes.push(Stripe {
                top,
                first: a,
                middle: b,
                bottom,
                score,
            });
        }
    }
    stripes.sort_by(|a, b| b.score.total_cmp(&a.score));
    stripes.dedup_by(|a, b| {
        (a.top.x - b.top.x).hypot(a.top.y - b.top.y) < 8.
            && (a.bottom.x - b.bottom.x).hypot(a.bottom.y - b.bottom.y) < 8.
    });
    stripes.truncate(8);
    stripes
}

fn rotate_to_axis(image: &GrayImage, theta: f32) -> GrayImage {
    if theta.abs() < 0.005 {
        return image.clone();
    }
    rotate_about_center_no_crop(
        image,
        theta,
        Interpolation::Bilinear,
        Border::Constant(Luma([255])),
    )
}

fn rotate_point(
    point: geometry::Point,
    theta: f32,
    source: (u32, u32),
    destination: (u32, u32),
) -> geometry::Point {
    let (sine, cosine) = (theta as f64).sin_cos();
    let x = point.x - source.0 as f64 * 0.5;
    let y = point.y - source.1 as f64 * 0.5;
    geometry::Point {
        x: cosine * x - sine * y + destination.0 as f64 * 0.5,
        y: sine * x + cosine * y + destination.1 as f64 * 0.5,
    }
}

fn unrotate_point(
    point: geometry::Point,
    theta: f32,
    source: (u32, u32),
    destination: (u32, u32),
) -> geometry::Point {
    rotate_point(point, -theta, destination, source)
}

fn grayscale(image: &RgbImage) -> GrayImage {
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        Luma([
            ((19595 * pixel[0] as u32 + 38470 * pixel[1] as u32 + 7471 * pixel[2] as u32 + 32768)
                >> 16) as u8,
        ])
    })
}

fn refine_grid_cells(image: &GrayImage, body: &[geometry::Point; 4]) -> Vec<[geometry::Point; 4]> {
    let rows = label_row_offsets(image, body);
    let points: Vec<_> = (0..=ROWS)
        .map(|row| {
            (0..=COLS)
                .map(|column| {
                    geometry::project(
                        body,
                        column as f64 / COLS as f64,
                        row as f64 / ROWS as f64 + rows[row],
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let mut cells = Vec::with_capacity(ROWS * COLS);
    for row in 0..ROWS {
        for column in 0..COLS {
            cells.push([
                points[row][column],
                points[row][column + 1],
                points[row + 1][column + 1],
                points[row + 1][column],
            ]);
        }
    }
    cells
}

fn label_row_offsets(image: &GrayImage, body: &[geometry::Point; 4]) -> [f64; ROWS + 1] {
    let mut offsets = [0.; ROWS + 1];
    for (line, offset) in offsets.iter_mut().enumerate().take(ROWS).skip(1) {
        let expected_direction = if line % 2 == 1 { 1. } else { -1. };
        let nominal = line as f64 / ROWS as f64;
        let delta = 0.14 / ROWS as f64;
        let mut best_score = f64::NEG_INFINITY;
        let mut best_offset = 0.;
        for step in -24..=24 {
            let candidate_offset = step as f64 * 0.01 / ROWS as f64;
            let seam = nominal + candidate_offset;
            let before = label_strip_tone(image, body, seam - delta);
            let after = label_strip_tone(image, body, seam + delta);
            let (Some(before), Some(after)) = (before, after) else {
                continue;
            };
            let score = expected_direction * (after - before);
            if score > best_score {
                best_score = score;
                best_offset = candidate_offset;
            }
        }
        // Keep the body geometry seam if the label strip has no clear transition.
        if best_score >= 8. {
            *offset = best_offset;
        }
    }
    offsets
}

fn label_strip_tone(image: &GrayImage, body: &[geometry::Point; 4], v: f64) -> Option<f64> {
    let values = [0.08, 0.18, 0.28, 0.72, 0.82, 0.92]
        .into_iter()
        .filter_map(|fraction| sample(image, geometry::project(body, -fraction / COLS as f64, v)))
        .collect::<Vec<_>>();
    (values.len() >= 4).then(|| median(values))
}

fn sample(image: &GrayImage, point: geometry::Point) -> Option<f64> {
    if !point.x.is_finite()
        || !point.y.is_finite()
        || point.x < 0.
        || point.y < 0.
        || point.x >= image.width() as f64
        || point.y >= image.height() as f64
    {
        return None;
    }
    let x = point.x.round().clamp(0., (image.width() - 1) as f64) as u32;
    let y = point.y.round().clamp(0., (image.height() - 1) as f64) as u32;
    Some(image.get_pixel(x, y)[0] as f64)
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
