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
#[test]
fn independent_coverage_stops_capture_without_running_recognition() {
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin();
    let now = Instant::now();
    scanner.observe(&id, None, now).unwrap();
    // Camera-open time and false background grids do not start the eight seconds.
    let later = now + Duration::from_secs(100);
    assert_eq!(
        scanner
            .observe(&id, Some(frame(|_| false, 0)), later)
            .unwrap()
            .capture_elapsed_ms,
        0.
    );
    for round in 0..10 {
        let sample = later + Duration::from_millis(round * 100);
        let snapshot = scanner
            .observe(
                &id,
                Some(frame(|i| (i < 35) == (round % 2 == 0), round as u8)),
                sample,
            )
            .unwrap();
        assert_eq!(
            snapshot.phase,
            if round == 9 {
                MatrixCameraPhase::Processing
            } else {
                MatrixCameraPhase::Capturing
            }
        );
    }
    let report = scanner
        .diagnostics(&id, later + Duration::from_secs(1))
        .unwrap();
    assert_eq!(report.candidates, vec![5; CELLS]);
    assert!(report.qualified_samples.iter().all(|&n| n == 0));
    assert_eq!(report.capture_elapsed_ms, 900.);
    assert!(!report.timed_out);
}
#[test]
fn deadline_is_from_first_matrix_even_when_frames_stop_or_remain_incomplete() {
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin();
    let now = Instant::now();
    scanner
        .observe(&id, Some(frame(|i| i != 69, 1)), now)
        .unwrap();
    scanner
        .observe(
            &id,
            Some(frame(|i| i != 69, 2)),
            now + Duration::from_millis(100),
        )
        .unwrap();
    let before = scanner
        .observe(&id, None, now + CAPTURE_LIMIT - Duration::from_millis(1))
        .unwrap();
    assert_eq!(before.phase, MatrixCameraPhase::Capturing);
    let timeout = scanner.observe(&id, None, now + CAPTURE_LIMIT).unwrap();
    assert_eq!(timeout.phase, MatrixCameraPhase::Processing);
    assert!(timeout.timed_out);
    assert_eq!(timeout.ready_cells, 0);
    assert_eq!(timeout.total_samples, 138);
    assert_eq!(timeout.capture_elapsed_ms, 8000.);
    let frozen = scanner
        .observe(&id, Some(frame(|_| true, 3)), now + Duration::from_secs(20))
        .unwrap();
    assert_eq!(frozen.total_samples, 138);
    assert_eq!(frozen.capture_elapsed_ms, 8000.);
}
#[test]
fn bounded_distinct_candidates_vote_offline_without_order_dependent_reopening() {
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin();
    let now = Instant::now();
    for round in 0..14 {
        let time = now + Duration::from_millis(round * 100);
        scanner
            .observe(
                &id,
                Some(frame(|i| i != 69 || round >= 9, round as u8)),
                time,
            )
            .unwrap();
        // A duplicate wakeup never contributes a second sample at this instant.
        scanner
            .observe(
                &id,
                Some(frame(|i| i != 69 || round >= 9, round as u8)),
                time,
            )
            .unwrap();
    }
    let report = scanner
        .diagnostics(&id, now + Duration::from_secs(2))
        .unwrap();
    assert_eq!(&report.candidates[..69], &[6; 69]);
    assert_eq!(report.candidates[69], 5);
    let mut seen = vec![0; CELLS];
    let mut last = None;
    while let Some(mut batch) = scanner.batch(&id).unwrap() {
        if seen.iter().all(|&n| n == 0) {
            batch.cells[0] = CELLS;
            assert!(scanner
                .apply(&id, &batch, vec![], now + Duration::from_secs(2))
                .is_err());
            batch = scanner.batch(&id).unwrap().unwrap();
        }
        let readings = batch
            .cells
            .iter()
            .map(|&i| {
                seen[i] += 1;
                MatrixReading {
                    text: if i == 0 && seen[i] > 3 { "B" } else { "A" }.into(),
                    confidence: if i == 69 && seen[i] < 5 { 0.79 } else { 0.8 },
                }
            })
            .collect();
        let result = scanner
            .apply(&id, &batch, readings, now + Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            scanner.apply(&id, &batch, vec![], now + Duration::from_secs(2)),
            Err(MatrixScanError::Ended)
        ));
        last = Some(result);
    }
    let result = last.unwrap();
    assert_eq!(result.phase, MatrixCameraPhase::Complete);
    assert!(!result.cells[0].stable); // 3 A versus 3 B must remain a tie.
    assert!(result.cells[1..69]
        .iter()
        .all(|c| c.stable && c.text == "A"));
    assert!(!result.cells[69].stable); // weak candidates do not become votes.
    let next = scanner.begin();
    scanner.stop(&id);
    assert!(scanner.is_active(&next));
    assert!(scanner.batch(&id).is_err());
    scanner.stop(&next);
    assert!(scanner.batch(&next).is_err());
}
#[test]
fn invalid_capture_does_not_advance_clock_or_fill_candidates() {
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin();
    let now = Instant::now();
    let mut invalid = frame(|_| true, 0);
    invalid.glyphs[0].gray.pop();
    assert!(scanner.observe(&id, Some(invalid), now).is_err());
    let mut invalid = frame(|_| true, 0);
    invalid.glyphs[0].glare = f64::NAN;
    assert!(scanner.observe(&id, Some(invalid), now).is_err());
    let mut invalid = frame(|_| true, 0);
    invalid.glyphs[0].sharpness = f64::NAN;
    assert!(scanner.observe(&id, Some(invalid), now).is_err());
    assert!(scanner
        .diagnostics(&id, now)
        .unwrap()
        .candidates
        .iter()
        .all(|&n| n == 0));
    for (sharpness, source_size) in [(9., 20.), (100., 9.)] {
        let mut low_quality = frame(|_| true, 0);
        for glyph in &mut low_quality.glyphs {
            glyph.sharpness = sharpness;
            glyph.source_size = source_size;
        }
        assert_eq!(
            scanner
                .observe(&id, Some(low_quality), now + Duration::from_millis(500))
                .unwrap()
                .capture_elapsed_ms,
            0.
        );
    }
    let mut camera_resolution_boundary = frame(|_| true, 0);
    for glyph in &mut camera_resolution_boundary.glyphs {
        glyph.sharpness = 10.;
        glyph.source_size = 10.;
    }
    scanner
        .observe(
            &id,
            Some(camera_resolution_boundary),
            now + Duration::from_secs(1),
        )
        .unwrap();
    assert!(scanner.observe(&id, Some(frame(|_| true, 0)), now).is_err());
}

#[test]
fn glare_waits_for_clear_cells_without_preventing_first_matrix_admission() {
    let mut scanner = MatrixCameraScanner::default();
    let id = scanner.begin();
    let now = Instant::now();
    for round in 0..7 {
        let mut sample = frame(|_| true, round);
        if round < 2 {
            sample.glyphs[0].glare = 0.5;
        }
        let state = scanner
            .observe(
                &id,
                Some(sample),
                now + Duration::from_millis(round as u64 * 100),
            )
            .unwrap();
        assert!(state.bounds.is_some());
        assert_eq!(
            state.phase,
            if round < 6 {
                MatrixCameraPhase::Capturing
            } else {
                MatrixCameraPhase::Processing
            }
        );
    }
    let report = scanner
        .diagnostics(&id, now + Duration::from_secs(1))
        .unwrap();
    assert_eq!(report.candidates[0], 5);
    assert_eq!(report.glare_rejections[0], 2);
    assert_eq!(report.geometry_frames, 7);
    assert_eq!(report.admission_rejections, 0);
    assert_eq!(report.capture_elapsed_ms, 600.);
}
