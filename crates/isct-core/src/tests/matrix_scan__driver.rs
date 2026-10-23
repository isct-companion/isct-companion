use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

// The real tracker and model need pixels and ONNX; isct-ocr's tests drive
// this same driver with them. Here a scripted capability isolates what the
// driver itself decides: hand-off, keyframe ownership, admission and votes.
const BODY: [MatrixPoint; 4] = [
    MatrixPoint { x: 0.1, y: 0.2 },
    MatrixPoint { x: 0.9, y: 0.2 },
    MatrixPoint { x: 0.9, y: 0.8 },
    MatrixPoint { x: 0.1, y: 0.8 },
];
/// Each cut cell carries its index in its first pixel; `glyph` decides its
/// measurements and the recognizer reads `A + index % 26`.
struct Scripted {
    glyph: fn(usize) -> MatrixCaptureGlyph,
    /// A cell that is read, but never clearly.
    weak: Option<u8>,
    cuts: AtomicU32,
}
fn clear(cell: usize) -> MatrixCaptureGlyph {
    let mut gray = vec![255; 96 * 96];
    gray[0] = cell as u8;
    MatrixCaptureGlyph {
        gray,
        contrast: 30.,
        sharpness: 100.,
        source_size: 20.,
        centering: 0.7,
        glare: 0.,
    }
}
struct Found;
impl MatrixGridTracker for Found {
    fn track(
        &mut self,
        planes: MatrixPlanes<'_>,
        _: f64,
    ) -> Result<MatrixGridTrack, MatrixScanError> {
        assert_eq!(planes.luma.len(), planes.chroma.len());
        Ok(MatrixGridTrack {
            display: Some(BODY),
            body: Some(BODY),
            keyframe: true,
            elapsed_ms: 1.,
        })
    }
}
impl MatrixVision for Scripted {
    fn available(&self) -> bool {
        true
    }
    fn warm_up(&self) -> Result<(), MatrixScanError> {
        Ok(())
    }
    fn tracker(&self) -> Box<dyn MatrixGridTracker> {
        Box::new(Found)
    }
    fn decode(&self, _: &[u8]) -> Result<MatrixPicture, MatrixScanError> {
        Ok(picture())
    }
    fn convert(&self, frame: &MatrixYuvFrame<'_>) -> Result<MatrixPicture, MatrixScanError> {
        assert_eq!((frame.width, frame.height), (64, 32));
        Ok(picture())
    }
    fn cut(
        &self,
        luma: MatrixLuma<'_>,
        body: [MatrixPoint; 4],
    ) -> Result<MatrixCut, MatrixScanError> {
        assert_eq!(body, BODY);
        // The cut always reads the full-resolution plane, never the analysis one.
        assert_eq!((luma.width, luma.height), (128, 64));
        self.cuts.fetch_add(1, Ordering::Relaxed);
        Ok(MatrixCut {
            quads: (0..CELLS)
                .map(|i| {
                    let (x, y) = ((i % 10) as f64 / 10., (i / 10) as f64 / 7.);
                    [
                        MatrixPoint { x, y },
                        MatrixPoint { x: x + 0.09, y },
                        MatrixPoint {
                            x: x + 0.09,
                            y: y + 0.13,
                        },
                        MatrixPoint { x, y: y + 0.13 },
                    ]
                })
                .collect(),
            glyphs: (0..CELLS).map(self.glyph).collect(),
        })
    }
    fn recognize(&self, images: &[Vec<u8>]) -> Result<Vec<MatrixReading>, MatrixScanError> {
        assert!(images.len() <= 10);
        Ok(images
            .iter()
            .map(|image| MatrixReading {
                text: ((b'A' + image[0] % 26) as char).to_string(),
                confidence: if Some(image[0]) == self.weak {
                    0.6
                } else {
                    0.95
                },
            })
            .collect())
    }
}
fn picture() -> MatrixPicture {
    MatrixPicture {
        analysis_width: 32,
        analysis_height: 16,
        analysis_luma: vec![0; 512],
        analysis_chroma: vec![128; 512],
        width: 128,
        height: 64,
        luma: vec![0; 128 * 64],
    }
}
fn scripted(
    glyph: fn(usize) -> MatrixCaptureGlyph,
    weak: Option<u8>,
) -> (Arc<MatrixCameraDriver>, Arc<Scripted>) {
    let vision = Arc::new(Scripted {
        glyph,
        weak,
        cuts: AtomicU32::new(0),
    });
    (Arc::new(MatrixCameraDriver::new(vision.clone())), vision)
}
fn letter(cell: usize) -> String {
    ((b'A' + (cell % 26) as u8) as char).to_string()
}
fn yuv<'a>(y: &'a [u8], uv: &'a [u8]) -> MatrixYuvFrame<'a> {
    MatrixYuvFrame {
        width: 64,
        height: 32,
        y,
        y_row_stride: 64,
        u: uv,
        v: &uv[1..],
        uv_row_stride: 64,
        uv_pixel_stride: 2,
        matrix: MatrixYuvMatrix::Bt709,
        full_range: true,
    }
}

/// Desktop sends planes and then the keyframe it was asked for; a phone sends
/// one YUV frame. Both must reach the same consensus through one consumer,
/// with outlines from the frame just sent and only sampled frames cut.
#[test]
fn desktop_and_native_transports_drive_one_session_to_the_same_votes() {
    for native in [false, true] {
        let (driver, vision) = scripted(clear, None);
        let id = driver.begin().unwrap();
        let consumer = {
            let (driver, id) = (driver.clone(), id.clone());
            std::thread::spawn(move || driver.process(&id, |_| true))
        };
        let full = picture();
        let (y, uv) = (vec![0u8; 64 * 32], vec![128u8; 64 * 16]);
        let mut last = driver.poll(&id).unwrap();
        for frame_id in 0..400u32 {
            last = if native {
                driver
                    .submit(&id, frame_id, &yuv(&y, &uv), frame_id as f64 * 20.)
                    .unwrap()
            } else {
                let tracked = driver
                    .track(&id, frame_id, full.planes(), frame_id as f64 * 20.)
                    .unwrap();
                if tracked.keyframe_frame_id == Some(frame_id) {
                    driver.keyframe(&id, frame_id, full.full()).unwrap()
                } else {
                    tracked
                }
            };
            if last.phase == MatrixCameraPhase::Complete {
                break;
            }
            assert_eq!(
                last.bounds_frame_id,
                Some(frame_id),
                "outline from this frame"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(last.phase, MatrixCameraPhase::Complete, "native={native}");
        let done = consumer.join().unwrap().unwrap();
        assert!(!done.timed_out, "native={native}");
        for (cell, vote) in done.cells.iter().enumerate() {
            assert_eq!(vote.text, letter(cell), "native={native} cell {cell}");
            assert!(vote.stable);
        }
        // Keyframes are sampled at Core's interval: not every tracked frame is cut.
        let report = driver.stop(&id).unwrap();
        let cuts = vision.cuts.load(Ordering::Relaxed) as usize;
        assert_eq!(cuts, report.keyframes, "native={native}");
        assert!(
            cuts >= 3 && report.frames.len() > 2 * cuts,
            "native={native} cuts={cuts}"
        );
        assert!(matches!(driver.poll(&id), Err(MatrixScanError::Ended)));
    }
}

#[test]
fn a_keyframe_is_cut_only_for_the_frame_core_requested() {
    let (driver, vision) = scripted(clear, None);
    let id = driver.begin().unwrap();
    let full = picture();
    let first = driver.track(&id, 1, full.planes(), 0.).unwrap();
    assert_eq!(first.keyframe_frame_id, Some(1));
    // Until a keyframe arrives, a newer qualifying frame replaces the request.
    let second = driver.track(&id, 2, full.planes(), 10.).unwrap();
    assert_eq!(second.keyframe_frame_id, Some(2));
    driver.keyframe(&id, 1, full.full()).unwrap();
    assert_eq!(
        vision.cuts.load(Ordering::Relaxed),
        0,
        "superseded keyframe dropped"
    );
    let cut = driver.keyframe(&id, 2, full.full()).unwrap();
    assert_eq!(vision.cuts.load(Ordering::Relaxed), 1);
    assert_eq!(cut.total_samples, CELLS);
    // The next frame is inside the sampling interval of that keyframe.
    let third = driver.track(&id, 3, full.planes(), 20.).unwrap();
    assert_eq!(third.keyframe_frame_id, None);
    driver.keyframe(&id, 3, full.full()).unwrap();
    assert_eq!(vision.cuts.load(Ordering::Relaxed), 1);
    // A stopped session drops its tracker and pending keyframe.
    driver.stop(&id);
    assert!(matches!(
        driver.track(&id, 4, full.planes(), 30.),
        Err(MatrixScanError::Ended)
    ));
}

/// A photo is one live frame: the same glare gate, texture gate and evidence
/// threshold decide each cell, so its result never claims more than a live
/// scan would from the same picture.
#[test]
fn a_photo_is_read_with_the_live_admission_and_evidence_rules() {
    fn mixed(cell: usize) -> MatrixCaptureGlyph {
        let mut glyph = clear(cell);
        match cell {
            0 => glyph.glare = 0.5,
            1 => glyph.contrast = 0.,
            2 => glyph.centering = 0.2,
            _ => {}
        }
        glyph
    }
    let (driver, _) = scripted(mixed, Some(69));
    let mut seen = vec![];
    let scan = driver
        .scan_photo(b"photo", |done, total| {
            seen.push((done, total));
            true
        })
        .unwrap();
    assert_eq!(scan.bounds, BODY);
    // Three rejected cells settle at once; the 67 read ones in batches of ten.
    assert_eq!(seen[..3], [(0, CELLS), (3, CELLS), (13, CELLS)]);
    assert_eq!(seen.last(), Some(&(CELLS, CELLS)));
    for (cell, vote) in scan.cells.iter().enumerate() {
        if matches!(cell, 0 | 1 | 2 | 69) {
            assert!(vote.text.is_empty(), "cell {cell} must be left for review");
        } else {
            assert_eq!(vote.text, letter(cell));
            assert_eq!(
                matrix_confidence(&vote.text, vote.confidence),
                MatrixConfidence::High
            );
        }
        assert!(!vote.stable, "one photo is one observation");
    }
    // A background without letter texture is not a Matrix, as on camera.
    let (blank, _) = scripted(
        |cell| MatrixCaptureGlyph {
            contrast: 0.,
            ..clear(cell)
        },
        None,
    );
    assert!(matches!(
        blank.scan_photo(b"photo", |_, _| true),
        Err(MatrixScanError::Unreadable(_))
    ));
    assert!(matches!(
        driver.scan_photo(b"photo", |done, _| done == 0),
        Err(MatrixScanError::Ended)
    ));
}
