//! Live Matrix scheduling and per-cell confidence-weighted consensus.
//! Camera resources and model execution belong to capability adapters.
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use thiserror::Error;

pub const CELLS: usize = 70;
const HIGH_CONFIDENCE: f64 = 0.8;
const CELL_WINDOW: usize = 6;
const MIN_CELL_SAMPLES: usize = 3;
mod capture;
mod driver;
mod vision;
pub use capture::*;
pub use driver::*;
pub use vision::*;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixPoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixReading {
    pub text: String,
    pub confidence: f64,
}
fn valid_reading(reading: &MatrixReading) -> bool {
    reading.confidence.is_finite()
        && (0. ..=1.).contains(&reading.confidence)
        && (reading.text.is_empty()
            || (reading.text.len() == 1 && reading.text.as_bytes()[0].is_ascii_uppercase()))
}
/// How far a recognized cell can be trusted when a person reviews the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum MatrixConfidence {
    High,
    /// Read, but worth checking against the card.
    Medium,
    /// Unreadable or likely wrong.
    Low,
}

/// The review level of one recognized cell; every shell colours by this.
pub fn matrix_confidence(text: &str, confidence: f64) -> MatrixConfidence {
    if text.is_empty() || !confidence.is_finite() || confidence < 0.75 {
        MatrixConfidence::Low
    } else if confidence >= 0.9 {
        MatrixConfidence::High
    } else {
        MatrixConfidence::Medium
    }
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixVote {
    pub text: String,
    pub confidence: f64,
    pub stable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum MatrixScanPhase {
    Initial,
    Refine,
    Verify,
    Complete,
}

/// A plan consumes at most one observation per selected cell from one new frame.
#[derive(Clone, Debug)]
pub struct MatrixScanPlan {
    revision: usize,
    pub cells: Vec<usize>,
    pub phase: MatrixScanPhase,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixScanTiming {
    pub decode_ms: f64,
    pub geometry_ms: f64,
    pub preprocess_ms: f64,
    pub inference_ms: f64,
    pub total_ms: f64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixScanSnapshot {
    pub bounds: Option<[MatrixPoint; 4]>,
    pub confidence: f64,
    pub cells: Vec<MatrixVote>,
    pub stable_cells: usize,
    pub complete: bool,
    pub needs_adjustment: bool,
    pub phase: MatrixScanPhase,
    pub sampled_cells: usize,
    pub frames: usize,
    pub timing: MatrixScanTiming,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixScanOutline {
    pub bounds: [MatrixPoint; 4],
}

/// Shell lifecycle/performance facts only; no letters or photos are logged.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixCaptureMetrics {
    pub elapsed_ms: f64,
    pub tracked_ms: Option<f64>,
    pub frames: usize,
    pub frame_ms: Vec<f64>,
    #[serde(default)]
    pub timeline: Vec<MatrixFrameTimeline>,
}
/// Webview clock (`performance.now()`, ms) for one frame sent for analysis.
/// `media_ms` is the video position of the analysed frame. `painted_ms` is
/// when that frame was drawn with its outline, `painted_media_ms` the video
/// position of the drawn picture and `live_media_ms` the live video's then.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixFrameTimeline {
    pub frame_id: u32,
    pub media_ms: f64,
    pub captured_ms: f64,
    pub sent_ms: f64,
    pub responded_ms: f64,
    pub sequence: u32,
    pub bounds_frame_id: Option<u32>,
    pub painted_ms: Option<f64>,
    pub painted_media_ms: Option<f64>,
    #[serde(default)]
    pub live_media_ms: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixScanDiagnostics {
    pub phase: MatrixScanPhase,
    pub frames: usize,
    pub analyzed_cells: usize,
    pub elapsed_ms: f64,
    pub qualified_samples: Vec<usize>,
    pub stable_cells: usize,
    pub attempts: Vec<usize>,
    pub blank_reads: Vec<usize>,
    pub weak_reads: Vec<usize>,
    pub conflicts: Vec<usize>,
    pub timings: Vec<MatrixScanTiming>,
}

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum MatrixScanError {
    #[error("This camera scan has ended. Open the camera again.")]
    Ended,
    #[error("Invalid matrix frame: {0}")]
    InvalidFrame(String),
    /// A still photo without a readable Matrix, or not an image at all.
    #[error("{0}")]
    Unreadable(String),
    /// This build or device has no OCR backend; shells hide the scanner.
    #[error("Matrix scanner is unavailable in this build.")]
    Unavailable,
    #[error("{0}")]
    Backend(String),
}
fn outline(quads: &[[MatrixPoint; 4]]) -> Result<MatrixScanOutline, MatrixScanError> {
    if quads.len() != CELLS
        || quads.iter().flatten().any(|p| {
            !p.x.is_finite()
                || !p.y.is_finite()
                || !(0. ..=1.).contains(&p.x)
                || !(0. ..=1.).contains(&p.y)
        })
    {
        return Err(MatrixScanError::InvalidFrame(
            "Expected 70 normalized cell quadrilaterals.".into(),
        ));
    }
    Ok(MatrixScanOutline {
        bounds: [quads[0][0], quads[9][1], quads[69][2], quads[60][3]],
    })
}
#[derive(Default)]
pub struct MatrixScanner {
    full_frame: bool,
    active: Option<(String, Consensus)>,
}
impl MatrixScanner {
    /// Fast classifiers can afford a whole-card readability check on every frame.
    pub fn full_frame() -> Self {
        Self {
            full_frame: true,
            active: None,
        }
    }
    pub fn begin(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.active = Some((
            id.clone(),
            Consensus {
                full_frame: self.full_frame,
                ..Consensus::default()
            },
        ));
        id
    }
    pub fn is_active(&self, id: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(current, _)| current == id)
    }
    fn session(&self, id: &str) -> Result<&Consensus, MatrixScanError> {
        self.active
            .as_ref()
            .filter(|(current, _)| current == id)
            .map(|(_, s)| s)
            .ok_or(MatrixScanError::Ended)
    }
    pub fn plan(&self, id: &str) -> Result<MatrixScanPlan, MatrixScanError> {
        let session = self.session(id)?;
        if session.complete {
            return Err(MatrixScanError::Ended);
        }
        Ok(session.plan())
    }
    pub fn diagnostics(&self, id: &str) -> Result<MatrixScanDiagnostics, MatrixScanError> {
        Ok(self.session(id)?.diagnostics())
    }
    pub fn outline(
        &self,
        id: &str,
        quads: &[[MatrixPoint; 4]],
    ) -> Result<MatrixScanOutline, MatrixScanError> {
        self.session(id)?;
        outline(quads)
    }
    pub fn stop(&mut self, id: &str) {
        if self.is_active(id) {
            self.active = None;
        }
    }
    pub fn observe(
        &mut self,
        id: &str,
        plan: &MatrixScanPlan,
        frame: Option<(Vec<[MatrixPoint; 4]>, Vec<MatrixReading>)>,
        timing: MatrixScanTiming,
    ) -> Result<MatrixScanSnapshot, MatrixScanError> {
        let (_, session) = self
            .active
            .as_mut()
            .filter(|(current, _)| current == id)
            .ok_or(MatrixScanError::Ended)?;
        session.observe(plan, frame, timing)
    }
}
#[derive(Clone, Default)]
struct CellEvidence {
    samples: VecDeque<MatrixReading>,
    last_attempt: usize,
    attempts: usize,
    blank_reads: usize,
    weak_reads: usize,
    conflicts: usize,
}
impl CellEvidence {
    fn vote(&self) -> MatrixVote {
        let mut weights = [0.; 26];
        let mut counts = [0usize; 26];
        for sample in &self.samples {
            let i = (sample.text.as_bytes()[0] - b'A') as usize;
            weights[i] += sample.confidence;
            counts[i] += 1;
        }
        let winner = (0..26)
            .max_by(|&a, &b| weights[a].total_cmp(&weights[b]))
            .unwrap_or(0);
        let total: f64 = weights.iter().sum();
        let share = if total > 0. {
            weights[winner] / total
        } else {
            0.
        };
        let mean = if counts[winner] > 0 {
            weights[winner] / counts[winner] as f64
        } else {
            0.
        };
        MatrixVote {
            text: if total > 0. {
                ((b'A' + winner as u8) as char).to_string()
            } else {
                String::new()
            },
            confidence: mean * share,
            stable: self.samples.len() >= MIN_CELL_SAMPLES
                && counts[winner] * 2 > self.samples.len()
                && share > 0.5,
        }
    }
    fn add(&mut self, reading: MatrixReading, revision: usize) {
        self.collect(reading, revision, true);
    }
    fn collect(&mut self, reading: MatrixReading, revision: usize, reopen: bool) {
        self.last_attempt = revision;
        self.attempts += 1;
        if reading.text.is_empty() {
            self.blank_reads += 1;
            return;
        }
        if reading.confidence < HIGH_CONFIDENCE {
            self.weak_reads += 1;
            return;
        }
        let previous = self.vote();
        // A new clear contradiction reopens a settled cell instead of allowing
        // its old majority to hide a changed card or an earlier wrong result.
        if reopen && previous.stable && previous.text != reading.text {
            self.conflicts += 1;
            self.samples.clear();
        }
        if self.samples.len() == CELL_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(reading);
    }
}
struct Consensus {
    full_frame: bool,
    evidence: Vec<CellEvidence>,
    revision: usize,
    tracked_frames: usize,
    missing_since: Option<Instant>,
    started: Instant,
    analyzed_cells: usize,
    complete: bool,
    stalled_rounds: usize,
    timings: VecDeque<MatrixScanTiming>,
}
impl Default for Consensus {
    fn default() -> Self {
        Self {
            full_frame: false,
            evidence: vec![CellEvidence::default(); CELLS],
            revision: 0,
            tracked_frames: 0,
            missing_since: None,
            started: Instant::now(),
            analyzed_cells: 0,
            complete: false,
            stalled_rounds: 0,
            timings: VecDeque::new(),
        }
    }
}
impl Consensus {
    fn plan(&self) -> MatrixScanPlan {
        let votes: Vec<_> = self.evidence.iter().map(CellEvidence::vote).collect();
        let phase = if self.complete {
            MatrixScanPhase::Complete
        } else if self.tracked_frames == 0 {
            MatrixScanPhase::Initial
        } else if votes.iter().all(|v| v.stable) {
            MatrixScanPhase::Verify
        } else {
            MatrixScanPhase::Refine
        };
        let cells = if self.full_frame || phase != MatrixScanPhase::Refine {
            (0..CELLS).collect()
        } else {
            let mut pending: Vec<_> = (0..CELLS).filter(|&i| !votes[i].stable).collect();
            // Oldest attempts first: unreadable cells cannot starve the rest.
            pending.sort_by_key(|&i| {
                (
                    self.evidence[i].last_attempt,
                    self.evidence[i].samples.len(),
                    i,
                )
            });
            let mut selected: Vec<_> = pending.into_iter().take(10).collect();
            // Bounded background checks invalidate stale settled values early.
            if self.tracked_frames.is_multiple_of(8) {
                let mut settled: Vec<_> = (0..CELLS).filter(|&i| votes[i].stable).collect();
                settled.sort_by_key(|&i| self.evidence[i].last_attempt);
                selected.truncate(8);
                selected.extend(settled.into_iter().take(2));
            }
            selected
        };
        MatrixScanPlan {
            revision: self.revision,
            cells,
            phase,
        }
    }
    fn observe(
        &mut self,
        plan: &MatrixScanPlan,
        frame: Option<(Vec<[MatrixPoint; 4]>, Vec<MatrixReading>)>,
        timing: MatrixScanTiming,
    ) -> Result<MatrixScanSnapshot, MatrixScanError> {
        if plan.revision != self.revision || self.complete {
            return Err(MatrixScanError::Ended);
        }
        let expected = self.plan();
        if plan.cells != expected.cells || plan.phase != expected.phase {
            return Err(MatrixScanError::InvalidFrame(
                "The cell plan was modified.".into(),
            ));
        }
        let Some((quads, readings)) = frame else {
            return Ok(self.missing(plan.phase, timing));
        };
        let bounds = outline(&quads)?.bounds;
        if readings.len() != plan.cells.len() || readings.iter().any(|r| !valid_reading(r)) {
            return Err(MatrixScanError::InvalidFrame(
                "Readings must match the planned cells and have finite confidence.".into(),
            ));
        }
        let clear = readings
            .iter()
            .filter(|r| !r.text.is_empty() && r.confidence >= HIGH_CONFIDENCE)
            .count();
        if self.full_frame && clear < CELLS * 7 / 10 {
            // Blur and false geometry must not donate isolated confident guesses.
            // Use the existing verification readability requirement for this mode.
            self.analyzed_cells += plan.cells.len();
            self.record_timing(timing.clone());
            return Ok(self.missing(plan.phase, timing));
        }
        self.revision += 1;
        self.tracked_frames += 1;
        self.missing_since = None;
        let before = self.evidence.iter().filter(|e| e.vote().stable).count();
        for (&i, reading) in plan.cells.iter().zip(readings) {
            self.evidence[i].add(reading, self.revision);
        }
        let after = self.evidence.iter().filter(|e| e.vote().stable).count();
        self.stalled_rounds = if after > before {
            0
        } else {
            self.stalled_rounds + 1
        };
        self.analyzed_cells += plan.cells.len();
        self.complete = plan.phase == MatrixScanPhase::Verify
            && clear >= CELLS * 7 / 10
            && self.evidence.iter().all(|e| e.vote().stable);
        self.record_timing(timing.clone());
        Ok(self.snapshot(Some(bounds), plan.phase, plan.cells.len(), timing))
    }
    fn missing(&mut self, phase: MatrixScanPhase, timing: MatrixScanTiming) -> MatrixScanSnapshot {
        self.revision += 1;
        let since = self.missing_since.get_or_insert_with(Instant::now);
        // Camera processing frequency varies: frame counts cannot represent a
        // consistent grace period for normal hand movement.
        if since.elapsed() >= Duration::from_secs(4) {
            self.evidence = vec![CellEvidence::default(); CELLS];
            self.tracked_frames = 0;
        }
        self.snapshot(None, phase, 0, timing)
    }
    fn record_timing(&mut self, timing: MatrixScanTiming) {
        if self.timings.len() == 128 {
            self.timings.pop_front();
        }
        self.timings.push_back(timing);
    }
    fn snapshot(
        &self,
        bounds: Option<[MatrixPoint; 4]>,
        phase: MatrixScanPhase,
        sampled_cells: usize,
        timing: MatrixScanTiming,
    ) -> MatrixScanSnapshot {
        let cells: Vec<_> = self.evidence.iter().map(CellEvidence::vote).collect();
        MatrixScanSnapshot {
            bounds,
            confidence: cells.iter().map(|c| c.confidence).fold(1., f64::min),
            stable_cells: cells.iter().filter(|c| c.stable).count(),
            cells,
            complete: self.complete,
            needs_adjustment: !self.complete
                && self.stalled_rounds >= 8
                && self
                    .evidence
                    .iter()
                    .any(|e| !e.vote().stable && e.attempts >= 4),
            phase: if self.complete {
                MatrixScanPhase::Complete
            } else {
                phase
            },
            sampled_cells,
            frames: self.revision,
            timing,
        }
    }
    fn diagnostics(&self) -> MatrixScanDiagnostics {
        MatrixScanDiagnostics {
            phase: self.plan().phase,
            frames: self.revision,
            analyzed_cells: self.analyzed_cells,
            elapsed_ms: self.started.elapsed().as_secs_f64() * 1000.,
            qualified_samples: self.evidence.iter().map(|e| e.samples.len()).collect(),
            stable_cells: self.evidence.iter().filter(|e| e.vote().stable).count(),
            attempts: self.evidence.iter().map(|e| e.attempts).collect(),
            blank_reads: self.evidence.iter().map(|e| e.blank_reads).collect(),
            weak_reads: self.evidence.iter().map(|e| e.weak_reads).collect(),
            conflicts: self.evidence.iter().map(|e| e.conflicts).collect(),
            timings: self.timings.iter().cloned().collect(),
        }
    }
}
#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    use ts_rs::TS;
    vec![
        MatrixPoint::decl(),
        MatrixReading::decl(),
        MatrixVote::decl(),
        MatrixConfidence::decl(),
        MatrixScanPhase::decl(),
        MatrixScanTiming::decl(),
        MatrixScanSnapshot::decl(),
        MatrixScanOutline::decl(),
        MatrixCaptureMetrics::decl(),
        MatrixFrameTimeline::decl(),
        MatrixCameraPhase::decl(),
        MatrixCameraSnapshot::decl(),
    ]
}
#[cfg(test)]
#[path = "tests/matrix_scan.rs"]
mod tests;
