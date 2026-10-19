use super::*;

#[test]
fn canonical_hash_is_stable() {
    let first: Value = serde_json::from_str(r#"{ "b": 2.0, "a": { "y": true, "x": 1 } }"#).unwrap();
    let second: Value = serde_json::from_str(r#"{"a":{"x":1.0,"y":true},"b":2}"#).unwrap();
    assert_eq!(
        content_hash(&first).unwrap(),
        content_hash(&second).unwrap()
    );
}
