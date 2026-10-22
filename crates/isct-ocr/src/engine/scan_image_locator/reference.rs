//! Two directional rectification constraints measured from grayscale anchors.
//! Work in the coarse body's coordinates, then compose the transforms so the
//! recognition path samples the original pixels once. No cell aspect-ratio prior.
use super::{geometry, median, GrayImage};
use geometry::Point;

pub(super) struct Rectification {
    pub(super) body: [Point; 4],
}

fn sample(image: &GrayImage, point: Point) -> Option<f64> {
    image::imageops::interpolate_bilinear(image, point.x as f32, point.y as f32)
        .map(|pixel| pixel[0] as f64)
}

/// The header need not be horizontal after the strip axis is made upright.
/// Use imageproc's contour rectangle only as a seed; all final boundaries are
/// measured again from gray transitions by the two directional stages below.
pub(super) fn seed(image: &GrayImage, centers: [Point; 4]) -> Option<Rectification> {
    use imageproc::{
        contours::{find_contours_with_threshold, BorderType},
        geometry::min_area_rect,
    };
    let pitch = (centers[1].y - centers[0].y) * 0.5;
    let inner = transition(
        |x| {
            let mut values: Vec<_> = centers[1..]
                .iter()
                .flat_map(|center| {
                    [-0.3, -0.15, 0.15, 0.3].into_iter().filter_map(move |dy| {
                        sample(
                            image,
                            Point {
                                x,
                                y: center.y + dy * pitch,
                            },
                        )
                    })
                })
                .collect();
            values.sort_by(f64::total_cmp);
            (values.len() >= 8).then(|| values[values.len() / 4])
        },
        centers[1].x + pitch,
        pitch * 0.7,
        pitch * 0.09,
        1.,
    )?;
    let mask = GrayImage::from_fn(image.width(), image.height(), |x, y| {
        image::Luma([if x as f64 > inner + 2. && image.get_pixel(x, y)[0] < 90 {
            255
        } else {
            0
        }])
    });
    let mask = imageproc::morphology::open(&mask, imageproc::distance_transform::Norm::LInf, 1);
    let mut best = None;
    let mut best_span = 0.;
    for contour in find_contours_with_threshold::<i32>(&mask, 128) {
        if contour.border_type != BorderType::Outer || contour.points.len() < 12 {
            continue;
        }
        let rect = min_area_rect(&contour.points);
        let mut points = rect.map(|p| Point {
            x: p.x as f64,
            y: p.y as f64,
        });
        points.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut left = [points[0], points[1]];
        let mut right = [points[2], points[3]];
        left.sort_by(|a, b| a.y.total_cmp(&b.y));
        right.sort_by(|a, b| a.y.total_cmp(&b.y));
        let span = right[1].x - left[1].x;
        let height = ((left[1].y - left[0].y) + (right[1].y - right[0].y)) * 0.5;
        let anchor = centers[0];
        if span < pitch * 7.
            || span < best_span
            || height < pitch * 0.35
            || height > pitch * 2.2
            || (left[1].x - anchor.x).abs() > pitch * 2.2
            || (left[1].y - (anchor.y - pitch * 0.5)).abs() > pitch * 0.8
        {
            continue;
        }
        let bottom = centers[3].y + (centers[3].y - centers[2].y) * 0.25;
        let body = [
            left[1],
            right[1],
            Point {
                x: right[1].x,
                y: bottom,
            },
            Point {
                x: left[1].x,
                y: bottom,
            },
        ];
        if let Some(refined) = refine(image, &body) {
            best_span = span;
            best = Some(refined);
        }
    }
    best
}

struct Line {
    slope: f64,
    intercept: f64,
    points: Vec<Point>,
}

// Reuse the robust line estimator. Scale normalized coordinates because its
// pair-distance threshold is expressed in pixels.
fn fit(points: Vec<Point>) -> Option<Line> {
    if points.len() < 5 {
        return None;
    }
    let scaled: Vec<_> = points
        .iter()
        .map(|p| Point {
            x: p.x * 1000.,
            y: p.y * 1000.,
        })
        .collect();
    let f = geometry::fit(&scaled);
    let intercept = f(0.) / 1000.;
    let slope = (f(1000.) - f(0.)) / 1000.;
    let residual = median(
        points
            .iter()
            .map(|p| (p.y - slope * p.x - intercept).abs())
            .collect(),
    );
    (residual < 0.012).then_some(Line {
        slope,
        intercept,
        points,
    })
}

// Measure a signed, sustained gray transition; local subpixel averaging avoids
// making short label boundaries depend on a one-pixel staircase.
fn transition(
    mut tone: impl FnMut(f64) -> Option<f64>,
    center: f64,
    radius: f64,
    delta: f64,
    sign: f64,
) -> Option<f64> {
    let step = radius / 40.;
    let mut scores = Vec::with_capacity(81);
    for i in 0..=80 {
        let p = center - radius + i as f64 * step;
        let score = sign * (tone(p + delta)? - tone(p - delta)?);
        scores.push((p, score));
    }
    let &(peak, strength) = scores.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    if strength < 12. {
        return None;
    }
    let mut weight = 0.;
    let mut sum = 0.;
    for (p, s) in scores {
        if (p - peak).abs() <= delta && s > strength * 0.65 {
            weight += s;
            sum += p * s;
        }
    }
    (weight > 0.).then_some(sum / weight)
}

pub(super) fn refine(image: &GrayImage, body: &[Point; 4]) -> Option<Rectification> {
    let tone = |u: f64, v: f64| sample(image, geometry::project(body, u, v));
    let horizontal = |row: usize| -> Option<Line> {
        let mut points = Vec::new();
        for i in 0..25 {
            // Fit the complete band transition; isolated numeral edges become
            // outliers rather than leaving two small, easily biased clusters.
            let u = if row == 0 {
                0.025 + i as f64 * 0.95 / 24.
            } else {
                -0.085 + i as f64 * 0.07 / 24.
            };
            let sign = if row == 0 || row % 2 == 1 { 1. } else { -1. };
            let Some(v) = transition(|v| tone(u, v), row as f64 / 7., 0.40 / 7., 0.055 / 7., sign)
            else {
                continue;
            };
            points.push(Point { x: u, y: v });
        }
        fit(points)
    };
    let top = horizontal(0)?;
    let seams: Vec<_> = (1..=7)
        .filter_map(|r| horizontal(r).map(|line| (r, line)))
        .collect();
    if seams.len() < 5 {
        return None;
    }
    // All physical row boundaries share a horizontal vanishing point [1,a,g].
    // The long header/body line fixes its direction near the top; the alternating
    // strip fixes the change of that direction down the card.
    let estimates: Vec<_> = seams
        .iter()
        .filter(|(r, _)| *r >= 3)
        .map(|(_, line)| (top.slope - line.slope) / (line.intercept - top.intercept))
        .filter(|v| v.is_finite())
        .collect();
    if estimates.len() < 3 {
        return None;
    }
    let initial_g = median(estimates);
    // White numeral strokes can produce a plausible local edge on one side of
    // a short strip. Require the measured lines to agree on one vanishing point
    // before they participate in the final transform.
    let initial_a = top.slope + top.intercept * initial_g;
    let seams: Vec<_> = seams
        .into_iter()
        .filter(|(_, line)| (line.slope - (initial_a - initial_g * line.intercept)).abs() < 0.08)
        .collect();
    let estimates: Vec<_> = seams
        .iter()
        .filter(|(r, _)| *r >= 3)
        .map(|(_, line)| (top.slope - line.slope) / (line.intercept - top.intercept))
        .filter(|value| value.is_finite())
        .collect();
    if seams.len() < 3 || estimates.len() < 2 {
        return None;
    }
    let bottom = &seams.iter().find(|(r, _)| *r == 7)?.1;
    let g = median(estimates);
    let a = top.slope + top.intercept * g;
    if g.abs() > 0.65 {
        return None;
    }
    let horizontal_inverse = |p: Point| -> Option<Point> {
        let z = 1. - g * p.x;
        (z > 0.25).then_some(Point {
            x: p.x / z,
            y: (p.y - a * p.x) / z,
        })
    };

    // The second stage reads the two sides of 1-7 and the outside of J.
    // Its measurements are transformed through stage one before fitting.
    let vertical = |u: f64, is_j: bool, sign: f64| -> Option<Line> {
        let mut points = Vec::new();
        for i in 0..49 {
            let v = if is_j {
                -0.125 + i as f64 * 0.105 / 48.
            } else {
                0.035 + i as f64 * 0.93 / 48.
            };
            if !is_j && (v * 7.).floor() as usize % 2 == 1 {
                continue;
            }
            let Some(x) = transition(|x| tone(x, v), u, 0.045, 0.006, sign) else {
                continue;
            };
            let p = horizontal_inverse(Point { x, y: v })?;
            points.push(Point { x: p.y, y: p.x });
        }
        fit(points)
    };
    let outer = vertical(-0.1, false, -1.)?;
    let left = vertical(0., false, 1.)?;
    let right = vertical(1., true, 1.)?;
    let directions = [
        Point {
            x: outer.intercept,
            y: outer.slope,
        },
        Point {
            x: left.intercept,
            y: left.slope,
        },
        Point {
            x: right.intercept,
            y: right.slope,
        },
    ];
    let scaled: Vec<_> = directions
        .iter()
        .map(|p| Point {
            x: p.x * 1000.,
            y: p.y * 1000.,
        })
        .collect();
    let f = geometry::fit(&scaled);
    let b = f(0.) / 1000.;
    let h = -(f(1000.) - f(0.)) / 1000.;
    if h.abs() > 0.65 {
        return None;
    }
    let top_v = top.intercept;
    let bottom_v = median(
        bottom
            .points
            .iter()
            .filter_map(|&p| horizontal_inverse(p).map(|p| p.y))
            .collect(),
    );
    if 1. - h * bottom_v < 0.25 {
        return None;
    }
    let y0 = top_v / (1. - h * top_v);
    let y1 = bottom_v / (1. - h * bottom_v);
    let x0 = left.intercept;
    let x1 = right.intercept;
    if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
        return None;
    }
    let mut result = [Point { x: 0., y: 0. }; 4];
    for (index, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        .into_iter()
        .enumerate()
    {
        let z = 1. + h * y;
        if z < 0.25 {
            return None;
        }
        let u = (x + b * y) / z;
        let v = y / z;
        let z = 1. + g * u;
        if z < 0.25 {
            return None;
        }
        let u0 = u / z;
        let v0 = (v + a * u) / z;
        if !(-0.35..1.35).contains(&u0) || !(-0.35..1.35).contains(&v0) {
            return None;
        }
        result[index] = geometry::project(body, u0, v0);
    }
    Some(Rectification { body: result })
}
