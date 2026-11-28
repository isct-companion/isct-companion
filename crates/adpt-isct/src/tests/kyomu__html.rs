use super::*;

#[test]
fn numbers_keep_the_json_shape_the_browser_stored() {
    // Integral credits stay integers so stored row hashes do not churn, and a
    // value JavaScript's Number() rejects is null, as JSON.stringify(NaN) was.
    assert_eq!(js_number("2"), serde_json::json!(2));
    assert_eq!(js_number(" 1.5 "), serde_json::json!(1.5));
    assert_eq!(js_number("1.2.3"), Value::Null);
    assert_eq!(js_number("."), Value::Null);
}
