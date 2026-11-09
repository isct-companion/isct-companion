use super::*;

#[test]
fn each_cell_votes_only_over_its_own_qualified_frames_including_point_eight() {
    let mut a3 = CellEvidence::default();
    let mut b2 = CellEvidence::default();
    for image in 1..=6 {
        a3.collect(
            MatrixReading {
                text: if image == 4 { "Z" } else { "A" }.into(),
                confidence: match image {
                    1 => 0.8,
                    4 => 0.99,
                    5 => 0.89,
                    _ => 0.79,
                },
            },
            true,
        );
        b2.collect(
            MatrixReading {
                text: if image == 4 { "Y" } else { "B" }.into(),
                confidence: if [2, 4, 5, 6].contains(&image) {
                    0.88
                } else {
                    0.7
                },
            },
            true,
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
    a3.collect(
        MatrixReading {
            text: "Z".into(),
            confidence: 0.79,
        },
        true,
    );
    assert_eq!(a3.samples.len(), 3);
    assert!(a3.vote().stable);
    let mut tied = CellEvidence::default();
    for text in ["A", "B", "A", "B"] {
        tied.collect(
            MatrixReading {
                text: text.into(),
                confidence: 0.85,
            },
            true,
        );
    }
    assert!(!tied.vote().stable);
}

#[test]
fn review_levels_split_at_the_shared_thresholds() {
    assert_eq!(matrix_confidence("A", 0.9, true), MatrixConfidence::High);
    assert_eq!(matrix_confidence("A", 0.89, true), MatrixConfidence::Medium);
    assert_eq!(matrix_confidence("A", 0.75, true), MatrixConfidence::Medium);
    assert_eq!(matrix_confidence("A", 0.74, true), MatrixConfidence::Low);
    // Without agreeing observations a high score is only worth a check, and a
    // low one stays low.
    assert_eq!(
        matrix_confidence("A", 0.99, false),
        MatrixConfidence::Medium
    );
    assert_eq!(
        matrix_confidence("A", 0.75, false),
        MatrixConfidence::Medium
    );
    assert_eq!(matrix_confidence("A", 0.74, false), MatrixConfidence::Low);
    // An empty or non-finite reading is never trusted, whatever the score.
    assert_eq!(matrix_confidence("", 1.0, true), MatrixConfidence::Low);
    assert_eq!(
        matrix_confidence("A", f64::NAN, true),
        MatrixConfidence::Low
    );
    assert_eq!(
        matrix_confidence("A", f64::INFINITY, true),
        MatrixConfidence::Low
    );
}

#[test]
fn camera_votes_cap_unsettled_cells_but_one_photo_reading_is_not_capped() {
    let reading = |text: &str| MatrixReading {
        text: text.into(),
        confidence: 0.95,
    };
    let mut cell = CellEvidence::default();
    assert_eq!(cell.vote().level, MatrixConfidence::Low);
    assert_eq!(cell.photo_vote().level, MatrixConfidence::Low);
    cell.collect(reading("A"), false);
    // One clear reading: a camera cell still waits for agreement; a photo never
    // gets a second reading, so its score decides.
    assert!(!cell.vote().stable);
    assert_eq!(cell.vote().level, MatrixConfidence::Medium);
    assert_eq!(cell.photo_vote().level, MatrixConfidence::High);
    cell.collect(reading("A"), false);
    cell.collect(reading("A"), false);
    assert!(cell.vote().stable);
    assert_eq!(cell.vote().level, MatrixConfidence::High);
}
