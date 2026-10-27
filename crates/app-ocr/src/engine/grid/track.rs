//! Between fits, follow the card with pyramidal Lucas-Kanade (the
//! `optical-flow-lk` crate) on Shi-Tomasi corners inside the card and a RANSAC
//! frame-to-frame homography. Mirrors `Tracker` and `OneEuro` of the
//! reference: refit every few frames or on weak tracking, detect only when no
//! grid survives, confirm sudden jumps, smooth only the displayed corners.
use super::fit::{fit, FitInfo};
use super::{apply, body, cell_px, detect, distance, multiply, Planes, COLS, ROWS};
use crate::engine::vendor::opencv_homography::{find_homography, Homography};
use optical_flow_lk::{
    good_features_to_track, TrackKernel, TrackOptions, TrackParams, TrackStatus, TrackerContext,
    DEFAULT_MIN_EIGEN_THRESHOLD,
};

const REFIT_EVERY: usize = 8;
const LEVELS: usize = 4;
const PARAMS: TrackParams = TrackParams {
    window_size: 21,
    max_iterations: 30,
    min_eigen_threshold: DEFAULT_MIN_EIGEN_THRESHOLD,
};
// The f32 kernel: the crate's long-standing arithmetic on every target.
const OPTIONS: TrackOptions = TrackOptions::new().with_kernel(TrackKernel::Reference);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrackMode {
    Lost,
    Detected,
    Refitted,
    Tracked,
}

/// One analysed frame. Quads are in the analysed image's pixels.
pub(crate) struct TrackStep {
    pub(crate) mode: TrackMode,
    pub(crate) grid: Option<Homography>,
    pub(crate) display: Option<[[f64; 2]; 4]>,
    /// Geometry fresh from a fit that is good enough to crop cells from.
    pub(crate) keyframe: bool,
    pub(crate) fit: Option<FitInfo>,
    pub(crate) stages: Vec<(&'static str, Homography)>,
}

/// One Euro filter (Casiez et al.) over the eight corner coordinates.
struct OneEuro {
    x: Option<[f64; 8]>,
    dx: [f64; 8],
    t: f64,
}
impl OneEuro {
    const MIN_CUTOFF: f64 = 1.5;
    const BETA: f64 = 0.02;
    const D_CUTOFF: f64 = 1.;
    fn new() -> Self {
        Self {
            x: None,
            dx: [0.; 8],
            t: 0.,
        }
    }
    fn alpha(cutoff: f64, dt: f64) -> f64 {
        let tau = 1. / (2. * std::f64::consts::PI * cutoff);
        1. / (1. + tau / dt)
    }
    fn filter(&mut self, value: [f64; 8], t: f64) -> [f64; 8] {
        let Some(x) = self.x.filter(|_| t > self.t) else {
            self.x = Some(value);
            self.dx = [0.; 8];
            self.t = t;
            return value;
        };
        let dt = t - self.t;
        let a_d = Self::alpha(Self::D_CUTOFF, dt);
        let mut out = x;
        for i in 0..8 {
            let dx = (value[i] - x[i]) / dt;
            self.dx[i] += a_d * (dx - self.dx[i]);
            let cutoff = Self::MIN_CUTOFF + Self::BETA * self.dx[i].abs();
            out[i] = x[i] + Self::alpha(cutoff, dt) * (value[i] - x[i]);
        }
        self.x = Some(out);
        self.t = t;
        out
    }
}

fn area(q: &[[f64; 2]; 4]) -> f64 {
    (0..4)
        .map(|i| q[i][0] * q[(i + 1) % 4][1] - q[(i + 1) % 4][0] * q[i][1])
        .sum::<f64>()
        .abs()
        / 2.
}

fn angle(q: &[[f64; 2]; 4]) -> f64 {
    (q[1][1] - q[0][1]).atan2(q[1][0] - q[0][0]).to_degrees()
}

/// Two consecutive detections disagree in size or position.
fn jump_between(a: &[[f64; 2]; 4], b: &[[f64; 2]; 4]) -> bool {
    let ratio = area(b) / area(a).max(1.);
    let move_limit = 0.5 * distance(a[1], a[0]) / COLS;
    !(0.8..1.25).contains(&ratio) || (0..4).any(|i| distance(a[i], b[i]) > move_limit)
}

pub(crate) struct Tracker {
    context: TrackerContext,
    size: (u32, u32),
    has_previous: bool,
    grid: Option<Homography>,
    points: Vec<(f32, f32)>,
    since_fit: usize,
    smooth: OneEuro,
    last_accepted: Option<([[f64; 2]; 4], f64)>,
    pending: Option<[[f64; 2]; 4]>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            context: TrackerContext::new(),
            size: (0, 0),
            has_previous: false,
            grid: None,
            points: Vec::new(),
            since_fit: 0,
            smooth: OneEuro::new(),
            last_accepted: None,
            pending: None,
        }
    }
}

impl Tracker {
    /// Shi-Tomasi corners inside the card (labels and header included).
    fn features(planes: &Planes, h: &Homography) -> Vec<(f32, f32)> {
        let outline = [[-1., -1.], [COLS, -1.], [COLS, ROWS], [-1., ROWS]].map(|p| apply(h, p));
        let inside = |x: f64, y: f64| {
            (0..4).all(|i| {
                let (a, b) = (outline[i], outline[(i + 1) % 4]);
                (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]) >= 0.
            })
        };
        let spacing = (0.35 * cell_px(h)).max(3.) as u32;
        good_features_to_track(&planes.luma, 0.01, spacing)
            .into_iter()
            .filter(|c| inside(c.x as f64, c.y as f64))
            .take(200)
            .map(|c| c.position())
            .collect()
    }

    /// Sudden area/angle change from the last accepted grid (within 0.3 s).
    fn jump(&self, grid: &Homography, t: f64) -> bool {
        let Some((last, at)) = self.last_accepted.filter(|(_, at)| t - at <= 0.3) else {
            return false;
        };
        let _ = at;
        let b = body(grid);
        let ratio = area(&b) / area(&last).max(1.);
        let turn = ((angle(&b) - angle(&last) + 180.).rem_euclid(360.) - 180.).abs();
        !(0.6 < ratio && ratio < 1.65) || turn > 25.
    }

    fn accept(&mut self, planes: &Planes, grid: Homography, t: f64) -> bool {
        if self.jump(&grid, t) {
            // A real move is confirmed by the next independent measurement.
            let b = body(&grid);
            let confirmed = self.pending.is_some_and(|p| !jump_between(&p, &b));
            self.pending = Some(b);
            if !confirmed {
                return false;
            }
        }
        self.pending = None;
        self.grid = Some(grid);
        self.last_accepted = Some((body(&grid), t));
        self.points = Self::features(planes, &grid);
        self.since_fit = 0;
        true
    }

    /// Frame-to-frame homography from tracked corners, if healthy.
    fn follow(&mut self) -> Option<Homography> {
        let grid = self.grid?;
        if self.points.len() < 8 {
            return None;
        }
        let results = self
            .context
            .track_fb_with(&self.points, None, PARAMS, 1.0, OPTIONS)
            .to_vec();
        let (before, after): (Vec<[f64; 2]>, Vec<[f64; 2]>) = self
            .points
            .iter()
            .zip(&results)
            .filter(|(_, r)| r.status == TrackStatus::Tracked)
            .map(|(p, r)| ([p.0 as f64, p.1 as f64], [r.pos.0 as f64, r.pos.1 as f64]))
            .unzip();
        if before.len() < 12 {
            return None;
        }
        let (step, mask) = find_homography(&before, &after, 1.5, 2000, 0.995)?;
        let kept: Vec<usize> = (0..mask.len()).filter(|&i| mask[i]).collect();
        let residual = (kept
            .iter()
            .map(|&i| distance(apply(&step, before[i]), after[i]).powi(2))
            .sum::<f64>()
            / kept.len().max(1) as f64)
            .sqrt()
            / cell_px(&grid);
        if (kept.len() as f64) < 0.5 * self.points.len() as f64 || residual >= 0.15 {
            return None;
        }
        self.points = kept
            .iter()
            .map(|&i| (after[i][0] as f32, after[i][1] as f32))
            .collect();
        Some(multiply(&step, &grid))
    }

    /// Analyse the next frame; `t` is its capture time in seconds.
    pub(crate) fn step(&mut self, planes: &Planes, t: f64) -> TrackStep {
        let size = (planes.luma.width(), planes.luma.height());
        if size != self.size {
            *self = Self {
                size,
                ..Self::default()
            };
        }
        self.context.advance(&planes.luma, LEVELS);
        let tracked = if self.has_previous {
            self.follow()
        } else {
            None
        };
        self.has_previous = true;
        let mut stages = Vec::new();
        if let Some(h) = tracked {
            stages.push(("track", h));
        }
        let mut fit_info = None;
        let due = tracked.is_none() || self.since_fit + 1 >= REFIT_EVERY;
        let (grid, mode) = if let (Some(h), false) = (tracked, due) {
            self.grid = Some(h);
            self.since_fit += 1;
            if self.points.len() < 40 {
                self.points = Self::features(planes, &h);
            }
            (Some(h), TrackMode::Tracked)
        } else {
            let seed = tracked.or(self.grid);
            let refit = seed.and_then(|seed| {
                let (h, info) = fit(planes, seed, &[0.3, 0.2], None);
                fit_info = Some(info);
                h
            });
            match (refit, tracked) {
                (Some(h), _) if self.accept(planes, h, t) => {
                    stages.push(("fit", h));
                    (Some(h), TrackMode::Refitted)
                }
                // Healthy tracking survives one failed refit.
                (_, Some(h)) => {
                    self.grid = Some(h);
                    self.since_fit += 1;
                    (Some(h), TrackMode::Tracked)
                }
                _ => match detect(planes) {
                    Some((seed, h, info)) => {
                        stages.push(("detect", seed));
                        stages.push(("fit", h));
                        fit_info = Some(info);
                        if self.accept(planes, h, t) {
                            self.smooth = OneEuro::new();
                            (Some(h), TrackMode::Detected)
                        } else {
                            self.grid = None;
                            self.points.clear();
                            (None, TrackMode::Lost)
                        }
                    }
                    None => {
                        self.grid = None;
                        self.points.clear();
                        (None, TrackMode::Lost)
                    }
                },
            }
        };
        let keyframe = matches!(mode, TrackMode::Detected | TrackMode::Refitted)
            && fit_info.as_ref().is_some_and(|f| f.residual < 0.08);
        let display = grid.map(|h| {
            let b = body(&h);
            let flat = self
                .smooth
                .filter(std::array::from_fn(|i| b[i / 2][i % 2]), t);
            std::array::from_fn(|i| [flat[i * 2], flat[i * 2 + 1]])
        });
        TrackStep {
            mode,
            grid,
            display,
            keyframe,
            fit: fit_info,
            stages,
        }
    }
}
