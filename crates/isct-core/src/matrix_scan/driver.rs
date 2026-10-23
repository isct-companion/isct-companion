//! The one driver of a Matrix scan. Every shell feeds it frames (desktop:
//! analysis planes, then a requested keyframe; phones: one YUV frame) or an
//! encoded photo, and renders the snapshots it returns. The tracker ↔ Core
//! hand-off, the pending keyframe and the OCR consumer live here only.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
use super::*;
use std::sync::{Arc, Mutex};

type Inspector = Box<dyn Fn(&MatrixCut) + Send + Sync>;

/// Image-side state of the live session: its tracker and the keyframe Core
/// asked for (frame id and the grid fitted on that frame).
#[derive(Default)]
struct Geometry {
    session: String,
    tracker: Option<Box<dyn MatrixGridTracker>>,
    keyframe: Option<(u32, [MatrixPoint; 4])>,
}

/// A photo's readings in row order (A1…J1, A2…) and where the grid was found.
pub struct MatrixPhotoScan {
    pub cells: Vec<MatrixVote>,
    pub bounds: [MatrixPoint; 4],
}

pub struct MatrixCameraDriver {
    vision: Arc<dyn MatrixVision>,
    scanner: Mutex<MatrixCameraScanner>,
    geometry: Mutex<Geometry>,
    inspector: Option<Inspector>,
}
fn unavailable() -> MatrixScanError {
    MatrixScanError::Backend("Camera scanner state unavailable. Restart the app.".into())
}
impl MatrixCameraDriver {
    pub fn new(vision: Arc<dyn MatrixVision>) -> Self {
        Self {
            vision,
            scanner: Mutex::default(),
            geometry: Mutex::default(),
            inspector: None,
        }
    }
    /// Debug tooling sees every cut keyframe (numeric measurements only).
    pub fn with_inspector(
        mut self,
        inspector: impl Fn(&MatrixCut) + Send + Sync + 'static,
    ) -> Self {
        self.inspector = Some(Box::new(inspector));
        self
    }
    pub fn available(&self) -> bool {
        self.vision.available()
    }
    pub fn warm_up(&self) -> Result<(), MatrixScanError> {
        self.vision.warm_up()
    }
    fn with<T>(&self, f: impl FnOnce(&mut MatrixCameraScanner) -> T) -> Result<T, MatrixScanError> {
        self.scanner
            .lock()
            .map(|mut scanner| f(&mut scanner))
            .map_err(|_| unavailable())
    }
    fn geometry<T>(&self, f: impl FnOnce(&mut Geometry) -> T) -> Result<T, MatrixScanError> {
        self.geometry
            .lock()
            .map(|mut geometry| f(&mut geometry))
            .map_err(|_| unavailable())
    }

    /// Starts a session; its seven-second budget starts now.
    pub fn begin(&self) -> Result<String, MatrixScanError> {
        if !self.vision.available() {
            return Err(MatrixScanError::Backend(
                "Matrix scanner is unavailable in this build.".into(),
            ));
        }
        let id = self.with(|s| s.begin())?;
        let tracker = self.vision.tracker();
        self.geometry(|g| {
            *g = Geometry {
                session: id.clone(),
                tracker: Some(tracker),
                keyframe: None,
            }
        })?;
        Ok(id)
    }
    /// Ends the session and returns its aggregate diagnostics, if it existed.
    pub fn stop(&self, id: &str) -> Option<MatrixCameraDiagnostics> {
        let diagnostics = self
            .with(|s| {
                let report = s.diagnostics(id, Instant::now()).ok();
                s.stop(id);
                report
            })
            .ok()
            .flatten();
        let _ = self.geometry(|g| {
            if g.session == id {
                *g = Geometry::default();
            }
        });
        diagnostics
    }
    /// A lifecycle wakeup: the deadline advances even when video stalls.
    pub fn poll(&self, id: &str) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        self.with(|s| s.observe(id, None, Instant::now()))?
    }

    /// The fast loop: one frame's analysis planes. The snapshot carries the
    /// outline fitted on this very frame and, when Core wants its cells,
    /// `keyframe_frame_id == Some(frame_id)`.
    pub fn track(
        &self,
        id: &str,
        frame_id: u32,
        planes: MatrixPlanes<'_>,
        captured_ms: f64,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let current = self.poll(id)?;
        if current.phase != MatrixCameraPhase::Capturing {
            return Ok(current);
        }
        let track = self.geometry(|g| {
            if g.session != id {
                return Err(MatrixScanError::Ended);
            }
            g.tracker
                .as_mut()
                .ok_or(MatrixScanError::Ended)?
                .track(planes, captured_ms / 1000.)
        })??;
        let snapshot = self.with(|s| {
            s.track(
                id,
                MatrixTrackedFrame {
                    frame_id,
                    bounds: track.display,
                    keyframe_ready: track.keyframe,
                    analysis_ms: track.elapsed_ms,
                },
                Instant::now(),
            )
        })??;
        let wanted = snapshot.keyframe_frame_id == Some(frame_id);
        self.geometry(|g| {
            if g.session == id {
                g.keyframe = track.body.filter(|_| wanted).map(|body| (frame_id, body));
            }
        })?;
        Ok(snapshot)
    }
    /// The requested keyframe: the full-resolution luma of the frame named in
    /// the last snapshot's `keyframe_frame_id`, cut with the grid fitted on it.
    /// A keyframe superseded by a newer request is dropped, not cut.
    pub fn keyframe(
        &self,
        id: &str,
        frame_id: u32,
        luma: MatrixLuma<'_>,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        // A late reply for an older frame leaves a newer request pending.
        let grid = self.geometry(|g| match g.keyframe {
            Some((pending, body)) if g.session == id && pending == frame_id => {
                g.keyframe = None;
                Some(body)
            }
            _ => None,
        })?;
        let Some(body) = grid else {
            return self.poll(id);
        };
        let cut = self.vision.cut(luma, body)?;
        if let Some(inspect) = &self.inspector {
            inspect(&cut);
        }
        let frame = MatrixCaptureFrame {
            frame_id,
            quads: cut.quads,
            glyphs: cut.glyphs,
        };
        self.with(|s| s.observe(id, Some(frame), Instant::now()))?
    }
    /// A native camera frame: tracked and, when Core asks for it, cut at once
    /// from the full resolution this call already holds.
    pub fn submit(
        &self,
        id: &str,
        frame_id: u32,
        frame: &MatrixYuvFrame<'_>,
        captured_ms: f64,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let current = self.poll(id)?;
        if current.phase != MatrixCameraPhase::Capturing {
            return Ok(current);
        }
        let picture = self.vision.convert(frame)?;
        let snapshot = self.track(id, frame_id, picture.planes(), captured_ms)?;
        if snapshot.keyframe_frame_id == Some(frame_id) {
            return self.keyframe(id, frame_id, picture.full());
        }
        Ok(snapshot)
    }

    /// The consumer: reserves microbatches, runs the model and applies the
    /// readings until the session completes, stops or fails. It runs beside
    /// acquisition on its own thread; `progress` returning false ends it.
    pub fn process(
        &self,
        id: &str,
        mut progress: impl FnMut(&MatrixCameraSnapshot) -> bool,
    ) -> Result<MatrixCameraSnapshot, MatrixScanError> {
        let result = (|| loop {
            let current = self.poll(id)?;
            if current.phase == MatrixCameraPhase::Complete {
                return Ok(current);
            }
            let Some(batch) = self.with(|s| s.batch(id, Instant::now()))?? else {
                // The session deadline bounds this wait; no lock is held.
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            let readings = self.vision.recognize(&batch.images)?;
            let snapshot = self.with(|s| s.apply(id, &batch, readings, Instant::now()))??;
            if snapshot.phase == MatrixCameraPhase::Complete {
                return Ok(snapshot);
            }
            if !progress(&snapshot) {
                return Err(MatrixScanError::Ended);
            }
        })();
        if result.is_err() {
            self.with(|s| s.stop(id))?;
        }
        result
    }

    /// A still photo, read as one frame of a live session: a fresh tracker
    /// finds the grid, the same cut and admission apply, and each admitted
    /// cell gets one reading through the same evidence vote.
    pub fn scan_photo(
        &self,
        encoded: &[u8],
        progress: impl FnMut(usize, usize) -> bool,
    ) -> Result<MatrixPhotoScan, MatrixScanError> {
        let picture = self.vision.decode(encoded)?;
        let track = self.vision.tracker().track(picture.planes(), 0.)?;
        let body = track.body.ok_or_else(|| {
            MatrixScanError::Unreadable(
                "Could not locate all 7 × 10 cells. Include the A–J header and 1–7 row labels in a focused photo."
                    .into(),
            )
        })?;
        let cut = self.vision.cut(picture.full(), body)?;
        let cells = read_matrix_photo(cut, |images| self.vision.recognize(images), progress)?;
        Ok(MatrixPhotoScan {
            cells,
            bounds: body,
        })
    }
}
#[cfg(test)]
#[path = "../tests/matrix_scan__driver.rs"]
mod tests;
