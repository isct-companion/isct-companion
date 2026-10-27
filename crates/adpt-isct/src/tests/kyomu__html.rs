use super::*;

fn grid(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|row| row.iter().map(|cell| cell.to_string()).collect())
        .collect()
}

#[test]
fn requirement_header_is_as_deep_as_the_corner_cell_spans() {
    // `<th rowspan="2">区分</th><th colspan="2">教養</th>` as `matrix` expands
    // it: the corner repeats down the two header rows.
    let spanned = grid(&[
        &["区分", "教養", "教養"],
        &["区分", "文系", "英語"],
        &["卒業要件", "2", "4"],
        &["修得済単位", "2", "2"],
    ]);
    let depth = corner_depth(&spanned);
    assert_eq!(depth, 2);
    let table = split_header(spanned, header_depth(depth, 0, Some(0)));
    assert_eq!(table.headers, ["区分", "教養 / 文系", "教養 / 英語"]);
    assert_eq!(table.rows.len(), 2);

    let flat = grid(&[&["区分", "計"], &["卒業要件", "40"]]);
    assert_eq!(corner_depth(&flat), 1);
    // No evidence of depth: a lone row, or no rows at all.
    assert_eq!(corner_depth(&grid(&[&["区分", "計"]])), 0);
    assert_eq!(corner_depth(&[]), 0);
}

#[test]
fn header_depth_prefers_known_depth_then_thead_then_first_data_row() {
    assert_eq!(header_depth(2, 1, Some(1)), 2);
    assert_eq!(header_depth(0, 1, Some(3)), 1);
    assert_eq!(header_depth(0, 0, Some(2)), 2);
    // A first row that already holds data, or no data row, still leaves one
    // header row.
    assert_eq!(header_depth(0, 0, Some(0)), 1);
    assert_eq!(header_depth(0, 0, None), 1);
}

#[test]
fn numbers_keep_the_json_shape_the_browser_stored() {
    // Integral credits stay integers so stored row hashes do not churn, and a
    // value JavaScript's Number() rejects is null, as JSON.stringify(NaN) was.
    assert_eq!(js_number("2"), serde_json::json!(2));
    assert_eq!(js_number(" 1.5 "), serde_json::json!(1.5));
    assert_eq!(js_number("1.2.3"), Value::Null);
    assert_eq!(js_number("."), Value::Null);
}
