//! Bounded camera acquisition followed by offline per-cell voting.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use super::*;

const GRAY_PIXELS: usize = 96 * 96;
const CAPTURE_LIMIT: Duration = Duration::from_secs(8);
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
// Three qualified votes are needed; retain room for weak model observations.
const CAPTURE_TARGET: usize = 5;
const MIN_CONTRAST: f64 = 12.;
// A 720p camera can leave a Matrix cell only 12–13 source pixels tall. The
// 96px rectification lowers its Laplacian score even when the glyph remains
// readable, so these gates reject blur without requiring a higher-resolution
// camera than the native shell provides.
const MIN_SHARPNESS: f64 = 10.;
const MIN_SOURCE_SIZE: f64 = 10.;
const MIN_CENTERING: f64 = 0.5;

/// Image capabilities report measurements; core decides admission and retention.
pub struct MatrixCaptureGlyph {
    pub gray: Vec<u8>,
    pub contrast: f64,
    pub sharpness: f64,
    pub source_size: f64,
    pub centering: f64,
}
pub struct MatrixCaptureFrame {
    pub quads: Vec<[MatrixPoint; 4]>,
    pub glyphs: Vec<MatrixCaptureGlyph>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum MatrixCameraPhase {
    Capturing,
    Processing,
    Complete,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MatrixCameraSnapshot {
    pub phase: MatrixCameraPhase,
    pub bounds: Option<[MatrixPoint; 4]>,
    pub ready_cells: usize,
    pub capture_elapsed_ms: f64,
    pub timed_out: bool,
    pub completed_samples: usize,
    pub total_samples: usize,
    pub cells: Vec<MatrixVote>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixCameraDiagnostics {
    pub phase: MatrixCameraPhase,
    pub capture_elapsed_ms: f64,
    pub processing_ms: f64,
    pub timed_out: bool,
    pub candidates: Vec<usize>,
    pub qualified_samples: Vec<usize>,
    pub stable_cells: usize,
    pub completed_samples: usize,
    pub total_samples: usize,
}
pub struct MatrixCameraBatch {
    revision: usize,
    pub cells: Vec<usize>,
    pub images: Vec<Vec<u8>>,
}
struct Candidate {
    quality: f64,
    gray: Vec<u8>,
}
struct Acquisition {
    phase: MatrixCameraPhase,
    first_matrix: Option<Instant>,
    sealed_at: Option<Instant>,
    finished_at: Option<Instant>,
    last_observation: Option<Instant>,
    last_samples: Vec<Option<Instant>>,
    pools: Vec<Vec<Candidate>>,
    counts: Vec<usize>,
    bounds: Option<[MatrixPoint; 4]>,
    timed_out: bool,
    work: VecDeque<(usize, Vec<u8>)>,
    total: usize,
    revision: usize,
    evidence: Vec<CellEvidence>,
}
impl Default for Acquisition {
    fn default() -> Self {
        Self {
            phase: MatrixCameraPhase::Capturing,
            first_matrix: None,
            sealed_at: None,
            finished_at: None,
            last_observation: None,
            last_samples: vec![None; CELLS],
            pools: (0..CELLS).map(|_| Vec::new()).collect(),
            counts: vec![0; CELLS],
            bounds: None,
            timed_out: false,
            work: VecDeque::new(),
            total: 0,
            revision: 0,
            evidence: vec![CellEvidence::default(); CELLS],
        }
    }
}
#[derive(Default)]
pub struct MatrixCameraScanner {
    active: Option<(String, Acquisition)>,
}
impl MatrixCameraScanner {
    pub fn begin(&mut self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.active = Some((id.clone(), Acquisition::default()));
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
    fn session(&self, id: &str) -> Result<&Acquisition, MatrixScanError> {
        self.active
            .as_ref()
            .filter(|(current, _)| current == id)
            .map(|(_, session)| session)
            .ok_or(MatrixScanError::Ended)
    }
    fn session_mut(&mut self, id: &str) -> Result<&mut Acquisition, MatrixScanError> {
        self.active
            .as_mut()
            .filter(|(current, _)| current == id)
            .map(|(_, session)| session)
            .ok_or(MatrixScanError::Ended)
    }
    /// `None` is a periodic wakeup or a frame without a located Matrix.
    /// The deadline starts at the first plausible Matrix, never at camera open.
    pub fn observe(
        &mut self,
        id: &str,
        frame: Option<MatrixCaptureFrame>,
        now: Instant,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let session = self.session_mut(id)?;
        if session.phase != MatrixCameraPhase::Capturing {
            return Ok(session.snapshot(now));
        }
        if session.last_observation.is_some_and(|last| now < last) {
            return Err(MatrixScanError::InvalidFrame(
                "Capture time moved backwards.".into(),
            ));
        }
        if let Some(frame) = &frame {
            outline(&frame.quads)?;
            if frame.glyphs.len() != CELLS
                || frame.glyphs.iter().any(|g| {
                    g.gray.len() != GRAY_PIXELS
                        || !(0. ..=1.).contains(&g.centering)
                        || [g.contrast, g.sharpness, g.source_size, g.centering]
                            .iter()
                            .any(|v| !v.is_finite() || *v < 0.)
                })
            {
                return Err(MatrixScanError::InvalidFrame(
                    "Expected 70 measured grayscale glyphs.".into(),
                ));
            }
        }
        session.last_observation = Some(now);
        if session
            .first_matrix
            .is_some_and(|first| now.duration_since(first) >= CAPTURE_LIMIT)
        {
            session.timed_out = true;
            session.seal(now);
            return Ok(session.snapshot(now));
        }
        if let Some(frame) = frame {
            let usable = |g: &MatrixCaptureGlyph| {
                g.contrast >= MIN_CONTRAST
                    && g.sharpness >= MIN_SHARPNESS
                    && g.source_size >= MIN_SOURCE_SIZE
                    && g.centering >= MIN_CENTERING
            };
            // Empty backgrounds can resemble a grid. Confirm character texture
            // before starting the clock; later admission is independent per cell.
            if session.first_matrix.is_none()
                && frame.glyphs.iter().filter(|g| usable(g)).count() < CELLS / 2
            {
                return Ok(session.snapshot(now));
            }
            session.first_matrix.get_or_insert(now);
            session.bounds = Some(outline(&frame.quads)?.bounds);
            for (i, glyph) in frame.glyphs.into_iter().enumerate() {
                if !usable(&glyph)
                    || session.last_samples[i]
                        .is_some_and(|last| now.duration_since(last) < SAMPLE_INTERVAL)
                {
                    continue;
                }
                session.last_samples[i] = Some(now);
                let pool = &mut session.pools[i];
                pool.push(Candidate {
                    // A cropped grid edge is sharp but belongs outside the
                    // glyph's center. Prefer centered texture over such edges.
                    quality: glyph.sharpness.sqrt() * glyph.centering.powi(4),
                    gray: glyph.gray,
                });
                pool.sort_by(|a, b| b.quality.total_cmp(&a.quality));
                pool.truncate(CELL_WINDOW);
                session.counts[i] = pool.len();
            }
            if session.counts.iter().all(|&count| count >= CAPTURE_TARGET) {
                session.seal(now);
            }
        } else {
            session.bounds = None;
        }
        Ok(session.snapshot(now))
    }
    pub fn batch(&self, id: &str) -> Result<Option<MatrixCameraBatch>, MatrixScanError> {
        let session = self.session(id)?;
        if session.phase == MatrixCameraPhase::Capturing {
            return Err(MatrixScanError::InvalidFrame(
                "Capture is not finished.".into(),
            ));
        }
        if session.phase != MatrixCameraPhase::Processing || session.work.is_empty() {
            return Ok(None);
        }
        let (cells, images) = session.work.iter().take(10).cloned().unzip();
        Ok(Some(MatrixCameraBatch {
            revision: session.revision,
            cells,
            images,
        }))
    }
    pub fn apply(
        &mut self,
        id: &str,
        batch: &MatrixCameraBatch,
        readings: Vec<MatrixReading>,
        now: Instant,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let session = self.session_mut(id)?;
        if session.phase != MatrixCameraPhase::Processing || batch.revision != session.revision {
            return Err(MatrixScanError::Ended);
        }
        if readings.len() != batch.cells.len()
            || batch.cells.is_empty()
            || readings.iter().any(|r| !valid_reading(r))
            || session.work.len().min(10) != batch.cells.len()
            || batch.images.len() != batch.cells.len()
            || session
                .work
                .iter()
                .take(10)
                .zip(batch.cells.iter().zip(&batch.images))
                .any(|((index, image), (actual, gray))| index != actual || image != gray)
        {
            return Err(MatrixScanError::InvalidFrame(
                "Candidate batch was modified.".into(),
            ));
        }
        for (&cell, reading) in batch.cells.iter().zip(readings) {
            // Offline samples have no chronological "new card" event. All
            // candidates contribute to one weighted vote, regardless of order.
            session.evidence[cell].collect(reading, session.revision, false);
            session.work.pop_front();
        }
        session.revision += 1;
        if session.work.is_empty() {
            session.phase = MatrixCameraPhase::Complete;
            session.finished_at = Some(now);
        }
        Ok(session.snapshot(now))
    }
    pub fn diagnostics(
        &self,
        id: &str,
        now: Instant,
    ) -> Result<MatrixCameraDiagnostics, MatrixScanError> {
        let session = self.session(id)?;
        let snapshot = session.snapshot(now);
        Ok(MatrixCameraDiagnostics {
            phase: snapshot.phase,
            capture_elapsed_ms: snapshot.capture_elapsed_ms,
            processing_ms: session.sealed_at.map_or(0., |start| {
                session
                    .finished_at
                    .unwrap_or(now)
                    .saturating_duration_since(start)
                    .as_secs_f64()
                    * 1000.
            }),
            timed_out: session.timed_out,
            candidates: session.counts.clone(),
            qualified_samples: session.evidence.iter().map(|e| e.samples.len()).collect(),
            stable_cells: snapshot.cells.iter().filter(|c| c.stable).count(),
            completed_samples: snapshot.completed_samples,
            total_samples: session.total,
        })
    }
}
impl Acquisition {
    fn seal(&mut self, now: Instant) {
        self.sealed_at = Some(now);
        // Round-robin keeps progress distributed across cells.
        for _ in 0..CELL_WINDOW {
            for (cell, pool) in self.pools.iter_mut().enumerate() {
                if !pool.is_empty() {
                    self.work.push_back((cell, pool.remove(0).gray));
                }
            }
        }
        self.total = self.work.len();
        self.phase = if self.total == 0 {
            MatrixCameraPhase::Complete
        } else {
            MatrixCameraPhase::Processing
        };
        if self.total == 0 {
            self.finished_at = Some(now);
        }
    }
    fn snapshot(&self, now: Instant) -> MatrixCameraSnapshot {
        MatrixCameraSnapshot {
            phase: self.phase,
            bounds: self.bounds,
            ready_cells: self
                .counts
                .iter()
                .filter(|&&count| count >= CAPTURE_TARGET)
                .count(),
            capture_elapsed_ms: self.first_matrix.map_or(0., |start| {
                self.sealed_at
                    .unwrap_or(now)
                    .saturating_duration_since(start)
                    .as_secs_f64()
                    * 1000.
            }),
            timed_out: self.timed_out,
            completed_samples: self.total - self.work.len(),
            total_samples: self.total,
            cells: if self.phase == MatrixCameraPhase::Capturing {
                vec![]
            } else {
                self.evidence.iter().map(CellEvidence::vote).collect()
            },
        }
    }
}
#[cfg(test)]
#[path = "../tests/matrix_scan__capture.rs"]
mod tests;
