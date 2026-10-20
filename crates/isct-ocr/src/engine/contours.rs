//! Projective grid fallback. imageproc owns contour tracing and polygon fitting;
//! the card-specific checks below distinguish a matrix from text and barcodes.
use super::geometry::{project, Point};
use image::{GrayImage, Luma, RgbImage};
use imageproc::{
    contours::find_contours,
    geometry::{approximate_polygon_dp, arc_length, convex_hull},
};

fn intersect(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let (ux, uy, vx, vy) = (b.x - a.x, b.y - a.y, d.x - c.x, d.y - c.y);
    let det = ux * vy - uy * vx;
    if det.abs() < 1e-6 {
        return None;
    }
    let t = ((c.x - a.x) * vy - (c.y - a.y) * vx) / det;
    Some(Point {
        x: a.x + t * ux,
        y: a.y + t * uy,
    })
}
fn quad(points: &[imageproc::point::Point<i32>]) -> Option<[Point; 4]> {
    let hull = convex_hull(points);
    if hull.len() < 4 {
        return None;
    }
    // Simplify the closed ring as two open arcs: imageproc's closed option
    // removes the last vertex without joining the closing side.
    let split = (1..hull.len())
        .max_by_key(|&i| (hull[i].x - hull[0].x).pow(2) + (hull[i].y - hull[0].y).pow(2))?;
    let epsilon = arc_length(&hull, true) * 0.015;
    let mut polygon = approximate_polygon_dp(&hull[..=split], epsilon, false);
    let mut tail = hull[split..].to_vec();
    tail.push(hull[0]);
    let tail = approximate_polygon_dp(&tail, epsilon, false);
    polygon.pop();
    polygon.extend_from_slice(&tail[..tail.len() - 1]);
    if !(4..=8).contains(&polygon.len()) {
        return None;
    }
    // A matrix's blank upper-left label makes its outline a pentagon. Extend
    // the four main sides through that notch instead of treating it as a cell.
    let mut sides: Vec<_> = (0..polygon.len()).collect();
    sides.sort_by_key(|&i| {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        std::cmp::Reverse((a.x - b.x).pow(2) + (a.y - b.y).pow(2))
    });
    sides.truncate(4);
    sides.sort_unstable();
    let side = |i: usize| {
        let a = polygon[sides[i]];
        let b = polygon[(sides[i] + 1) % polygon.len()];
        (
            Point {
                x: a.x as f64,
                y: a.y as f64,
            },
            Point {
                x: b.x as f64,
                y: b.y as f64,
            },
        )
    };
    let mut result = [Point { x: 0., y: 0. }; 4];
    for (i, p) in result.iter_mut().enumerate() {
        let (a, b) = side(i);
        let (c, d) = side((i + 1) % 4);
        *p = intersect(a, b, c, d)?;
    }
    let cross = (result[1].x - result[0].x) * (result[2].y - result[1].y)
        - (result[1].y - result[0].y) * (result[2].x - result[1].x);
    if cross < 0. {
        result.reverse();
    }
    Some(result)
}
fn gray(image: &RgbImage, p: Point) -> f64 {
    if !p.x.is_finite()
        || !p.y.is_finite()
        || p.x < 0.
        || p.y < 0.
        || p.x >= image.width() as f64
        || p.y >= image.height() as f64
    {
        return 255.;
    }
    let p = image.get_pixel(
        p.x.round().min((image.width() - 1) as f64) as u32,
        p.y.round().min((image.height() - 1) as f64) as u32,
    );
    p[0] as f64 * 0.299 + p[1] as f64 * 0.587 + p[2] as f64 * 0.114
}
/// The label bands are not necessarily one body cell wide/high. Locate their
/// paper transition before splitting the body; equal eighths clip real glyphs.
fn axis_profiles(image: &RgbImage, corners: &[Point; 4], vertical: bool) -> (Vec<f64>, Vec<f64>) {
    let edge = if vertical { 1 } else { 3 };
    let length = (corners[edge].x - corners[0].x).hypot(corners[edge].y - corners[0].y);
    let steps = length.ceil() as usize;
    (0..=steps)
        .map(|i| {
            let axis = i as f64 / steps as f64;
            let mut samples: Vec<_> = (0..40)
                .map(|j| {
                    let cross = 0.25 + j as f64 * 0.017;
                    let (u, v) = if vertical {
                        (axis, cross)
                    } else {
                        (cross, axis)
                    };
                    gray(image, project(corners, u, v))
                })
                .collect();
            samples.sort_by(f64::total_cmp);
            // A glyph stem may cover half a column, but a grid line spans
            // nearly all of it. Keep the median for the solid label band.
            (samples[samples.len() / 2], samples[samples.len() * 4 / 5])
        })
        .unzip()
}

fn label_boundary(profile: &[f64]) -> Option<f64> {
    let steps = profile.len().checked_sub(1)?;
    let start = ((steps as f64 * 0.035).ceil() as usize).max(2);
    let end = (steps as f64 * 0.24).ceil() as usize;
    let peak = (start..end).max_by(|&a, &b| {
        (profile[a + 2] - profile[a - 2]).total_cmp(&(profile[b + 2] - profile[b - 2]))
    })?;
    let rise = profile[peak + 2] - profile[peak - 2];
    if rise < 15. {
        return None;
    }
    // A later internal grid line can be darker than the label band. Choose the
    // first comparable paper transition, not the strongest later row/column.
    let best = (start..end).find(|&i| profile[i + 2] - profile[i - 2] >= rise * 0.8)?;
    let (mut position, mut weight) = (0., 0.);
    for i in best - 2..best + 2 {
        let rise = (profile[i + 1] - profile[i]).max(0.);
        position += (i as f64 + 0.5) * rise;
        weight += rise;
    }
    (weight > 0.).then_some(position / weight / steps as f64)
}

fn body_axis(profile: &[f64], start: f64, count: usize) -> (f64, f64) {
    let steps = (profile.len() - 1) as f64;
    let pitch = (1. - start) / count as f64;
    let mut lines = vec![];
    for line in 1..count {
        let center = start + line as f64 * pitch;
        let lo = ((center - pitch * 0.18) * steps).ceil() as usize;
        let hi = ((center + pitch * 0.18) * steps).floor() as usize;
        let Some(peak) = (lo..=hi).min_by(|&a, &b| profile[a].total_cmp(&profile[b])) else {
            continue;
        };
        let background = profile[lo].max(profile[hi]);
        if background - profile[peak] < 8. {
            continue;
        }
        let threshold = (background + profile[peak]) / 2.;
        let (mut a, mut b) = (peak, peak);
        while a > lo && profile[a - 1] < threshold {
            a -= 1;
        }
        while b < hi && profile[b + 1] < threshold {
            b += 1;
        }
        let (mut position, mut weight) = (0., 0.);
        for (i, &value) in profile.iter().enumerate().take(b + 1).skip(a) {
            let ink = background - value;
            position += i as f64 * ink;
            weight += ink;
        }
        lines.push(Point {
            x: line as f64 * 100.,
            y: position / weight / steps,
        });
    }
    if lines.len() < count - 2 {
        return (start, 1.);
    }
    // Reuse the robust line fit used by the header detector. Internal line
    // centers remove the half-border bias of a dark-label/paper transition.
    let fit = super::geometry::fit(&lines);
    let (a, b) = (fit(0.), fit(count as f64 * 100.));
    if (a - start).abs() > pitch * 0.12
        || (b - 1.).abs() > pitch * 0.12
        || lines.iter().any(|p| (fit(p.x) - p.y).abs() > pitch * 0.12)
    {
        return (start, 1.);
    }
    (a, b)
}

pub(super) fn validated_body(image: &RgbImage, corners: &[Point; 4]) -> Option<[Point; 4]> {
    if corners.iter().any(|p| {
        !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.
            || p.y < 0.
            || p.x >= image.width() as f64
            || p.y >= image.height() as f64
    }) {
        return None;
    }
    for i in 0..4 {
        let (a, b, c) = (corners[i], corners[(i + 1) % 4], corners[(i + 2) % 4]);
        if (b.x - a.x).hypot(b.y - a.y) < if i % 2 == 0 { 44. } else { 32. }
            || (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) <= 0.
        {
            return None;
        }
    }
    let sample = |u, v| gray(image, project(corners, u, v));
    let dark_header = (0..100)
        .filter(|&i| {
            let u = 0.25 + i as f64 * 0.007;
            sample(u, 0.015) + 20.
                < [0.35, 0.45, 0.55]
                    .into_iter()
                    .map(|v| sample(u, v))
                    .fold(0., f64::max)
        })
        .count();
    let dark_labels = (0..70)
        .filter(|&i| {
            let v = 0.25 + i as f64 * 0.01;
            sample(0.015, v) + 20.
                < [0.25, 0.35, 0.45]
                    .into_iter()
                    .map(|u| sample(u, v))
                    .fold(0., f64::max)
        })
        .count();
    if dark_header < 60 || dark_labels < 40 {
        return None;
    }
    let (labels_x, lines_x) = axis_profiles(image, corners, true);
    let (labels_y, lines_y) = axis_profiles(image, corners, false);
    let (left, right) = body_axis(&lines_x, label_boundary(&labels_x)?, 10);
    let (top, bottom) = body_axis(&lines_y, label_boundary(&labels_y)?, 7);
    // Each internal boundary must be darker than both neighboring cell interiors.
    // Search one preview pixel on either side to tolerate thin antialiased lines.
    for vertical in [true, false] {
        let count = if vertical { 10 } else { 7 };
        let mut supported = 0;
        for line in 1..count {
            let mut hits = 0;
            for i in 0..40 {
                let cross = if vertical {
                    top + (bottom - top) * (0.04 + i as f64 * 0.023)
                } else {
                    left + (right - left) * (0.04 + i as f64 * 0.023)
                };
                let (start, span) = if vertical {
                    (left, right - left)
                } else {
                    (top, bottom - top)
                };
                let pitch = span / count as f64;
                let axis = start + line as f64 * pitch;
                let at = |offset| {
                    if vertical {
                        project(corners, axis + offset, cross)
                    } else {
                        project(corners, cross, axis + offset)
                    }
                };
                let a = at(0.);
                let b = at(0.22 * pitch);
                let c = at(-0.22 * pitch);
                let background = gray(image, b).min(gray(image, c));
                let dx = b.x - c.x;
                let dy = b.y - c.y;
                let length = dx.hypot(dy).max(1.);
                let ink = [-1., 0., 1.]
                    .into_iter()
                    .map(|d| {
                        gray(
                            image,
                            Point {
                                x: a.x + d * dx / length,
                                y: a.y + d * dy / length,
                            },
                        )
                    })
                    .fold(255., f64::min);
                if ink + 6. < background {
                    hits += 1;
                }
            }
            if hits >= 12 {
                supported += 1;
            }
        }
        // A reflection can cut the contour in half. Vertical lines alone
        // cannot prove that all seven rows are inside the proposed body.
        if supported < count - 2 {
            return None;
        }
    }
    Some([
        project(corners, left, top),
        project(corners, right, top),
        project(corners, right, bottom),
        project(corners, left, bottom),
    ])
}

pub fn locate(image: &RgbImage) -> Option<Vec<[Point; 4]>> {
    let luminance = GrayImage::from_fn(image.width(), image.height(), |x, y| {
        Luma([gray(
            image,
            Point {
                x: x as f64,
                y: y as f64,
            },
        ) as u8])
    });
    for threshold in [125., 155., 100., 185., 0.] {
        let binary = if threshold == 0. {
            let edges = imageproc::edges::canny(&luminance, 12., 24.);
            imageproc::morphology::dilate(&edges, imageproc::distance_transform::Norm::LInf, 1)
        } else {
            GrayImage::from_fn(image.width(), image.height(), |x, y| {
                Luma([
                    if gray(
                        image,
                        Point {
                            x: x as f64,
                            y: y as f64,
                        },
                    ) < threshold
                    {
                        255
                    } else {
                        0
                    },
                ])
            })
        };
        for contour in find_contours::<i32>(&binary) {
            if contour.points.len() < 100 {
                continue;
            }
            let Some(corners) = quad(&contour.points) else {
                continue;
            };
            for turn in 0..4 {
                let corners = std::array::from_fn(|i| corners[(i + turn) % 4]);
                let Some(body) = validated_body(image, &corners) else {
                    continue;
                };
                return Some(
                    (0..70)
                        .map(|i| {
                            let (c, r) = ((i % 10) as f64, (i / 10) as f64);
                            [
                                (c / 10., r / 7.),
                                ((c + 1.) / 10., r / 7.),
                                ((c + 1.) / 10., (r + 1.) / 7.),
                                (c / 10., (r + 1.) / 7.),
                            ]
                            .map(|(u, v)| project(&body, u, v))
                        })
                        .collect(),
                );
            }
        }
    }
    None
}
