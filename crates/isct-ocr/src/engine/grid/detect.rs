//! Detection from scratch: seed homographies from dark printed bands, then
//! grow a joint fit from the seed's own band. Mirrors `header_candidates`,
//! `hypotheses` and `detect` of the reference. imageproc supplies the
//! thresholds, contours, hole filling and polygon simplification.
use super::fit::{fit, prior, FitInfo, Grow, HEADER, LABELS};
use super::{from_quad, Planes, COLS, ROWS};
use crate::engine::vendor::opencv_homography::Homography;
use image::{GrayImage, Luma};
use imageproc::{
    contours::{find_contours, BorderType},
    geometry::{approximate_polygon_dp, arc_length},
    point::Point,
    region_labelling::{connected_components, Connectivity},
};

struct Band {
    grow: Grow,
    p: [f64; 2],
    q: [f64; 2],
    normal: [f64; 2],
    depth: f64,
}

/// Fill holes (white letters) inside dark regions: background components
/// that do not reach the image border.
fn fill_holes(mask: &GrayImage) -> GrayImage {
    let inverse = GrayImage::from_fn(mask.width(), mask.height(), |x, y| {
        Luma([if mask.get_pixel(x, y)[0] == 0 { 255 } else { 0 }])
    });
    let labels = connected_components(&inverse, Connectivity::Four, Luma([0]));
    let count = labels.as_raw().iter().copied().max().unwrap_or(0) as usize + 1;
    let mut outside = vec![false; count];
    let (w, h) = (mask.width(), mask.height());
    for (x, y, label) in labels.enumerate_pixels() {
        if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
            outside[label[0] as usize] = true;
        }
    }
    GrayImage::from_fn(w, h, |x, y| {
        let label = labels.get_pixel(x, y)[0] as usize;
        Luma([
            if mask.get_pixel(x, y)[0] != 0 || (label != 0 && !outside[label]) {
                255
            } else {
                0
            },
        ])
    })
}

/// Closed polygon simplification. imageproc's closed mode drops the last
/// vertex without its closing side, so simplify the ring as two open arcs
/// between two mutually distant points (as OpenCV's closed approxPolyDP
/// does), then merge neighbouring sides that are nearly collinear so a long
/// printed edge is one side wherever the ring was split.
fn polygon(points: &[Point<i32>]) -> Vec<[f64; 2]> {
    if points.len() < 4 {
        return Vec::new();
    }
    let farthest = |from: usize| {
        (0..points.len())
            .max_by_key(|&i| {
                (points[i].x - points[from].x).pow(2) + (points[i].y - points[from].y).pow(2)
            })
            .unwrap_or(from)
    };
    let a = farthest(0);
    let b = farthest(a);
    let (a, b) = (a.min(b), a.max(b));
    if a == b {
        return Vec::new();
    }
    let epsilon = arc_length(points, true) * 0.01;
    let mut ring = approximate_polygon_dp(&points[a..=b], epsilon, false);
    let mut tail = points[b..].to_vec();
    tail.extend_from_slice(&points[..=a]);
    let tail = approximate_polygon_dp(&tail, epsilon, false);
    ring.pop();
    ring.extend_from_slice(&tail[..tail.len().saturating_sub(1)]);
    let mut poly: Vec<[f64; 2]> = ring.iter().map(|p| [p.x as f64, p.y as f64]).collect();
    loop {
        let n = poly.len();
        if n < 4 {
            break;
        }
        let straight = (0..n).find(|&i| {
            let (p, q, r) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let (u, v) = ([q[0] - p[0], q[1] - p[1]], [r[0] - q[0], r[1] - q[1]]);
            let cos = (u[0] * v[0] + u[1] * v[1]) / (u[0].hypot(u[1]) * v[0].hypot(v[1])).max(1e-9);
            cos > 8f64.to_radians().cos()
        });
        match straight {
            Some(i) => {
                poly.remove(i);
            }
            None => break,
        }
    }
    poly
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Straight outer edges of dark, letter-filled bands and the dark depth
/// behind them. The header may merge with the label column at its corner,
/// so a band is one long polygon edge plus its depth, not a whole component.
fn bands(planes: &Planes) -> Vec<Band> {
    let gray = &planes.luma;
    let (w, h) = (gray.width(), gray.height());
    let mut masks = Vec::new();
    // Dark relative to the neighbourhood: dim rooms put the paper itself
    // below any fixed level, while the band stays darker than its margin.
    for (block, offset) in [((w / 12).max(31) | 1, 20), ((w / 6).max(31) | 1, 25)] {
        let bright = imageproc::contrast::adaptive_threshold(gray, block / 2, offset);
        masks.push(GrayImage::from_fn(w, h, |x, y| {
            Luma([if bright.get_pixel(x, y)[0] == 0 {
                255
            } else {
                0
            }])
        }));
    }
    for level in [60, 100] {
        masks.push(GrayImage::from_fn(w, h, |x, y| {
            Luma([if gray.get_pixel(x, y)[0] < level {
                255
            } else {
                0
            }])
        }));
    }
    let mut out = Vec::new();
    for mask in masks {
        let filled = fill_holes(&mask);
        for contour in find_contours::<i32>(&mask) {
            if contour.border_type != BorderType::Outer || contour.parent.is_some() {
                continue;
            }
            let area = shoelace(&contour.points);
            if area < (w as f64 * 0.01).powi(2) {
                continue;
            }
            let poly = polygon(&contour.points);
            let n = poly.len();
            // Sides, plus chords over one or two vertices that stay within 4 %
            // of the chord: a letter or the label corner can notch a side.
            let mut sides = Vec::new();
            for k in 0..n {
                for span in 1..=3.min(n.saturating_sub(1)) {
                    let (p, q) = (poly[k], poly[(k + span) % n]);
                    let chord = (q[0] - p[0]).hypot(q[1] - p[1]);
                    let straight = (1..span).all(|m| {
                        let r = poly[(k + m) % n];
                        ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])).abs()
                            / chord.max(1e-9)
                            < 0.04 * chord
                    });
                    if straight {
                        sides.push((p, q));
                    }
                }
            }
            for (p, q) in sides {
                let length = (q[0] - p[0]).hypot(q[1] - p[1]);
                if length < w as f64 * 0.08 {
                    continue;
                }
                let d = [(q[0] - p[0]) / length, (q[1] - p[1]) / length];
                for normal in [[-d[1], d[0]], [d[1], -d[0]]] {
                    let depths: Vec<f64> = (0..15)
                        .map(|i| {
                            let f = 0.15 + 0.05 * i as f64;
                            let start = [
                                p[0] + f * (q[0] - p[0]) + normal[0] * 1.5,
                                p[1] + f * (q[1] - p[1]) + normal[1] * 1.5,
                            ];
                            let mut run = 0.;
                            while run < length / 4. {
                                let x = (start[0] + normal[0] * run).round();
                                let y = (start[1] + normal[1] * run).round();
                                if x < 0.
                                    || y < 0.
                                    || x >= w as f64
                                    || y >= h as f64
                                    || filled.get_pixel(x as u32, y as u32)[0] == 0
                                {
                                    break;
                                }
                                run += 1.;
                            }
                            run
                        })
                        .collect();
                    let mean = depths.iter().sum::<f64>() / depths.len() as f64;
                    let spread = (depths.iter().map(|d| (d - mean).powi(2)).sum::<f64>()
                        / depths.len() as f64)
                        .sqrt();
                    let depth = median(depths) + 1.5;
                    if depth < 4. || spread > 0.25 * depth {
                        continue;
                    }
                    let grow = match length / depth {
                        r if (7. ..=24.).contains(&r) => Grow::Header,
                        r if (3.5..=9.).contains(&r) => Grow::Labels,
                        _ => continue,
                    };
                    out.push(Band {
                        grow,
                        p,
                        q,
                        normal,
                        depth,
                    });
                }
            }
        }
    }
    out
}

fn shoelace(points: &[Point<i32>]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.x as f64 * b.y as f64 - b.x as f64 * a.y as f64
        })
        .sum::<f64>()
        .abs()
        / 2.
}

/// Four readings of a band: its found edge is the outer or the inner edge,
/// with the first column (or row) at either end.
fn hypotheses(band: &Band) -> Vec<Homography> {
    let far = |p: [f64; 2]| {
        [
            p[0] + band.normal[0] * band.depth,
            p[1] + band.normal[1] * band.depth,
        ]
    };
    let near = (band.p, band.q);
    let deep = (far(band.p), far(band.q));
    let model = match band.grow {
        Grow::Header => [[0., -HEADER], [COLS, -HEADER], [COLS, 0.], [0., 0.]],
        Grow::Labels => [[-LABELS, 0.], [-LABELS, ROWS], [0., ROWS], [0., 0.]],
    };
    let mut out = Vec::new();
    for (outer, inner) in [(near, deep), (deep, near)] {
        for flip in [false, true] {
            let (a, b) = if flip { (outer.1, outer.0) } else { outer };
            let (c, d) = if flip { (inner.1, inner.0) } else { inner };
            if let Some(h) = from_quad(model, [a, b, d, c]) {
                out.push(h);
            }
        }
    }
    out
}

/// The seed and fitted grid with the strongest anchors, if any passes.
pub(crate) fn detect(planes: &Planes) -> Option<(Homography, Homography, FitInfo)> {
    let mut best: Option<(Homography, Homography, FitInfo)> = None;
    for band in bands(planes) {
        for seed in hypotheses(&band) {
            if prior(planes, &seed) < 0.2 {
                continue;
            }
            if let (Some(h), info) = fit(planes, seed, &[0.4, 0.3, 0.2], Some(band.grow)) {
                if best.as_ref().is_none_or(|b| info.anchor > b.2.anchor) {
                    best = Some((seed, h, info));
                }
            }
        }
    }
    best
}
