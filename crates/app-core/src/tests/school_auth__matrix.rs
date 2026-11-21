use super::*;
use crate::school_auth::{tests::MemoryVault, SchoolCredentials};

#[test]
fn matrix_text_follows_the_editors_visual_row_order() {
    let service = SchoolAuthService::new(MemoryVault::default());
    assert!(service.matrix_text().is_err());
    // Row-major A1…J1, A2…: cell i holds the i-th letter, wrapping after Z.
    let matrix: String = (0..70u8).map(|i| (b'A' + i % 26) as char).collect();
    service
        .promote(
            SchoolAuthProfile::TokyoTech,
            &SchoolCredentials {
                username: "user".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix,
            },
        )
        .unwrap();
    let text = service.matrix_text().unwrap();
    let rows = text.lines().collect::<Vec<_>>();
    assert_eq!(rows.len(), 7);
    assert!(rows.iter().all(|row| row.len() == 10));
    assert_eq!(rows[0], "ABCDEFGHIJ");
    assert_eq!(rows[6], "IJKLMNOPQR");

    // The copy pastes back as the same cells, lower case and tabs included.
    let stored = service
        .load(SchoolAuthProfile::TokyoTech)
        .unwrap()
        .unwrap()
        .matrix;
    for pasted in [text.clone(), text.to_lowercase().replace('\n', "\t")] {
        assert_eq!(
            parse_matrix_text(&pasted).unwrap(),
            MatrixPaste {
                cells: stored.clone(),
                order: MatrixOrder::Rows
            }
        );
    }
}

#[test]
fn titech_app_export_pastes_down_columns_and_bad_pastes_are_rejected() {
    // A titech-app export (random test data): a JSON array listing
    // A1…A7, then B1…B7, and so on.
    let export = "LCIVTBAQMCMJXLTIRNPLZIMLFZXHWCACIOGFNNMVOPLUNLIERRTYZUXIZCXCAOSPLMRSVI";
    let json =
        serde_json::to_string(&export.chars().map(String::from).collect::<Vec<_>>()).unwrap();
    let pasted = parse_matrix_text(&format!("{json}\n")).unwrap();
    assert_eq!(pasted.order, MatrixOrder::Columns);
    let cells: Vec<char> = pasted.cells.chars().collect();
    // Column A is the first seven entries; row 1 takes every seventh.
    assert_eq!(
        (0..7)
            .map(|row| cells[row * MATRIX_COLUMNS])
            .collect::<String>(),
        "LCIVTBA"
    );
    assert_eq!(cells[..10].iter().collect::<String>(), "LQTIWFLRZP");
    for (cell, &found) in cells.iter().enumerate() {
        let (row, column) = (cell / MATRIX_COLUMNS, cell % MATRIX_COLUMNS);
        assert_eq!(found, export.as_bytes()[column * MATRIX_ROWS + row] as char);
    }

    let entries = |count: usize, entry: &str| serde_json::to_string(&vec![entry; count]).unwrap();
    for bad in [
        "A".repeat(69),
        "A".repeat(71),
        format!("{}-", "A".repeat(69)),
        format!("{}É", "A".repeat(69)),
        entries(69, "A"),
        entries(70, "AB"),
        entries(70, "1"),
    ] {
        assert!(parse_matrix_text(&bad).is_err(), "{bad:?}");
    }
}
