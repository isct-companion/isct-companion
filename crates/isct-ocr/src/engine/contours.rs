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
pub(super) fn valid(image: &RgbImage, corners: &[Point; 4]) -> bool {
    if corners.iter().any(|p| {
        !p.x.is_finite()
            || !p.y.is_finite()
            || p.x < 0.
            || p.y < 0.
            || p.x >= image.width() as f64
            || p.y >= image.height() as f64
    }) {
        return false;
    }
    for i in 0..4 {
        let (a, b, c) = (corners[i], corners[(i + 1) % 4], corners[(i + 2) % 4]);
        if (b.x - a.x).hypot(b.y - a.y) < if i % 2 == 0 { 44. } else { 32. }
            || (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) <= 0.
        {
            return false;
        }
    }
    let sample = |u, v| gray(image, project(corners, u, v));
    let dark_header = (0..100)
        .filter(|&i| {
            let u = (1.2 + i as f64 * 0.096) / 11.;
            sample(u, 0.2 / 8.) + 20.
                < [1.2, 1.5, 1.8]
                    .into_iter()
                    .map(|v| sample(u, v / 8.))
                    .fold(0., f64::max)
        })
        .count();
    let dark_labels = (0..70)
        .filter(|&i| {
            let v = (1.2 + i as f64 * 0.094) / 8.;
            sample(0.2 / 11., v) + 20.
                < [1.2, 1.5, 1.8]
                    .into_iter()
                    .map(|u| sample(u / 11., v))
                    .fold(0., f64::max)
        })
        .count();
    if dark_header < 60 || dark_labels < 40 {
        return false;
    }
    // Each internal boundary must be darker than both neighboring cell interiors.
    // Search one preview pixel on either side to tolerate thin antialiased lines.
    let mut supported = 0;
    for vertical in [true, false] {
        let count = if vertical { 10 } else { 7 };
        for line in 2..=count {
            let mut hits = 0;
            for i in 0..40 {
                let cross = if vertical {
                    (1.3 + i as f64 * 0.16) / 8.
                } else {
                    (1.3 + i as f64 * 0.235) / 11.
                };
                let axis = line as f64 / if vertical { 11. } else { 8. };
                let at = |offset| {
                    if vertical {
                        project(corners, axis + offset, cross)
                    } else {
                        project(corners, cross, axis + offset)
                    }
                };
                let a = at(0.);
                let b = at(0.22 / if vertical { 11. } else { 8. });
                let c = at(-0.22 / if vertical { 11. } else { 8. });
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
    }
    supported >= 12
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
                if !valid(image, &corners) {
                    continue;
                }
                return Some(
                    (0..70)
                        .map(|i| {
                            let (c, r) = ((i % 10 + 1) as f64, (i / 10 + 1) as f64);
                            [
                                (c / 11., r / 8.),
                                ((c + 1.) / 11., r / 8.),
                                ((c + 1.) / 11., (r + 1.) / 8.),
                                (c / 11., (r + 1.) / 8.),
                            ]
                            .map(|(u, v)| project(&corners, u, v))
                        })
                        .collect(),
                );
            }
        }
    }
    None
}
