//! Bounded, confidence-weighted consensus for a live matrix-card scan.
//! Camera handles and OCR engines stay in their platform/capability adapters.
use serde::Serialize;
use std::collections::VecDeque;
use thiserror::Error;

pub const CELLS: usize = 70;
const HIGH_CONFIDENCE: f64 = 0.9;
const CELL_WINDOW: usize = 6;
const MIN_CELL_SAMPLES: usize = 3;

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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixVote {
    pub text: String,
    /// Mean winning OCR confidence times its share of the qualified vote weight.
    pub confidence: f64,
    pub stable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixScanSnapshot {
    /// Only the outer Matrix boundary is rendered over the camera preview.
    pub bounds: Option<[MatrixPoint; 4]>,
    /// The weakest cell determines whole-card confidence.
    pub confidence: f64,
    pub cells: Vec<MatrixVote>,
    pub stable_cells: usize,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixScanOutline {
    pub bounds: [MatrixPoint; 4],
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

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "camelCase")]
pub enum MatrixScanError {
    #[error("This camera scan has ended. Open the camera again.")]
    Ended,
    #[error("Invalid matrix frame: {0}")]
    InvalidFrame(String),
    #[error("{0}")]
    Backend(String),
}

/// A single active scan. Starting/replacing/stopping a session invalidates late
/// worker results; shells never own or reconstruct the vote history.
#[derive(Default)]
pub struct MatrixScanner {
    active: Option<(String, Consensus)>,
}
impl MatrixScanner {
    pub fn begin(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.active = Some((id.clone(), Consensus::default()));
        id
    }
    pub fn is_active(&self, id: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(current, _)| current == id)
    }
    /// Geometry arrives before model inference; projecting it never adds votes.
    pub fn outline(
        &self,
        id: &str,
        quads: &[[MatrixPoint; 4]],
    ) -> Result<MatrixScanOutline, MatrixScanError> {
        if !self.is_active(id) {
            return Err(MatrixScanError::Ended);
        }
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
        frame: Option<(Vec<[MatrixPoint; 4]>, Vec<MatrixReading>)>,
    ) -> Result<MatrixScanSnapshot, MatrixScanError> {
        let Some((current, consensus)) = self.active.as_mut() else {
            return Err(MatrixScanError::Ended);
        };
        if current != id {
            return Err(MatrixScanError::Ended);
        }
        consensus.observe(frame)
    }
}

#[derive(Clone, Default)]
struct CellEvidence {
    // Each cell has its own sample window. A low-confidence A3 does not
    // consume a slot or dilute A3's votes when B2 happens to be readable.
    samples: VecDeque<MatrixReading>,
}
impl CellEvidence {
    fn add(&mut self, reading: &MatrixReading) -> MatrixVote {
        if !reading.text.is_empty() && reading.confidence >= HIGH_CONFIDENCE {
            if self.samples.len() == CELL_WINDOW {
                self.samples.pop_front();
            }
            self.samples.push_back(reading.clone());
        }
        let mut weights = [0.; 26];
        let mut counts = [0usize; 26];
        for sample in &self.samples {
            let index = (sample.text.as_bytes()[0] - b'A') as usize;
            weights[index] += sample.confidence;
            counts[index] += 1;
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
        let text = if total > 0. {
            ((b'A' + winner as u8) as char).to_string()
        } else {
            String::new()
        };
        MatrixVote {
            text,
            confidence: mean * share,
            stable: self.samples.len() >= MIN_CELL_SAMPLES
                && counts[winner] * 2 > self.samples.len()
                && share > 0.5,
        }
    }
}

struct Consensus {
    evidence: Vec<CellEvidence>,
    missing_frames: usize,
}
impl Default for Consensus {
    fn default() -> Self {
        Self {
            evidence: vec![CellEvidence::default(); CELLS],
            missing_frames: 0,
        }
    }
}
impl Consensus {
    fn observe(
        &mut self,
        frame: Option<(Vec<[MatrixPoint; 4]>, Vec<MatrixReading>)>,
    ) -> Result<MatrixScanSnapshot, MatrixScanError> {
        let Some((quads, readings)) = frame else {
            // Brief tracking failures while moving closer must not erase a scan.
            // Sustained absence starts a fresh card; no result is accepted while lost.
            self.missing_frames += 1;
            if self.missing_frames >= 4 {
                *self = Self::default();
            }
            return Ok(MatrixScanSnapshot {
                bounds: None,
                confidence: 0.,
                cells: vec![],
                stable_cells: 0,
                complete: false,
            });
        };
        let bounds = outline(&quads)?.bounds;
        if readings.len() != CELLS
            || readings.iter().any(|r| {
                !r.confidence.is_finite()
                    || !(0. ..=1.).contains(&r.confidence)
                    || !(r.text.is_empty()
                        || (r.text.len() == 1 && r.text.as_bytes()[0].is_ascii_uppercase()))
            })
        {
            return Err(MatrixScanError::InvalidFrame(
                "Expected 70 letters with finite confidence.".into(),
            ));
        }
        self.missing_frames = 0;
        // A cell's identity is its row/column, not its position in the preview.
        // Translation, perspective and approaching the camera retain its evidence.
        let cells: Vec<_> = self
            .evidence
            .iter_mut()
            .zip(&readings)
            .map(|(e, r)| e.add(r))
            .collect();
        let stable_cells = cells.iter().filter(|cell| cell.stable).count();
        let confidence = cells.iter().map(|c| c.confidence).fold(1., f64::min);
        Ok(MatrixScanSnapshot {
            bounds: Some(bounds),
            confidence,
            cells,
            stable_cells,
            complete: stable_cells == CELLS,
        })
    }
}

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    use ts_rs::TS;
    vec![
        MatrixPoint::decl(),
        MatrixReading::decl(),
        MatrixVote::decl(),
        MatrixScanSnapshot::decl(),
        MatrixScanOutline::decl(),
    ]
}

#[cfg(test)]
#[path = "tests/matrix_scan.rs"]
mod tests;
