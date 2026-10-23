//! Template-specific color edge measurements after coarse localization.
//! A generic threshold/contour library cannot express the alternating 7-row
//! card layout. Sample narrow profiles in its projective coordinates instead.
use super::geometry::{self, Point};
use image::RgbImage;

fn quantile(mut values: Vec<f64>, numerator: usize, denominator: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    values
        .get((values.len() - 1) * numerator / denominator)
        .copied()
}
fn color(image: &RgbImage, body: &[Point; 4], weights: [i16; 3], u: f64, v: f64) -> Option<f64> {
    let p = geometry::project(body, u, v);
    if !p.x.is_finite()
        || !p.y.is_finite()
        || p.x < 0.
        || p.y < 0.
        || p.x >= image.width() as f64 - 1.
        || p.y >= image.height() as f64 - 1.
    {
        return None;
    }
    let x = p.x.floor() as u32;
    let y = p.y.floor() as u32;
    let dx = p.x - x as f64;
    let dy = p.y - y as f64;
    let value = |x, y| {
        let p = image.get_pixel(x, y);
        p.0.iter()
            .zip(weights)
            .map(|(&value, weight)| value as f64 * weight as f64)
            .sum::<f64>()
            / weights
                .iter()
                .filter(|&&v| v > 0)
                .map(|&v| v as f64)
                .sum::<f64>()
    };
    Some(
        (value(x, y) * (1. - dx) + value(x + 1, y) * dx) * (1. - dy)
            + (value(x, y + 1) * (1. - dx) + value(x + 1, y + 1) * dx) * dy,
    )
}
fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn edge(
    center: f64,
    radius: f64,
    sign: f64,
    contrast: f64,
    profile: impl Fn(f64) -> Option<f64>,
) -> Option<f64> {
    // One source-pixel step and a 4 px derivative resist JPEG/defocus noise.
    let candidates: Vec<_> = (-(radius.ceil() as i32)..=radius.ceil() as i32)
        .filter_map(|i| {
            let x = center + i as f64;
            Some((x, sign * (profile(x + 2.)? - profile(x - 2.)?)))
        })
        .collect();
    let best = candidates.iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    if best.1 < (contrast * 0.22).max(2.) {
        return None;
    }
    let strong: Vec<_> = candidates
        .iter()
        .filter(|p| (p.0 - best.0).abs() <= 2. && p.1 >= best.1 * 0.65)
        .collect();
    let weight: f64 = strong.iter().map(|p| p.1).sum();
    Some(strong.iter().map(|p| p.0 * p.1).sum::<f64>() / weight)
}
fn line(points: &[Point], length: f64, tolerance: f64) -> Option<(f64, f64)> {
    if points.len() < 3 {
        return None;
    }
    let fit = geometry::fit(points);
    let offset = fit(0.);
    let slope = (fit(length) - offset) / length;
    let residual = quantile(
        points.iter().map(|p| (p.y - fit(p.x)).abs()).collect(),
        1,
        2,
    )?;
    (residual < tolerance).then_some((slope, offset))
}
fn intersection(horizontal: (f64, f64), vertical: (f64, f64)) -> Option<Point> {
    let denominator = 1. - horizontal.0 * vertical.0;
    if denominator.abs() < 0.1 {
        return None;
    }
    let x = (vertical.0 * horizontal.1 + vertical.1) / denominator;
    Some(Point {
        x,
        y: horizontal.0 * x + horizontal.1,
    })
}

pub(super) fn refine(image: &RgbImage, body: [Point; 4], weights: [i16; 3]) -> Option<[Point; 4]> {
    let width = (distance(body[0], body[1]) + distance(body[3], body[2])) * 0.5;
    let height = (distance(body[0], body[3]) + distance(body[1], body[2])) * 0.5;
    let pitch = height / 7.;
    if width < 60. || pitch < 6. {
        return None;
    }
    let rows: Vec<_> = (0..7)
        .map(|row| {
            quantile(
                (0..10)
                    .flat_map(|column| {
                        [0.15, 0.3, 0.7, 0.85].into_iter().filter_map(move |dx| {
                            color(
                                image,
                                &body,
                                weights,
                                (column as f64 + dx) / 10.,
                                (row as f64 + 0.5) / 7.,
                            )
                        })
                    })
                    .collect(),
                3,
                4,
            )
        })
        .collect::<Option<_>>()?;
    let pink = quantile([0, 2, 4, 6].map(|r| rows[r]).to_vec(), 1, 2)?;
    let paper = quantile([1, 3, 5].map(|r| rows[r]).to_vec(), 1, 2)?;
    let contrast = pink - paper;
    if contrast < 4.
        || [0, 2, 4, 6]
            .iter()
            .filter(|&&r| rows[r] > paper + contrast * 0.5)
            .count()
            < 3
        || [1, 3, 5]
            .iter()
            .filter(|&&r| rows[r] < pink - contrast * 0.5)
            .count()
            < 2
    {
        return None;
    }
    // All eight alternating seams constrain the row pitch. Fitting just the
    // outside pink edge can shift the first row enough to clip a Q descender.
    let columns: Vec<_> = (0..10)
        .filter_map(|column| {
            let seams: Vec<_> = (0..=7)
                .filter_map(|row| {
                    let center = row as f64 * pitch;
                    let y = edge(
                        center,
                        (pitch * 0.32).min(24.),
                        if row % 2 == 0 { 1. } else { -1. },
                        contrast,
                        |y| {
                            quantile(
                                [0.1, 0.2, 0.3, 0.7, 0.8, 0.9]
                                    .into_iter()
                                    .filter_map(|dx| {
                                        color(
                                            image,
                                            &body,
                                            weights,
                                            (column as f64 + dx) / 10.,
                                            y / height,
                                        )
                                    })
                                    .collect(),
                                3,
                                4,
                            )
                        },
                    )?;
                    Some(Point { x: center, y })
                })
                .collect();
            if seams.len() < 6 {
                return None;
            }
            let (slope, offset) = line(&seams, height, (pitch * 0.06).max(1.5))?;
            Some((
                Point {
                    x: (column as f64 + 0.5) * width / 10.,
                    y: offset,
                },
                Point {
                    x: (column as f64 + 0.5) * width / 10.,
                    y: slope * height + offset,
                },
            ))
        })
        .collect();
    if columns.len() < 7 {
        return None;
    }
    let top = line(
        &columns.iter().map(|p| p.0).collect::<Vec<_>>(),
        width,
        (pitch * 0.06).max(1.5),
    )?;
    let bottom = line(
        &columns.iter().map(|p| p.1).collect::<Vec<_>>(),
        width,
        (pitch * 0.06).max(1.5),
    )?;
    let vertical = |right: bool| {
        let center = if right { width } else { 0. };
        let points: Vec<_> = [0, 2, 4, 6]
            .into_iter()
            .filter_map(|row| {
                let x = edge(
                    center,
                    (pitch * 0.32).min(24.),
                    if right { -1. } else { 1. },
                    contrast,
                    |x| {
                        quantile(
                            [0.25, 0.4, 0.6, 0.75]
                                .into_iter()
                                .filter_map(|dy| {
                                    color(image, &body, weights, x / width, (row as f64 + dy) / 7.)
                                })
                                .collect(),
                            3,
                            4,
                        )
                    },
                )?;
                Some(Point {
                    x: (row as f64 + 0.5) * height / 7.,
                    y: x,
                })
            })
            .collect();
        line(&points, height, (pitch * 0.06).max(1.5))
    };

    let left = vertical(false)?;
    let right = vertical(true)?;
    let corners = [
        intersection(top, left)?,
        intersection(top, right)?,
        intersection(bottom, right)?,
        intersection(bottom, left)?,
    ];
    let expected = [
        Point { x: 0., y: 0. },
        Point { x: width, y: 0. },
        Point {
            x: width,
            y: height,
        },
        Point { x: 0., y: height },
    ];
    if corners
        .iter()
        .zip(expected)
        .any(|(&a, b)| distance(a, b) > pitch * 0.45)
    {
        return None;
    }
    Some(corners.map(|p| geometry::project(&body, p.x / width, p.y / height)))
}
