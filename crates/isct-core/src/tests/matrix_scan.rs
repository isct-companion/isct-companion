use super::*;

fn frame(letter: &str, confidence: f64) -> (Vec<[MatrixPoint; 4]>, Vec<MatrixReading>) {
    let quads = (0..CELLS)
        .map(|i| {
            let x = (i % 10) as f64 / 11.;
            let y = (i / 10) as f64 / 8.;
            [
                MatrixPoint { x, y },
                MatrixPoint { x: x + 0.08, y },
                MatrixPoint {
                    x: x + 0.08,
                    y: y + 0.1,
                },
                MatrixPoint { x, y: y + 0.1 },
            ]
        })
        .collect();
    (
        quads,
        (0..CELLS)
            .map(|_| MatrixReading {
                text: letter.into(),
                confidence,
            })
            .collect(),
    )
}

#[test]
fn confidence_weighting_requires_repeated_clear_consensus_and_evicts_old_errors() {
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    // Three weak votes must not defeat two clear votes just by their count.
    for _ in 0..3 {
        assert!(
            !scanner
                .observe(&id, Some(frame("B", 0.65)))
                .unwrap()
                .complete
        );
    }
    for _ in 0..2 {
        scanner.observe(&id, Some(frame("A", 0.99))).unwrap();
    }
    let snapshot = scanner.observe(&id, Some(frame("A", 0.99))).unwrap();
    assert_eq!(snapshot.cells[0].text, "A");
    assert!(!snapshot.complete); // conflicting confidence mass remains too high
    for _ in 0..5 {
        scanner.observe(&id, Some(frame("A", 0.99))).unwrap();
    }
    let snapshot = scanner.observe(&id, Some(frame("A", 0.99))).unwrap();
    assert!(snapshot.complete);
    assert_eq!(snapshot.stable_cells, CELLS);
    assert!((snapshot.cells[0].confidence - 0.99).abs() < 1e-9);
    // A conflicting current frame cannot auto-accept yesterday's winner.
    let snapshot = scanner.observe(&id, Some(frame("B", 0.99))).unwrap();
    assert_eq!(snapshot.cells[0].text, "A");
    assert!(!snapshot.cells[0].stable);
}

#[test]
fn one_unclear_cell_blocks_completion_and_lost_or_moved_cards_reset_votes() {
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    for _ in 0..3 {
        let mut input = frame("A", 0.99);
        input.1[69] = MatrixReading {
            text: "B".into(),
            confidence: 0.89,
        };
        let snapshot = scanner.observe(&id, Some(input)).unwrap();
        assert!(!snapshot.complete);
    }
    assert_eq!(
        scanner
            .observe(&id, Some(frame("A", 0.99)))
            .unwrap()
            .stable_cells,
        69
    );
    let lost = scanner.observe(&id, None).unwrap();
    assert!(lost.quads.is_empty() && lost.cells.is_empty() && !lost.complete);
    for i in 0..3 {
        assert_eq!(
            scanner
                .observe(&id, Some(frame("B", 0.99)))
                .unwrap()
                .complete,
            i == 2
        );
    }
    let mut moved = frame("B", 0.99);
    for quad in &mut moved.0 {
        for point in quad {
            point.x += 0.09;
        }
    }
    assert_eq!(scanner.observe(&id, Some(moved)).unwrap().stable_cells, 0);
    assert!(!scanner.observe(&id, Some(frame("", 0.))).unwrap().complete);
}

#[test]
fn sessions_reject_late_results_and_invalid_frames_without_polluting_consensus() {
    let mut scanner = MatrixScanner::default();
    let old = scanner.begin();
    let id = scanner.begin();
    scanner.stop(&old); // an old UI cleanup cannot kill its replacement
    assert!(matches!(
        scanner.observe(&old, Some(frame("A", 1.))),
        Err(MatrixScanError::Ended)
    ));
    for mut invalid in [frame("A", f64::NAN), frame("bad", 0.9), frame("A", 1.1)] {
        assert!(matches!(
            scanner.observe(&id, Some(invalid.clone())),
            Err(MatrixScanError::InvalidFrame(_))
        ));
        invalid.1.pop();
        assert!(scanner.observe(&id, Some(invalid)).is_err());
    }
    let mut invalid = frame("A", 1.);
    invalid.0[0][0].x = -0.1;
    assert!(scanner.observe(&id, Some(invalid)).is_err());
    for i in 0..3 {
        assert_eq!(
            scanner
                .observe(&id, Some(frame("B", 0.99)))
                .unwrap()
                .complete,
            i == 2
        );
    }
    scanner.stop(&id);
    assert!(matches!(
        scanner.observe(&id, None),
        Err(MatrixScanError::Ended)
    ));
}
