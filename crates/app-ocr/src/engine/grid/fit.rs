//! Joint grid fit: measure the printed lines near a predicted grid, intersect
//! them, and fit one homography to every intersection. Mirrors `measure`,
//! `fit`, `anchors`, `column_gaps` and `geometry_reject` of the reference.
use super::{apply, body, cell_px, distance, Channel, Planes, COLS, ROWS};
use crate::engine::vendor::opencv_homography::{find_homography, Homography};

/// Card constants in cell units, calibrated on SIFT-labelled frames with the
/// reference's `--calibrate`: header band height and label column width.
pub(super) const HEADER: f64 = 1.05;
pub(super) const LABELS: f64 = 1.03;
/// Half-width (cells) of the step derivative.
const W: f64 = 0.12;

/// One edge sample: model position, search direction, channel and the sign
/// the derivative must have along that direction.
#[derive(Clone, Copy)]
struct Sample {
    u: f64,
    v: f64,
    du: f64,
    dv: f64,
    channel: Channel,
    sign: f64,
}

struct LineSpec {
    name: &'static str,
    value: f64,
    horizontal: bool,
    samples: Vec<Sample>,
}

fn line_specs() -> Vec<LineSpec> {
    let mut specs = Vec::new();
    // Row steps are sampled in the ink-free gaps between letters.
    let mut across_u = vec![0.08];
    for c in 1..COLS as usize {
        across_u.extend([c as f64 - 0.07, c as f64 + 0.07]);
    }
    across_u.push(COLS - 0.08);
    let along = |v: f64, channel, sign| -> Vec<Sample> {
        across_u
            .iter()
            .map(|&u| Sample {
                u,
                v,
                du: 0.,
                dv: 1.,
                channel,
                sign,
            })
            .collect()
    };
    specs.push(LineSpec {
        name: "header-top",
        value: -HEADER,
        horizontal: true,
        samples: along(-HEADER, Channel::Luma, -1.),
    });
    const ROW_NAMES: [&str; 8] = [
        "row-0", "row-1", "row-2", "row-3", "row-4", "row-5", "row-6", "row-7",
    ];
    for (j, name) in ROW_NAMES.iter().enumerate() {
        // Rows 1, 3, 5, 7 are pink: the step below an even line rises in R−G.
        let sign = if j % 2 == 0 { 1. } else { -1. };
        specs.push(LineSpec {
            name,
            value: j as f64,
            horizontal: true,
            samples: along(j as f64, Channel::Chroma, sign),
        });
    }
    let rows_v: Vec<f64> = (0..14).map(|k| 0.25 + 0.5 * k as f64).collect();
    let header_v: Vec<f64> = (0..3)
        .map(|k| -HEADER + 0.2 + k as f64 * (HEADER - 0.4) / 2.)
        .collect();
    let across = |u: f64, vs: &[f64], channel, sign| -> Vec<Sample> {
        vs.iter()
            .map(|&v| Sample {
                u,
                v,
                du: 1.,
                dv: 0.,
                channel,
                sign,
            })
            .collect()
    };
    specs.push(LineSpec {
        name: "labels-left",
        value: -LABELS,
        horizontal: false,
        samples: across(-LABELS, &rows_v, Channel::Luma, -1.),
    });
    let mut left = across(0., &rows_v, Channel::Luma, 1.);
    left.extend(across(0., &header_v, Channel::Luma, -1.));
    specs.push(LineSpec {
        name: "body-left",
        value: 0.,
        horizontal: false,
        samples: left,
    });
    let pink_v: Vec<f64> = (0..ROWS as usize)
        .step_by(2)
        .flat_map(|r| [0.25, 0.5, 0.75].map(|d| r as f64 + d))
        .collect();
    let mut right = across(COLS, &pink_v, Channel::Chroma, -1.);
    right.extend(across(COLS, &header_v, Channel::Luma, 1.));
    specs.push(LineSpec {
        name: "body-right",
        value: COLS,
        horizontal: false,
        samples: right,
    });
    specs
}

/// Peak position of a step response. A sharp edge gives a flat top as wide
/// as the derivative window, so take the middle of the contiguous run within
/// 5 % of the maximum; a single-sample peak gets parabolic refinement.
fn peak(response: &[f64], offsets: &[f64]) -> (f64, f64) {
    let Some((k, &value)) = response
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
    else {
        return (0., f64::NEG_INFINITY);
    };
    let near = |i: usize| response[i] >= value - 0.05 * value.abs();
    let (mut lo, mut hi) = (k, k);
    while lo > 0 && near(lo - 1) {
        lo -= 1;
    }
    while hi + 1 < response.len() && near(hi + 1) {
        hi += 1;
    }
    if hi > lo {
        // A flat top cut off by the search window has no reliable middle.
        if lo == 0 || hi + 1 == response.len() {
            return (0., f64::NEG_INFINITY);
        }
        return ((offsets[lo] + offsets[hi]) / 2., value);
    }
    let mut shift = offsets[k];
    if k > 0 && k + 1 < response.len() {
        let (a, b, c) = (response[k - 1], response[k], response[k + 1]);
        let d = a - 2. * b + c;
        if d < 0. {
            shift += 0.5 * (a - c) / d * (offsets[1] - offsets[0]);
        }
    }
    (shift, value)
}

fn floor(channel: Channel) -> f64 {
    match channel {
        Channel::Luma => 12.,
        Channel::Chroma => 7.,
    }
}

/// Sub-pixel step positions for each sample, searched within `radius` cells.
fn edge_points(planes: &Planes, h: &Homography, samples: &[Sample], radius: f64) -> Vec<[f64; 2]> {
    let steps = ((radius / 0.025) as usize * 2 + 1).max(9);
    let offsets: Vec<f64> = (0..steps)
        .map(|k| -radius + 2. * radius * k as f64 / (steps - 1) as f64)
        .collect();
    let mut found = Vec::new();
    for s in samples {
        let normal = [-s.dv, s.du];
        let value = |t: f64| {
            [-0.06, 0., 0.06]
                .iter()
                .map(|&a| {
                    let uv = [
                        s.u + t * s.du + a * normal[0],
                        s.v + t * s.dv + a * normal[1],
                    ];
                    planes.sample(s.channel, apply(h, uv))
                })
                .sum::<f64>()
                / 3.
        };
        let response: Vec<f64> = offsets
            .iter()
            .map(|&t| s.sign * (value(t + W) - value(t - W)))
            .collect();
        let (shift, strength) = peak(&response, &offsets);
        if strength >= floor(s.channel) {
            found.push(apply(h, [s.u + shift * s.du, s.v + shift * s.dv]));
        }
    }
    found
}

/// Image line through `point` along unit `direction`.
#[derive(Clone, Copy)]
struct Line {
    point: [f64; 2],
    direction: [f64; 2],
}

/// Robust line: the pair with most inliers (exhaustive for these small sets),
/// then a Huber-weighted fit on those inliers (`cv::fitLine`, DIST_HUBER).
fn fit_line(points: &[[f64; 2]], tolerance: f64) -> Option<Line> {
    if points.len() < 4 {
        return None;
    }
    let mut best: Option<Vec<bool>> = None;
    for i in 0..points.len() {
        for j in i + 1..points.len() {
            let d = [points[j][0] - points[i][0], points[j][1] - points[i][1]];
            let n = d[0].hypot(d[1]);
            if n < 1e-6 {
                continue;
            }
            let normal = [-d[1] / n, d[0] / n];
            let inliers: Vec<bool> = points
                .iter()
                .map(|p| {
                    ((p[0] - points[i][0]) * normal[0] + (p[1] - points[i][1]) * normal[1]).abs()
                        < tolerance
                })
                .collect();
            let count = inliers.iter().filter(|&&b| b).count();
            if best
                .as_ref()
                .is_none_or(|b| count > b.iter().filter(|&&x| x).count())
            {
                best = Some(inliers);
            }
        }
    }
    let best = best?;
    let kept: Vec<[f64; 2]> = points
        .iter()
        .zip(&best)
        .filter(|(_, &b)| b)
        .map(|(p, _)| *p)
        .collect();
    if (kept.len() as f64) < (0.4 * points.len() as f64).max(4.) {
        return None;
    }
    let mut weights = vec![1.; kept.len()];
    let mut line = None;
    for _ in 0..10 {
        let total: f64 = weights.iter().sum();
        let mean = [0, 1].map(|k| {
            kept.iter()
                .zip(&weights)
                .map(|(p, w)| p[k] * w)
                .sum::<f64>()
                / total
        });
        let (mut sxx, mut sxy, mut syy) = (0., 0., 0.);
        for (p, w) in kept.iter().zip(&weights) {
            let (dx, dy) = (p[0] - mean[0], p[1] - mean[1]);
            sxx += w * dx * dx;
            sxy += w * dx * dy;
            syy += w * dy * dy;
        }
        let angle = 0.5 * (2. * sxy).atan2(sxx - syy);
        let direction = [angle.cos(), angle.sin()];
        let fitted = Line {
            point: mean,
            direction,
        };
        for (p, w) in kept.iter().zip(weights.iter_mut()) {
            let r = ((p[0] - mean[0]) * -direction[1] + (p[1] - mean[1]) * direction[0]).abs();
            *w = if r < 1.345 { 1. } else { 1.345 / r };
        }
        line = Some(fitted);
    }
    line
}

fn intersect(a: Line, b: Line) -> Option<[f64; 2]> {
    let det = a.direction[0] * -b.direction[1] + b.direction[0] * a.direction[1];
    if det.abs() < 1e-6 {
        return None;
    }
    let (qx, qy) = (b.point[0] - a.point[0], b.point[1] - a.point[1]);
    let s = (qx * -b.direction[1] + b.direction[0] * qy) / det;
    Some([
        a.point[0] + s * a.direction[0],
        a.point[1] + s * a.direction[1],
    ])
}

pub(super) struct Measured {
    model: Vec<[f64; 2]>,
    image: Vec<[f64; 2]>,
    lines: Vec<(&'static str, bool)>,
}

/// Intersections of the fitted printed lines. `max_v`/`max_u` restrict the
/// first pass of a seed that only knows its own band.
fn measure(planes: &Planes, h: &Homography, radius: f64, max_v: f64, max_u: f64) -> Measured {
    let cp = cell_px(h);
    let mut fitted: Vec<(&'static str, f64, bool, Line)> = Vec::new();
    for spec in line_specs() {
        // The header top seeds detection and anchors the indexing, but its
        // height varies with print and scan more than the body lines, so it
        // does not pull the fitted grid.
        if spec.name == "header-top"
            || (spec.horizontal && spec.value > max_v)
            || (!spec.horizontal && spec.value > max_u)
        {
            continue;
        }
        let samples: Vec<Sample> = spec
            .samples
            .into_iter()
            .filter(|s| s.v <= max_v && s.u <= max_u)
            .collect();
        let points = edge_points(planes, h, &samples, radius);
        let line = fit_line(&points, (0.06 * cp).max(1.));
        if let Some(line) = line {
            fitted.push((spec.name, spec.value, spec.horizontal, line));
        }
    }
    let mut measured = Measured {
        model: Vec::new(),
        image: Vec::new(),
        lines: fitted.iter().map(|f| (f.0, f.2)).collect(),
    };
    for &(_, hv, horizontal, hl) in &fitted {
        if !horizontal {
            continue;
        }
        for &(_, vv, vertical_is_horizontal, vl) in &fitted {
            if vertical_is_horizontal {
                continue;
            }
            if let Some(point) = intersect(hl, vl) {
                measured.model.push([vv, hv]);
                measured.image.push(point);
            }
        }
    }
    measured
}

/// Signed step strength of every printed boundary for a grid shifted by
/// (du, dv) cells, read exactly where the shifted model puts it.
fn anchors(planes: &Planes, h: &Homography, du: f64, dv: f64) -> f64 {
    let (mut total, mut count) = (0., 0usize);
    for spec in line_specs() {
        for s in &spec.samples {
            let (u, v) = (s.u + du, s.v + dv);
            let ahead = planes.sample(s.channel, apply(h, [u + W * s.du, v + W * s.dv]));
            let behind = planes.sample(s.channel, apply(h, [u - W * s.du, v - W * s.dv]));
            total += (s.sign * (ahead - behind) / floor(s.channel)).clamp(-3., 3.);
            count += 1;
        }
    }
    total / count as f64
}

/// Anchor score and its margin over every half- and whole-cell shift.
pub(super) fn anchored(planes: &Planes, h: &Homography) -> (f64, f64) {
    let base = anchors(planes, h, 0., 0.);
    const SHIFTS: [f64; 5] = [-1., -0.5, 0., 0.5, 1.];
    let best_other = SHIFTS
        .iter()
        .flat_map(|&du| SHIFTS.iter().map(move |&dv| (du, dv)))
        .filter(|&(du, dv)| du != 0. || dv != 0.)
        .map(|(du, dv)| anchors(planes, h, du, dv))
        .fold(f64::NEG_INFINITY, f64::max);
    (base, base - best_other)
}

/// Fraction of column gaps (u = 1..9, every row) clear of ink: with the right
/// column pitch the strip between two letters is lighter than their ink.
pub(super) fn column_gaps(planes: &Planes, h: &Homography) -> f64 {
    let gray = |u: f64, v: f64| planes.sample(Channel::Luma, apply(h, [u, v]));
    let mut strips = Vec::new();
    let mut all = Vec::new();
    for c in 1..COLS as usize {
        for r in 0..ROWS as usize {
            let (c, r) = (c as f64, r as f64);
            let mut gap = f64::INFINITY;
            for du in [-0.04, 0., 0.04] {
                for k in 0..7 {
                    let value = gray(c + du, r + 0.2 + 0.1 * k as f64);
                    gap = gap.min(value);
                    all.push(value);
                }
            }
            let ink = (0..5)
                .map(|k| -0.2 + 0.1 * k as f64)
                .flat_map(|d| [gray(c - 0.5 + d, r + 0.5), gray(c + 0.5 + d, r + 0.5)])
                .fold(f64::INFINITY, f64::min);
            strips.push((gap, ink));
        }
    }
    all.sort_by(f64::total_cmp);
    let paper = all[(all.len() - 1) * 9 / 10];
    strips
        .iter()
        .filter(|(gap, ink)| *gap > ink + 0.35 * (paper - ink))
        .count() as f64
        / strips.len() as f64
}

fn geometry_reject(planes: &Planes, h: &Homography) -> Option<&'static str> {
    let b = body(h);
    let cross = |i: usize| {
        let (p, q, r) = (b[i], b[(i + 1) % 4], b[(i + 2) % 4]);
        (q[0] - p[0]) * (r[1] - q[1]) - (q[1] - p[1]) * (r[0] - q[0])
    };
    // Convex and clockwise on screen (y down): the printed grid can be turned
    // any way but never mirrored, and its rows are symmetric top to bottom,
    // so a reflected fit would otherwise pass every other check.
    if (0..4).any(|i| cross(i) <= 0.) {
        return Some("non-convex or mirrored");
    }
    if b.iter()
        .any(|p| p[0] < 0. || p[1] < 0. || p[0] >= planes.width() || p[1] >= planes.height())
    {
        return Some("clipped");
    }
    // Conditioning in normalized coordinates: model unit box to image unit box.
    let t = nalgebra::Matrix3::new(
        1. / planes.width(),
        0.,
        0.,
        0.,
        1. / planes.height(),
        0.,
        0.,
        0.,
        1.,
    );
    let m = nalgebra::Matrix3::new(COLS, 0., 0., 0., ROWS, 0., 0., 0., 1.);
    let mut hn = t * nalgebra::Matrix3::from_row_slice(h) * m;
    hn /= hn[(2, 2)];
    let singular = hn.singular_values();
    if singular.min() <= 0. || singular.max() / singular.min() > 1e4 {
        return Some("ill-conditioned");
    }
    // Foreshortening: projective depth across the card stays within 2.5x.
    let depths: Vec<f64> = [
        [-1., -1.],
        [COLS + 1., -1.],
        [COLS + 1., ROWS + 1.],
        [-1., ROWS + 1.],
    ]
    .iter()
    .map(|[u, v]| h[6] * u + h[7] * v + h[8])
    .collect();
    let (lo, hi) = depths
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &d| {
            (a.min(d), b.max(d))
        });
    if lo <= 0. || hi / lo > 2.5 {
        return Some("foreshortened");
    }
    None
}

/// Where a seed starts growing from before the full grid is measured.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Grow {
    Header,
    Labels,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FitInfo {
    pub(crate) inliers: usize,
    pub(crate) measured: usize,
    pub(crate) residual: f64,
    pub(crate) anchor: f64,
    pub(crate) anchor_margin: f64,
    pub(crate) column_gaps: f64,
    pub(crate) reason: Option<&'static str>,
}

/// One homography over all measured intersections, or the gate that failed.
pub(crate) fn fit(
    planes: &Planes,
    mut h: Homography,
    radii: &[f64],
    grow: Option<Grow>,
) -> (Option<Homography>, FitInfo) {
    let mut info = FitInfo::default();
    let first: &[(f64, f64, f64)] = match grow {
        Some(Grow::Header) => &[(0.45, 3., COLS)],
        Some(Grow::Labels) => &[(0.45, ROWS, 3.)],
        None => &[],
    };
    let stages: Vec<(f64, f64, f64)> = first
        .iter()
        .copied()
        .chain(radii.iter().map(|&r| (r, ROWS, COLS)))
        .collect();
    let mut last = None;
    for (radius, max_v, max_u) in stages {
        let measured = measure(planes, &h, radius, max_v, max_u);
        let partial = max_v < ROWS || max_u < COLS;
        if measured.model.len() < if partial { 6 } else { 8 } {
            info.reason = Some("intersections");
            return (None, info);
        }
        // The reference's MAGSAC scores residuals softly up to 0.05 cell. Plain
        // RANSAC at that hard cut lets one side of a bent card outvote the
        // other, so take the consensus at 0.1 cell; the RMS gate below still
        // holds the fit to 0.08 cell.
        let threshold = (0.1 * cell_px(&h)).max(1.5);
        let Some((next, mask)) =
            find_homography(&measured.model, &measured.image, threshold, 2000, 0.999)
        else {
            info.reason = Some("homography");
            return (None, info);
        };
        h = next;
        last = Some((measured, mask));
    }
    let Some((measured, mask)) = last else {
        info.reason = Some("intersections");
        return (None, info);
    };
    let cp = cell_px(&h);
    let kept: Vec<usize> = (0..mask.len()).filter(|&i| mask[i]).collect();
    let squared: f64 = kept
        .iter()
        .map(|&i| distance(apply(&h, measured.model[i]), measured.image[i]).powi(2))
        .sum();
    info.inliers = kept.len();
    info.measured = measured.model.len();
    info.residual = (squared / kept.len().max(1) as f64).sqrt() / cp;
    let vertical: Vec<&str> = measured
        .lines
        .iter()
        .filter(|l| !l.1)
        .map(|l| l.0)
        .collect();
    let horizontal = measured.lines.iter().filter(|l| l.1).count();
    let rows: Vec<f64> = kept.iter().map(|&i| measured.model[i][1]).collect();
    let (top, bottom) = rows
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    // Inliers must span the grid: both body edges, each from the top rows to
    // the bottom rows, and rows at top and bottom.
    let edge_spans = |edges: &[f64]| {
        let rows: Vec<f64> = kept
            .iter()
            .map(|&i| measured.model[i])
            .filter(|m| edges.contains(&m[0]))
            .map(|m| m[1])
            .collect();
        rows.len() >= 4 && rows.iter().any(|&v| v <= 1.) && rows.iter().any(|&v| v >= ROWS - 1.)
    };
    if !edge_spans(&[COLS])
        || !edge_spans(&[0., -LABELS])
        || kept.len() < 8
        || (kept.len() as f64) < 0.6 * mask.len() as f64
        || vertical.len() < 2
        || horizontal < 5
        || !(vertical.contains(&"body-left") || vertical.contains(&"labels-left"))
        || !vertical.contains(&"body-right")
        || top > 1.
        || bottom < ROWS - 1.
    {
        info.reason = Some("inlier coverage");
        return (None, info);
    }
    if info.residual > 0.08 {
        info.reason = Some("residual");
        return (None, info);
    }
    if let Some(reason) = geometry_reject(planes, &h) {
        info.reason = Some(reason);
        return (None, info);
    }
    (info.anchor, info.anchor_margin) = anchored(planes, &h);
    if info.anchor_margin < 0.3 {
        info.reason = Some("anchor aliasing");
        return (None, info);
    }
    info.column_gaps = column_gaps(planes, &h);
    if info.column_gaps < 0.7 {
        info.reason = Some("column pitch");
        return (None, info);
    }
    (Some(h), info)
}

pub(super) fn prior(planes: &Planes, h: &Homography) -> f64 {
    anchors(planes, h, 0., 0.)
}
