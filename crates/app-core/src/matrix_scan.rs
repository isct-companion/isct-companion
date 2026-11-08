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
#[derive(Clone, Default)]
struct CellEvidence {
    samples: VecDeque<MatrixReading>,
    attempts: usize,
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
    fn collect(&mut self, reading: MatrixReading, reopen: bool) {
        self.attempts += 1;
        if reading.text.is_empty() || reading.confidence < HIGH_CONFIDENCE {
            return;
        }
        let previous = self.vote();
        // A new clear contradiction reopens a settled cell instead of allowing
        // its old majority to hide a changed card or an earlier wrong result.
        if reopen && previous.stable && previous.text != reading.text {
            self.samples.clear();
        }
        if self.samples.len() == CELL_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(reading);
    }
}
#[cfg(feature = "ts")]
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        MatrixPoint::decl(cfg),
        MatrixReading::decl(cfg),
        MatrixVote::decl(cfg),
        MatrixConfidence::decl(cfg),
        MatrixScanOutline::decl(cfg),
        MatrixCaptureMetrics::decl(cfg),
        MatrixFrameTimeline::decl(cfg),
        MatrixCameraPhase::decl(cfg),
        MatrixCameraSnapshot::decl(cfg),
    ]
}
#[cfg(test)]
#[path = "tests/matrix_scan.rs"]
mod tests;
