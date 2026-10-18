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

fn feed(
    scanner: &mut MatrixScanner,
    id: &str,
    input: Option<(Vec<[MatrixPoint; 4]>, Vec<MatrixReading>)>,
) -> MatrixScanSnapshot {
    let plan = scanner.plan(id).unwrap();
    let selected = input.map(|(quads, readings)| {
        (
            quads,
            plan.cells.iter().map(|&i| readings[i].clone()).collect(),
        )
    });
    scanner
        .observe(id, &plan, selected, MatrixScanTiming::default())
        .unwrap()
}

#[test]
fn each_cell_votes_only_over_its_own_qualified_frames_including_point_eight() {
    let mut a3 = CellEvidence::default();
    let mut b2 = CellEvidence::default();
    for image in 1..=6 {
        a3.add(
            MatrixReading {
                text: if image == 4 { "Z" } else { "A" }.into(),
                confidence: match image {
                    1 => 0.8,
                    4 => 0.99,
                    5 => 0.89,
                    _ => 0.79,
                },
            },
            image,
        );
        b2.add(
            MatrixReading {
                text: if image == 4 { "Y" } else { "B" }.into(),
                confidence: if [2, 4, 5, 6].contains(&image) {
                    0.88
                } else {
                    0.7
                },
            },
            image,
        );
    }
    assert_eq!(a3.samples.len(), 3);
    assert_eq!(b2.samples.len(), 4);
    assert_eq!(a3.vote().text, "A");
    assert_eq!(b2.vote().text, "B");
    assert!(a3.vote().stable && b2.vote().stable);
    assert!(
        (a3.vote().confidence - ((0.8 + 0.89) / 2.) * ((0.8 + 0.89) / (0.8 + 0.99 + 0.89))).abs()
            < 1e-9
    );
    a3.add(
        MatrixReading {
            text: "Z".into(),
            confidence: 0.79,
        },
        7,
    );
    assert_eq!(a3.samples.len(), 3);
    assert!(a3.vote().stable);
    let mut tied = CellEvidence::default();
    for (i, text) in ["A", "B", "A", "B"].into_iter().enumerate() {
        tied.add(
            MatrixReading {
                text: text.into(),
                confidence: 0.85,
            },
            i,
        );
    }
    assert!(!tied.vote().stable);
}

#[test]
fn scheduler_scans_all_then_refines_fairly_and_verifies_before_completion() {
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    assert_eq!(scanner.plan(&id).unwrap().cells.len(), CELLS);
    // One unreadable cell must not starve the remaining 69 cells.
    let mut partial = frame("A", 0.89);
    partial.1[0] = MatrixReading {
        text: String::new(),
        confidence: 0.,
    };
    feed(&mut scanner, &id, Some(partial.clone()));
    let mut stalled = None;
    for _ in 0..24 {
        let plan = scanner.plan(&id).unwrap();
        assert_eq!(plan.phase, MatrixScanPhase::Refine);
        assert!(plan.cells.len() <= 10);
        stalled = Some(feed(&mut scanner, &id, Some(partial.clone())));
    }
    assert!(stalled.unwrap().needs_adjustment);
    assert_eq!(scanner.diagnostics(&id).unwrap().stable_cells, 69);
    for _ in 0..3 {
        feed(&mut scanner, &id, Some(frame("A", 0.89)));
    }
    assert_eq!(scanner.plan(&id).unwrap().phase, MatrixScanPhase::Verify);
    // A clear contradiction during final verification reopens that cell.
    let mut corrected = frame("A", 0.89);
    corrected.1[69].text = "Z".into();
    assert!(!feed(&mut scanner, &id, Some(corrected.clone())).complete);
    assert_eq!(scanner.plan(&id).unwrap().phase, MatrixScanPhase::Refine);
    let mut final_result = None;
    for _ in 0..20 {
        let result = feed(&mut scanner, &id, Some(corrected.clone()));
        let complete = result.complete;
        final_result = Some(result);
        if complete {
            break;
        }
    }
    let result = final_result.unwrap();
    assert!(result.complete);
    assert_eq!(result.cells[69].text, "Z");
    let report = scanner.diagnostics(&id).unwrap();
    assert_eq!(report.stable_cells, CELLS);
    assert!(!serde_json::to_string(&report).unwrap().contains("text"));

    let mut fast = MatrixScanner::full_frame();
    let id = fast.begin();
    for _ in 0..2 {
        feed(&mut fast, &id, Some(frame("A", 0.89)));
    }
    assert_eq!(fast.plan(&id).unwrap().cells.len(), CELLS);
    let before = fast.diagnostics(&id).unwrap().qualified_samples;
    let mut blurred = frame("A", 0.79);
    for cell in &mut blurred.1[..48] {
        cell.text = "Z".into();
        cell.confidence = 0.99;
    }
    for _ in 0..5 {
        let rejected = feed(&mut fast, &id, Some(blurred.clone()));
        assert_eq!(rejected.sampled_cells, 0);
    }
    assert_eq!(fast.diagnostics(&id).unwrap().qualified_samples, before);
    assert_eq!(
        feed(&mut fast, &id, Some(frame("A", 0.89))).stable_cells,
        CELLS
    );
    assert_eq!(fast.plan(&id).unwrap().phase, MatrixScanPhase::Verify);
    let mut verification = frame("A", 0.1);
    for cell in &mut verification.1[..49] {
        cell.confidence = 0.8;
    }
    let finished = feed(&mut fast, &id, Some(verification));
    assert!(finished.complete);
    assert!(finished.cells.iter().all(|cell| cell.text == "A"));

    let id = fast.begin();
    feed(&mut fast, &id, Some(frame("A", 0.89)));
    for _ in 0..6 {
        feed(&mut fast, &id, None);
    }
    assert_eq!(fast.diagnostics(&id).unwrap().qualified_samples[0], 1);
    let (_, session) = fast.active.as_mut().unwrap();
    session.missing_since = Some(Instant::now() - Duration::from_secs(4));
    feed(&mut fast, &id, None);
    assert!(fast
        .diagnostics(&id)
        .unwrap()
        .qualified_samples
        .iter()
        .all(|&count| count == 0));
}

#[test]
fn moving_closer_and_brief_grid_loss_preserve_votes_but_sustained_absence_resets() {
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    feed(&mut scanner, &id, Some(frame("A", 0.85)));
    let before = scanner.diagnostics(&id).unwrap().qualified_samples;
    assert!(feed(&mut scanner, &id, None).bounds.is_none());
    let mut moved = frame("A", 0.85);
    for quad in &mut moved.0 {
        for point in quad {
            point.x = point.x * 0.7 + 0.15;
        }
    }
    feed(&mut scanner, &id, Some(moved));
    let after = scanner.diagnostics(&id).unwrap().qualified_samples;
    assert!(after.iter().zip(before).all(|(a, b)| *a >= b));
    for _ in 0..4 {
        assert!(!feed(&mut scanner, &id, None).complete);
    }
    assert_eq!(scanner.diagnostics(&id).unwrap().qualified_samples, after);
    let (_, session) = scanner.active.as_mut().unwrap();
    session.missing_since = Some(Instant::now() - Duration::from_secs(4));
    feed(&mut scanner, &id, None);
    assert_eq!(scanner.plan(&id).unwrap().phase, MatrixScanPhase::Initial);
    assert!(scanner
        .diagnostics(&id)
        .unwrap()
        .qualified_samples
        .iter()
        .all(|&n| n == 0));
}

#[test]
fn sessions_reject_stale_plans_and_invalid_frames_without_polluting_votes() {
    let mut scanner = MatrixScanner::default();
    let old = scanner.begin();
    let id = scanner.begin();
    scanner.stop(&old);
    assert!(matches!(scanner.plan(&old), Err(MatrixScanError::Ended)));
    assert!(matches!(
        scanner.outline(&old, &frame("A", 1.).0),
        Err(MatrixScanError::Ended)
    ));
    let plan = scanner.plan(&id).unwrap();
    for invalid in [frame("A", f64::NAN), frame("bad", 0.9), frame("A", 1.1)] {
        assert!(matches!(
            scanner.observe(&id, &plan, Some(invalid), MatrixScanTiming::default()),
            Err(MatrixScanError::InvalidFrame(_))
        ));
    }
    let mut invalid = frame("A", 1.);
    invalid.0[0][0].x = -0.1;
    assert!(scanner
        .observe(&id, &plan, Some(invalid), MatrixScanTiming::default())
        .is_err());
    let mut modified = plan.clone();
    modified.cells = vec![CELLS];
    assert!(scanner
        .observe(&id, &modified, None, MatrixScanTiming::default())
        .is_err());
    feed(&mut scanner, &id, Some(frame("A", 0.85)));
    assert!(matches!(
        scanner.observe(
            &id,
            &plan,
            Some(frame("A", 0.85)),
            MatrixScanTiming::default()
        ),
        Err(MatrixScanError::Ended)
    ));
    scanner.stop(&id);
    assert!(scanner.plan(&id).is_err());
}
