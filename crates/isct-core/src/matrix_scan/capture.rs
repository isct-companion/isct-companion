//! Bounded streaming acquisition and per-cell microbatch dispatch.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use super::*;
const GRAY_PIXELS: usize = 96 * 96;
const CAPTURE_LIMIT: Duration = Duration::from_secs(7);
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
const CANDIDATE_TTL: Duration = Duration::from_millis(700);
const AUDIT_INTERVAL: Duration = Duration::from_millis(500);
const POOL_LIMIT: usize = 2;
const MIN_CONTRAST: f64 = 12.;
const MIN_SHARPNESS: f64 = 10.;
const MIN_SOURCE_SIZE: f64 = 10.;
const MIN_CENTERING: f64 = 0.5;
const MAX_GLARE: f64 = 0.12;
const TRACE_LIMIT: usize = 1024;
/// Image capabilities report measurements; core decides admission and retention.
pub struct MatrixCaptureGlyph {
    pub gray: Vec<u8>,
    pub contrast: f64,
    pub sharpness: f64,
    pub source_size: f64,
    pub centering: f64,
    pub glare: f64,
}
pub struct MatrixCaptureFrame {
    /// Shell-assigned camera frame id, echoed in snapshots beside its bounds.
    pub frame_id: u32,
    pub quads: Vec<[MatrixPoint; 4]>,
    pub glyphs: Vec<MatrixCaptureGlyph>,
}
/// One frame of the fast geometry loop: the grid to draw on this frame and
/// whether this frame's fit is good enough to cut cells from.
pub struct MatrixTrackedFrame {
    pub frame_id: u32,
    pub bounds: Option<[MatrixPoint; 4]>,
    pub keyframe_ready: bool,
    pub analysis_ms: f64,
}
/// Diagnostics for one tracked frame. `observed_ms` is Core's session clock;
/// `keyframe` marks frames whose cells were later observed.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatrixFrameTrace {
    pub frame_id: u32,
    pub located: bool,
    pub analysis_ms: f64,
    pub observed_ms: f64,
    pub keyframe: bool,
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
    /// Increases with every traced or observed frame; older replies are stale.
    pub sequence: u32,
    pub bounds: Option<[MatrixPoint; 4]>,
    /// The camera frame `bounds` were measured on.
    pub bounds_frame_id: Option<u32>,
    /// Core wants the cells of this tracked frame now (it qualified and the
    /// sampling interval has passed); the shell sends its full resolution.
    pub keyframe_frame_id: Option<u32>,
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
    pub keyframes: usize,
    pub admission_rejections: usize,
    pub glare_rejections: Vec<usize>,
    pub frames: Vec<MatrixFrameTrace>,
}

#[derive(Clone)]
pub struct MatrixCameraBatch {
    owner: String,
    revision: usize,
    pub cells: Vec<usize>,
    pub images: Vec<Vec<u8>>,
}
struct Candidate {
    quality: f64,
    captured: Instant,
    gray: Vec<u8>,
}
struct Lease {
    batch: MatrixCameraBatch,
    captured: Vec<Instant>,
    dispatched: Instant,
}
struct Acquisition {
    started: Instant,
    limit: Duration,
    finished: Option<Instant>,
    last_observation: Option<Instant>,
    last_geometry: Option<Instant>,
    processing_time: Duration,
    last_samples: Vec<Option<Instant>>,
    last_dispatch: Vec<Option<Instant>>,
    pools: Vec<Vec<Candidate>>,
    bounds: Option<([MatrixPoint; 4], u32)>,
    sequence: u32,
    traces: Vec<MatrixFrameTrace>,
    keyframe_request: Option<u32>,
    last_keyframe: Option<Instant>,
    admitted: bool,
    timed_out: bool,
    lease: Option<Lease>,
    completed: usize,
    revision: usize,
    evidence: Vec<CellEvidence>,
    verifying_since: Option<Instant>,
    verified: Vec<bool>,
    keyframes: usize,
    admission_rejections: usize,
    glare_rejections: Vec<usize>,
}
impl Acquisition {
    fn new(started: Instant, limit: Duration) -> Self {
        Self {
            started,
            limit,
            finished: None,
            last_observation: None,
            last_geometry: None,
            processing_time: Duration::ZERO,
            last_samples: vec![None; CELLS],
            last_dispatch: vec![None; CELLS],
            pools: (0..CELLS).map(|_| Vec::new()).collect(),
            bounds: None,
            sequence: 0,
            traces: Vec::new(),
            keyframe_request: None,
            last_keyframe: None,
            admitted: false,
            timed_out: false,
            lease: None,
            completed: 0,
            revision: 0,
            evidence: vec![CellEvidence::default(); CELLS],
            verifying_since: None,
            verified: vec![false; CELLS],
            keyframes: 0,
            admission_rejections: 0,
            glare_rejections: vec![0; CELLS],
        }
    }
    fn expire(&mut self, now: Instant) {
        if self.finished.is_none() && now.saturating_duration_since(self.started) >= self.limit {
            self.timed_out = true;
            self.finish(self.started + self.limit);
        }
        for pool in &mut self.pools {
            pool.retain(|c| now.saturating_duration_since(c.captured) <= CANDIDATE_TTL);
        }
    }
    fn finish(&mut self, now: Instant) {
        self.finished = Some(now);
        if let Some(lease) = self.lease.take() {
            self.processing_time += now.saturating_duration_since(lease.dispatched);
        }
        self.pools.iter_mut().for_each(Vec::clear);
    }
    fn fresh_bounds(&self, now: Instant) -> Option<([MatrixPoint; 4], u32)> {
        self.last_geometry
            .filter(|&last| now.saturating_duration_since(last) < Duration::from_millis(300))
            .and(self.bounds)
    }
    fn snapshot(&self, now: Instant) -> MatrixCameraSnapshot {
        let cells: Vec<_> = self.evidence.iter().map(CellEvidence::vote).collect();
        MatrixCameraSnapshot {
            phase: if self.finished.is_some() {
                MatrixCameraPhase::Complete
            } else {
                MatrixCameraPhase::Capturing
            },
            sequence: self.sequence,
            bounds: self.fresh_bounds(now).map(|(b, _)| b),
            bounds_frame_id: self.fresh_bounds(now).map(|(_, f)| f),
            keyframe_frame_id: self.keyframe_request.filter(|_| self.finished.is_none()),
            ready_cells: cells.iter().filter(|v| v.stable).count(),
            capture_elapsed_ms: self
                .finished
                .unwrap_or(now)
                .saturating_duration_since(self.started)
                .as_secs_f64()
                * 1000.,
            timed_out: self.timed_out,
            completed_samples: self.completed,
            total_samples: self.completed
                + self.pools.iter().map(Vec::len).sum::<usize>()
                + self.lease.as_ref().map_or(0, |l| l.batch.cells.len()),
            cells,
        }
    }
}
fn validate(
    quads: &[[MatrixPoint; 4]],
    glyphs: &[MatrixCaptureGlyph],
) -> Result<(), MatrixScanError> {
    outline(quads)?;
    if glyphs.len() != CELLS
        || glyphs.iter().any(|g| {
            g.gray.len() != GRAY_PIXELS
                || !(0. ..=1.).contains(&g.centering)
                || !(0. ..=1.).contains(&g.glare)
                || [g.contrast, g.sharpness, g.source_size]
                    .iter()
                    .any(|v| !v.is_finite() || *v < 0.)
        })
    {
        return Err(MatrixScanError::InvalidFrame(
            "Expected 70 measured grayscale glyphs.".into(),
        ));
    }
    Ok(())
}
fn usable(g: &MatrixCaptureGlyph) -> bool {
    g.contrast >= MIN_CONTRAST
        && g.sharpness >= MIN_SHARPNESS
        && g.source_size >= MIN_SOURCE_SIZE
        && g.centering >= MIN_CENTERING
}
/// Empty backgrounds can resemble a grid: a first frame needs character
/// texture in half of its cells before anything it shows counts.
fn admits(glyphs: &[MatrixCaptureGlyph]) -> bool {
    glyphs.iter().filter(|g| usable(g)).count() >= CELLS / 2
}
/// A still photo is one keyframe of a session that never sees a second:
/// the same validation, admission, glare gate and evidence vote as a live
/// frame. Rejected cells stay blank for review; one reading cannot become
/// `stable`, so every photo cell is checked against the card.
pub fn read_matrix_photo(
    cut: MatrixCut,
    mut recognize: impl FnMut(&[Vec<u8>]) -> Result<Vec<MatrixReading>, MatrixScanError>,
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Vec<MatrixVote>, MatrixScanError> {
    validate(&cut.quads, &cut.glyphs)?;
    if !admits(&cut.glyphs) {
        return Err(MatrixScanError::Unreadable(
            "The letters are too small or blurred. Take a closer, focused photo of the grid."
                .into(),
        ));
    }
    let admitted: Vec<_> = cut
        .glyphs
        .into_iter()
        .enumerate()
        .filter(|(_, g)| g.glare <= MAX_GLARE && usable(g))
        .collect();
    let mut evidence = vec![CellEvidence::default(); CELLS];
    // Progress counts settled cells out of 70; rejected cells settle at once.
    let mut done = CELLS - admitted.len();
    if !progress(done, CELLS) {
        return Err(MatrixScanError::Ended);
    }
    for chunk in admitted.chunks(10) {
        let images: Vec<_> = chunk.iter().map(|(_, g)| g.gray.clone()).collect();
        let readings = recognize(&images)?;
        if readings.len() != chunk.len() || readings.iter().any(|r| !valid_reading(r)) {
            return Err(MatrixScanError::Backend(
                "The recognizer returned an invalid batch.".into(),
            ));
        }
        for ((cell, _), reading) in chunk.iter().zip(readings) {
            evidence[*cell].collect(reading, 0, false);
        }
        done += chunk.len();
        if !progress(done, CELLS) {
            return Err(MatrixScanError::Ended);
        }
    }
    Ok(evidence.iter().map(CellEvidence::vote).collect())
}
#[derive(Default)]
pub struct MatrixCameraScanner {
    active: Option<(String, Acquisition)>,
}
impl MatrixCameraScanner {
    pub fn begin(&mut self) -> String {
        self.begin_at(Instant::now(), CAPTURE_LIMIT)
    }
    /// Explicit clock and budget let replay use the same dispatcher as the app.
    pub fn begin_at(&mut self, now: Instant, limit: Duration) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.active = Some((id.clone(), Acquisition::new(now, limit.min(CAPTURE_LIMIT))));
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
            .map(|(_, s)| s)
            .ok_or(MatrixScanError::Ended)
    }
    fn session_mut(&mut self, id: &str) -> Result<&mut Acquisition, MatrixScanError> {
        self.active
            .as_mut()
            .filter(|(current, _)| current == id)
            .map(|(_, s)| s)
            .ok_or(MatrixScanError::Ended)
    }
    /// Periodic wakeups (`None`) advance the deadline even without any geometry.
    pub fn observe(
        &mut self,
        id: &str,
        frame: Option<MatrixCaptureFrame>,
        now: Instant,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let s = self.session_mut(id)?;
        if s.finished.is_some() {
            return Ok(s.snapshot(now));
        }
        if now < s.started || s.last_observation.is_some_and(|last| now < last) {
            return Err(MatrixScanError::InvalidFrame(
                "Capture time moved backwards.".into(),
            ));
        }
        if let Some(f) = &frame {
            validate(&f.quads, &f.glyphs)?;
        }
        s.last_observation = Some(now);
        s.expire(now);
        if s.finished.is_some() {
            return Ok(s.snapshot(now));
        }
        if let Some(frame) = frame {
            s.sequence += 1;
            s.keyframes += 1;
            s.last_keyframe = Some(now);
            s.keyframe_request = None;
            if let Some(trace) = s
                .traces
                .iter_mut()
                .rev()
                .find(|t| t.frame_id == frame.frame_id)
            {
                trace.keyframe = true;
            }
            if !s.admitted && !admits(&frame.glyphs) {
                s.admission_rejections += 1;
                return Ok(s.snapshot(now));
            }
            s.admitted = true;
            for (i, g) in frame.glyphs.into_iter().enumerate() {
                if g.glare > MAX_GLARE {
                    s.glare_rejections[i] += 1;
                    continue;
                }
                if !usable(&g)
                    || s.last_samples[i]
                        .is_some_and(|last| now.saturating_duration_since(last) < SAMPLE_INTERVAL)
                {
                    continue;
                }
                s.last_samples[i] = Some(now);
                let pool = &mut s.pools[i];
                pool.push(Candidate {
                    quality: g.sharpness.sqrt() * g.centering.powi(4) * (1. - g.glare),
                    captured: now,
                    gray: g.gray,
                });
                pool.sort_by(|a, b| {
                    b.quality
                        .total_cmp(&a.quality)
                        .then(b.captured.cmp(&a.captured))
                });
                pool.truncate(POOL_LIMIT);
            }
        }
        Ok(s.snapshot(now))
    }
    /// One frame of the fast loop. Its bounds become the drawn outline, and a
    /// frame that qualifies is requested as a keyframe once the sampling
    /// interval has passed since the last one. Located or not, it is traced.
    pub fn track(
        &mut self,
        id: &str,
        frame: MatrixTrackedFrame,
        now: Instant,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let s = self.session_mut(id)?;
        if s.finished.is_some() {
            return Ok(s.snapshot(now));
        }
        if now < s.started || s.last_observation.is_some_and(|last| now < last) {
            return Err(MatrixScanError::InvalidFrame(
                "Capture time moved backwards.".into(),
            ));
        }
        if frame.bounds.is_some_and(|b| {
            b.iter()
                .any(|p| !(0. ..=1.).contains(&p.x) || !(0. ..=1.).contains(&p.y))
        }) || !frame.analysis_ms.is_finite()
        {
            return Err(MatrixScanError::InvalidFrame(
                "Expected normalized bounds.".into(),
            ));
        }
        s.last_observation = Some(now);
        s.expire(now);
        if s.finished.is_some() {
            return Ok(s.snapshot(now));
        }
        s.sequence += 1;
        if let Some(bounds) = frame.bounds {
            s.bounds = Some((bounds, frame.frame_id));
            s.last_geometry = Some(now);
        }
        let due = s
            .last_keyframe
            .is_none_or(|last| now.saturating_duration_since(last) >= SAMPLE_INTERVAL);
        s.keyframe_request = (frame.keyframe_ready && due).then_some(frame.frame_id);
        if s.traces.len() < TRACE_LIMIT {
            s.traces.push(MatrixFrameTrace {
                frame_id: frame.frame_id,
                located: frame.bounds.is_some(),
                analysis_ms: frame.analysis_ms,
                observed_ms: now.saturating_duration_since(s.started).as_secs_f64() * 1000.,
                keyframe: false,
            });
        }
        Ok(s.snapshot(now))
    }
    /// Reserves one batch. A second consumer cannot process the same observations.
    pub fn batch(
        &mut self,
        id: &str,
        now: Instant,
    ) -> Result<Option<MatrixCameraBatch>, MatrixScanError> {
        let s = self.session_mut(id)?;
        s.expire(now);
        if s.finished.is_some() || s.lease.is_some() {
            return Ok(None);
        }
        let mut eligible: Vec<_> = (0..CELLS)
            .filter(|&i| {
                !s.pools[i].is_empty()
                    && (!s.evidence[i].vote().stable
                        || s.verifying_since
                            .is_some_and(|start| !s.verified[i] && s.pools[i][0].captured >= start)
                        || s.last_dispatch[i].is_none_or(|last| {
                            now.saturating_duration_since(last) >= AUDIT_INTERVAL
                        }))
            })
            .collect();
        eligible.sort_by_key(|&i| {
            (
                s.evidence[i].vote().stable,
                s.evidence[i].attempts,
                s.last_dispatch[i],
                i,
            )
        });
        eligible.truncate(10);
        if eligible.is_empty() {
            return Ok(None);
        }
        let mut images = Vec::new();
        let mut captured = Vec::new();
        for &i in &eligible {
            let candidate = s.pools[i].remove(0);
            captured.push(candidate.captured);
            images.push(candidate.gray);
            // Older observations must never arrive after a newer one for a cell.
            s.pools[i].retain(|c| c.captured > candidate.captured);
            s.last_dispatch[i] = Some(now);
        }
        s.revision += 1;
        let batch = MatrixCameraBatch {
            owner: id.into(),
            revision: s.revision,
            cells: eligible,
            images,
        };
        s.lease = Some(Lease {
            batch: batch.clone(),
            captured,
            dispatched: now,
        });
        Ok(Some(batch))
    }
    pub fn apply(
        &mut self,
        id: &str,
        batch: &MatrixCameraBatch,
        readings: Vec<MatrixReading>,
        now: Instant,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let s = self.session_mut(id)?;
        if batch.owner != id {
            return Err(MatrixScanError::Ended);
        }
        s.expire(now);
        // Results finishing after the deadline never change the frozen outcome.
        if s.finished.is_some() {
            return Ok(s.snapshot(now));
        }
        let Some(lease) = &s.lease else {
            return Err(MatrixScanError::Ended);
        };
        if batch.revision != lease.batch.revision {
            return Err(MatrixScanError::Ended);
        }
        if readings.len() != batch.cells.len()
            || readings.iter().any(|r| !valid_reading(r))
            || batch.cells != lease.batch.cells
            || batch.images != lease.batch.images
        {
            return Err(MatrixScanError::InvalidFrame(
                "Candidate batch was modified.".into(),
            ));
        }
        let captured = lease.captured.clone();
        s.processing_time += now.saturating_duration_since(lease.dispatched);
        s.lease = None;
        for ((&cell, reading), captured) in batch.cells.iter().zip(readings).zip(captured) {
            let before = s.evidence[cell].vote();
            let confirms = reading.confidence >= HIGH_CONFIDENCE && reading.text == before.text;
            s.evidence[cell].collect(reading, s.revision, true);
            let after = s.evidence[cell].vote();
            if before.stable && (!after.stable || before.text != after.text) {
                s.verifying_since = None;
                s.verified.fill(false);
            }
            if s.verifying_since.is_some_and(|start| captured >= start) && after.stable && confirms
            {
                s.verified[cell] = true;
            }
            s.completed += 1;
        }
        if s.evidence.iter().all(|e| e.vote().stable) {
            if s.verifying_since.is_none() {
                s.verifying_since = Some(now);
                s.verified.fill(false);
                // Require a fresh observation of every cell after consensus.
                s.pools.iter_mut().for_each(Vec::clear);
            } else if s.verified.iter().all(|&v| v) {
                s.finish(now);
            }
        }
        Ok(s.snapshot(now))
    }
    pub fn diagnostics(
        &self,
        id: &str,
        now: Instant,
    ) -> Result<MatrixCameraDiagnostics, MatrixScanError> {
        let s = self.session(id)?;
        let snapshot = s.snapshot(now);
        Ok(MatrixCameraDiagnostics {
            phase: snapshot.phase,
            capture_elapsed_ms: snapshot.capture_elapsed_ms,
            processing_ms: (s.processing_time
                + s.lease.as_ref().map_or(Duration::ZERO, |l| {
                    now.saturating_duration_since(l.dispatched)
                }))
            .as_secs_f64()
                * 1000.,
            timed_out: s.timed_out,
            candidates: s.pools.iter().map(Vec::len).collect(),
            qualified_samples: s.evidence.iter().map(|e| e.samples.len()).collect(),
            stable_cells: snapshot.ready_cells,
            completed_samples: s.completed,
            total_samples: snapshot.total_samples,
            keyframes: s.keyframes,
            admission_rejections: s.admission_rejections,
            glare_rejections: s.glare_rejections.clone(),
            frames: s.traces.clone(),
        })
    }
}
#[cfg(test)]
#[path = "../tests/matrix_scan__capture.rs"]
mod tests;
