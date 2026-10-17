//! Bounded, confidence-weighted consensus for a live matrix-card scan.
//! Camera handles and OCR engines stay in their platform/capability adapters.
use serde::Serialize;
use std::collections::VecDeque;
use thiserror::Error;

pub const CELLS: usize = 70;
const WINDOW: usize = 8;

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
    /// Mean winning confidence multiplied by its share of all confidence mass.
    pub confidence: f64,
    pub stable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixScanSnapshot {
    /// Normalized coordinates in the unmirrored camera image, in row order.
    pub quads: Vec<[MatrixPoint; 4]>,
    pub cells: Vec<MatrixVote>,
    pub stable_cells: usize,
    pub complete: bool,
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

#[derive(Default)]
struct Consensus {
    history: VecDeque<Vec<MatrixReading>>,
    previous: Vec<[MatrixPoint; 4]>,
}
impl Consensus {
    fn observe(
        &mut self,
        frame: Option<(Vec<[MatrixPoint; 4]>, Vec<MatrixReading>)>,
    ) -> Result<MatrixScanSnapshot, MatrixScanError> {
        let Some((quads, readings)) = frame else {
            // A lost card must not donate votes to the next card.
            self.history.clear();
            self.previous.clear();
            return Ok(MatrixScanSnapshot {
                quads: vec![],
                cells: vec![],
                stable_cells: 0,
                complete: false,
            });
        };
        if quads.len() != CELLS
            || readings.len() != CELLS
            || quads.iter().flatten().any(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || !(0. ..=1.).contains(&p.x)
                    || !(0. ..=1.).contains(&p.y)
            })
            || readings.iter().any(|r| {
                !r.confidence.is_finite()
                    || !(0. ..=1.).contains(&r.confidence)
                    || !(r.text.is_empty()
                        || (r.text.len() == 1 && r.text.as_bytes()[0].is_ascii_uppercase()))
            })
        {
            return Err(MatrixScanError::InvalidFrame(
                "Expected 70 cells with normalized geometry and finite confidence.".into(),
            ));
        }
        // Large camera/card movement breaks temporal correspondence.
        if self.previous.iter().zip(&quads).any(|(a, b)| {
            a.iter()
                .zip(b)
                .any(|(a, b)| (a.x - b.x).hypot(a.y - b.y) > 0.08)
        }) {
            self.history.clear();
        }
        self.previous = quads.clone();
        self.history.push_back(readings);
        if self.history.len() > WINDOW {
            self.history.pop_front();
        }
        let cells: Vec<_> = (0..CELLS)
            .map(|index| {
                let mut weights = [0.; 26];
                let mut counts = [0usize; 26];
                for frame in &self.history {
                    let reading = &frame[index];
                    if reading.text.len() == 1 && reading.confidence >= 0.65 {
                        let letter = (reading.text.as_bytes()[0] - b'A') as usize;
                        weights[letter] += reading.confidence;
                        counts[letter] += 1;
                    }
                }
                let winner = (0..26)
                    .max_by(|&a, &b| weights[a].total_cmp(&weights[b]))
                    .unwrap_or(0);
                let total: f64 = weights.iter().sum();
                let mean = if counts[winner] > 0 {
                    weights[winner] / counts[winner] as f64
                } else {
                    0.
                };
                let share = if total > 0. {
                    weights[winner] / total
                } else {
                    0.
                };
                let text = if total > 0. {
                    ((b'A' + winner as u8) as char).to_string()
                } else {
                    String::new()
                };
                let latest_agrees = self.history.back().is_some_and(|frame| {
                    frame[index].text == text && frame[index].confidence >= 0.75
                });
                MatrixVote {
                    text,
                    confidence: mean * share,
                    stable: counts[winner] >= 3 && mean >= 0.9 && share >= 0.85 && latest_agrees,
                }
            })
            .collect();
        let stable_cells = cells.iter().filter(|cell| cell.stable).count();
        Ok(MatrixScanSnapshot {
            quads,
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
    ]
}

#[cfg(test)]
#[path = "tests/matrix_scan.rs"]
mod tests;
