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
fn each_cell_votes_only_over_its_own_high_confidence_frames() {
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    let a3 = 20;
    let b2 = 11;
    let mut result = None;
    for image in 1..=6 {
        let mut input = frame("D", 0.99);
        input.1[a3] = MatrixReading {
            text: if image == 4 { "Z" } else { "A" }.into(),
            confidence: match image {
                1 => 0.91,
                4 => 0.99,
                5 => 0.96,
                _ => 0.89,
            },
        };
        input.1[b2] = MatrixReading {
            text: if image == 4 { "Y" } else { "B" }.into(),
            confidence: if [2, 4, 5, 6].contains(&image) {
                0.98
            } else {
                0.7
            },
        };
        // The last cell cannot be hidden by the other 69 ready cells.
        input.1[69].confidence = if [2, 5, 6].contains(&image) {
            0.99
        } else {
            0.89
        };
        let next = scanner.observe(&id, Some(input)).unwrap();
        assert_eq!(next.complete, image == 6);
        result = Some(next);
    }
    let result = result.unwrap();
    assert_eq!(result.cells[a3].text, "A"); // frames 1,4,5: A,Z,A
    assert_eq!(result.cells[b2].text, "B"); // frames 2,4,5,6: B,Y,B,B
    assert!(
        (result.cells[a3].confidence
            - ((0.91 + 0.96) / 2.) * ((0.91 + 0.96) / (0.91 + 0.99 + 0.96)))
            .abs()
            < 1e-9
    );
    assert!((result.cells[b2].confidence - 0.98 * 3. / 4.).abs() < 1e-9);
    assert_eq!(result.confidence, result.cells[a3].confidence);
    // A low-confidence wrong current frame cannot erase the established result.
    assert!(
        scanner
            .observe(&id, Some(frame("Z", 0.89)))
            .unwrap()
            .complete
    );
}

#[test]
fn movement_and_blur_preserve_votes_but_ties_and_sustained_absence_do_not_accept() {
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    for image in 0..8 {
        let mut input = frame("A", 0.99);
        for (index, cell) in input.1.iter_mut().enumerate() {
            if index % 2 == image % 2 {
                cell.text.clear();
                cell.confidence = 0.;
            }
        }
        for quad in &mut input.0 {
            for point in quad {
                point.x = point.x * 0.7 + if image % 2 == 0 { 0.15 } else { 0. };
            }
        }
        if image == 3 {
            assert!(scanner.observe(&id, None).unwrap().bounds.is_none());
        }
        let result = scanner.observe(&id, Some(input)).unwrap();
        // The clear samples from complementary frames add up at frame six.
        assert_eq!(result.complete, image >= 5);
    }
    for _ in 0..4 {
        assert!(!scanner.observe(&id, None).unwrap().complete);
    }
    let result = scanner.observe(&id, Some(frame("B", 0.99))).unwrap();
    assert!(!result.complete);
    assert!(result.confidence > 0.98); // confidence alone cannot replace sample quorum
    for letter in ["A", "A"] {
        scanner.observe(&id, Some(frame(letter, 0.99))).unwrap();
    }
    assert!(
        !scanner
            .observe(&id, Some(frame("B", 0.99)))
            .unwrap()
            .complete
    ); // 2–2 tie
    for _ in 0..6 {
        scanner.observe(&id, Some(frame("C", 0.95))).unwrap();
    }
    assert_eq!(
        scanner.observe(&id, Some(frame("C", 0.95))).unwrap().cells[0].text,
        "C"
    );
}

#[test]
fn sessions_reject_late_results_and_invalid_frames_without_polluting_consensus() {
    let mut scanner = MatrixScanner::default();
    let old = scanner.begin();
    let id = scanner.begin();
    scanner.stop(&old); // an old UI cleanup cannot kill its replacement
    assert!(matches!(
        scanner.outline(&old, &frame("A", 1.).0),
        Err(MatrixScanError::Ended)
    ));
    let outline = scanner.outline(&id, &frame("A", 1.).0).unwrap();
    assert_eq!(outline.bounds[2], frame("A", 1.).0[69][2]);
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
