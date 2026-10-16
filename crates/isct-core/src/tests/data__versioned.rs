use super::*;
use serde_json::json;

fn row(value: Value, updated_at: &str, rev: u64) -> VersionedValue {
    VersionedValue {
        hash: content_hash(&value).unwrap(),
        value,
        updated_at: updated_at.into(),
        updated_by: "test".into(),
        rev,
        source: None,
        deleted_at: None,
    }
}

#[test]
fn canonical_hash_is_stable() {
    let first: Value = serde_json::from_str(r#"{ "b": 2.0, "a": { "y": true, "x": 1 } }"#).unwrap();
    let second: Value = serde_json::from_str(r#"{"a":{"x":1.0,"y":true},"b":2}"#).unwrap();
    assert_eq!(
        content_hash(&first).unwrap(),
        content_hash(&second).unwrap()
    );
}

#[test]
fn update_honors_both_automatic_methods() {
    let current = row(json!({"name": "old"}), "2026-01-01T00:00:00Z", 1);
    let incoming = row(json!({"name": "new"}), "2026-01-02T00:00:00Z", 2);
    assert!(update(&current, &incoming, MergeMethod::UseLatest).changed);
    assert!(!update(&current, &incoming, MergeMethod::UseCurrent).changed);
    let manual = update(&current, &incoming, MergeMethod::ResolveManually);
    assert_eq!(manual.conflict[0].path, "name");
}
