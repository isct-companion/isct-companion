use super::*;
fn frame(usable: impl Fn(usize) -> bool, marker: u8) -> MatrixCaptureFrame {
    MatrixCaptureFrame {
        quads: (0..CELLS)
            .map(|i| {
                let x = (i % 10) as f64 / 10.;
                let y = (i / 10) as f64 / 7.;
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
        glyphs: (0..CELLS)
            .map(|i| MatrixCaptureGlyph {
                gray: vec![marker; GRAY_PIXELS],
                contrast: if usable(i) { 30. } else { 0. },
                sharpness: 100.,
                source_size: 20.,
                centering: 0.7,
                glare: 0.,
            })
            .collect(),
    }
}

fn read(batch: &MatrixCameraBatch, text: &str) -> Vec<MatrixReading> {
    batch
        .cells
        .iter()
        .map(|_| MatrixReading {
            text: text.into(),
            confidence: 0.99,
        })
        .collect()
}
#[test]
fn streaming_batches_are_exclusive_fair_and_keep_capture_open() {
    let now = Instant::now();
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin_at(now, CAPTURE_LIMIT);
    scanner.observe(&id, Some(frame(|_| true, 1)), now).unwrap();
    let batch = scanner.batch(&id, now).unwrap().unwrap();
    assert_eq!(batch.cells, (0..10).collect::<Vec<_>>());
    assert!(scanner.batch(&id, now).unwrap().is_none());
    // A producer is free to replace queued candidates while OCR owns its batch.
    for i in 1..5 {
        scanner
            .observe(
                &id,
                Some(frame(|_| true, i)),
                now + Duration::from_millis(i as u64 * 100),
            )
            .unwrap();
    }
    let at = now + Duration::from_millis(400);
    let snapshot = scanner.apply(&id, &batch, read(&batch, "A"), at).unwrap();
    assert_eq!(snapshot.phase, MatrixCameraPhase::Capturing);
    assert_eq!(snapshot.cells[0].text, "A");
    assert_eq!(
        scanner.batch(&id, at).unwrap().unwrap().cells,
        (10..20).collect::<Vec<_>>()
    );
    assert!(scanner
        .diagnostics(&id, at)
        .unwrap()
        .candidates
        .iter()
        .all(|&n| n <= POOL_LIMIT));
}
#[test]
fn session_deadline_freezes_results_even_without_geometry_or_with_inflight_ocr() {
    let now = Instant::now();
    let mut s = MatrixCameraScanner::default();
    let id = s.begin_at(now, Duration::from_secs(6));
    s.observe(&id, Some(frame(|_| true, 1)), now).unwrap();
    let batch = s.batch(&id, now).unwrap().unwrap();
    let at = now + Duration::from_secs(6);
    let result = s.apply(&id, &batch, read(&batch, "A"), at).unwrap();
    assert!(result.timed_out);
    assert_eq!(result.phase, MatrixCameraPhase::Complete);
    assert_eq!(result.completed_samples, 0);
    assert_eq!(result.capture_elapsed_ms, 6000.);
    assert_eq!(
        s.observe(&id, None, at + CAPTURE_LIMIT)
            .unwrap()
            .capture_elapsed_ms,
        6000.
    );
    let empty = s.begin_at(now, CAPTURE_LIMIT);
    assert!(
        s.observe(&empty, None, now + CAPTURE_LIMIT)
            .unwrap()
            .timed_out
    );
    assert!(s.apply(&empty, &batch, read(&batch, "A"), now).is_err());
}
#[test]
fn candidates_expire_and_tampered_or_replayed_batches_are_rejected() {
    let now = Instant::now();
    let mut s = MatrixCameraScanner::default();
    let id = s.begin_at(now, CAPTURE_LIMIT);
    s.observe(&id, Some(frame(|_| true, 1)), now).unwrap();
    let batch = s.batch(&id, now).unwrap().unwrap();
    let mut changed = batch.clone();
    changed.images[0][0] ^= 1;
    assert!(s.apply(&id, &changed, read(&changed, "A"), now).is_err());
    s.apply(&id, &batch, read(&batch, "A"), now).unwrap();
    assert!(s.apply(&id, &batch, read(&batch, "A"), now).is_err());
    assert!(s
        .batch(&id, now + CANDIDATE_TTL + Duration::from_millis(1))
        .unwrap()
        .is_none());
}
#[test]
fn consensus_needs_fresh_confirmation_and_contradiction_reopens_it() {
    let now = Instant::now();
    let mut s = MatrixCameraScanner::default();
    let id = s.begin_at(now, CAPTURE_LIMIT);
    for round in 0..3 {
        let at = now + Duration::from_millis(round * 100);
        s.observe(&id, Some(frame(|_| true, round as u8)), at)
            .unwrap();
        for _ in 0..7 {
            let b = s.batch(&id, at).unwrap().unwrap();
            s.apply(&id, &b, read(&b, "A"), at).unwrap();
        }
    }
    assert_eq!(
        s.observe(&id, None, now + Duration::from_millis(250))
            .unwrap()
            .ready_cells,
        CELLS
    );
    assert!(s
        .batch(&id, now + Duration::from_millis(250))
        .unwrap()
        .is_none());
    let at = now + Duration::from_millis(300);
    s.observe(&id, Some(frame(|_| true, 4)), at).unwrap();
    let b = s.batch(&id, at).unwrap().unwrap();
    let result = s.apply(&id, &b, read(&b, "B"), at).unwrap();
    assert_eq!(result.ready_cells, 60);
    assert_eq!(result.phase, MatrixCameraPhase::Capturing);
    assert_eq!(result.cells[0].text, "B");
    // Restore a consistent stream and require a full fresh pass to complete.
    for round in 4..20 {
        let at = now + Duration::from_millis(round * 100);
        s.observe(&id, Some(frame(|_| true, round as u8)), at)
            .unwrap();
        while let Some(b) = s.batch(&id, at).unwrap() {
            let result = s.apply(&id, &b, read(&b, "A"), at).unwrap();
            if result.phase == MatrixCameraPhase::Complete {
                assert!(!result.timed_out);
                assert_eq!(result.ready_cells, CELLS);
                assert!(result.cells.iter().all(|v| v.text == "A"));
                return;
            }
        }
    }
    panic!("consistent observations did not complete verification");
}
#[test]
fn invalid_frames_quality_thresholds_and_glare_keep_independent_admission() {
    let now = Instant::now();
    let mut s = MatrixCameraScanner::default();
    let id = s.begin_at(now, CAPTURE_LIMIT);
    for invalid in 0..3 {
        let mut f = frame(|_| true, 1);
        match invalid {
            0 => {
                f.glyphs[0].gray.pop();
            }
            1 => f.glyphs[0].glare = f64::NAN,
            _ => f.glyphs[0].sharpness = f64::NAN,
        }
        assert!(s.observe(&id, Some(f), now).is_err());
    }
    s.observe(&id, Some(frame(|_| false, 1)), now).unwrap();
    assert!(s.batch(&id, now).unwrap().is_none());
    let mut f = frame(|_| true, 2);
    f.glyphs[0].glare = 0.9;
    f.glyphs[1].source_size = 9.;
    f.glyphs[2].sharpness = 9.;
    f.glyphs[3].source_size = 10.;
    f.glyphs[3].sharpness = 10.;
    s.observe(&id, Some(f), now + SAMPLE_INTERVAL).unwrap();
    let b = s.batch(&id, now + SAMPLE_INTERVAL).unwrap().unwrap();
    assert_eq!(b.cells[0], 3);
    assert_eq!(
        s.diagnostics(&id, now + SAMPLE_INTERVAL)
            .unwrap()
            .glare_rejections[0],
        1
    );
    assert!(s.observe(&id, None, now).is_err());
    s.apply(&id, &b, read(&b, "A"), now + SAMPLE_INTERVAL)
        .unwrap();
    s.observe(&id, Some(frame(|_| true, 3)), now + SAMPLE_INTERVAL * 2)
        .unwrap();
    let recovered = s.batch(&id, now + SAMPLE_INTERVAL * 2).unwrap().unwrap();
    assert_eq!(&recovered.cells[..3], &[0, 1, 2]);
    assert!(s
        .observe(&id, None, now + Duration::from_millis(600))
        .unwrap()
        .bounds
        .is_none());
    s.stop(&id);
    assert!(s.batch(&id, now).is_err());
}
