use super::*;
use serde_json::json;
fn record(at: &str, rev: u64, device: &str) -> Value {
    json!({"id":"a", "updatedAt":at,"rev":rev,"updatedBy":device})
}
#[test]
fn later_revision_survives_a_clock_moving_backwards() {
    assert!(compare(
        &record("2020-01-01T00:00:00Z", 8, "phone"),
        &record("2030-01-01T00:00:00Z", 7, "laptop")
    )
    .is_gt());
}
#[test]
fn a_copy_never_removes_records_it_lacks_and_drops_legacy_fields() {
    // Schema 6 copies carried notes and restore generations; a newer
    // generation used to replace the other copy wholesale.
    let legacy = json!({"exportedAt":"2026-01-01T00:00:00Z","sync":{"workspaceId":"a","generation":{"counter":9,"id":"g"}},"tables":{},"localNotes":[{"id":"n"}]});
    let local = json!({"tables":{"personal_events":[record("2026-01-02T00:00:00Z", 1, "phone")]}});
    let merged = merge(&legacy, &local);
    assert_eq!(merged["tables"], merge(&local, &legacy)["tables"]);
    assert_eq!(
        merged["tables"]["personal_events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(merged["schemaVersion"], 7);
    assert_eq!(merged["exportedAt"], "2026-01-01T00:00:00Z");
    for field in ["sync", "localNotes"] {
        assert!(merged.get(field).is_none(), "{field}");
    }
    // The fingerprint counts records only, not when or by whom a copy was made.
    let mut later = merged.clone();
    later["exportedAt"] = json!("2026-02-01T00:00:00Z");
    assert_eq!(version(&merged).unwrap(), version(&later).unwrap());
    assert_eq!(version(&merged).unwrap(), version(&local).unwrap());
}
#[test]
fn edits_deletes_and_ties_converge() {
    let old = record("2026-01-01T00:00:00Z", 1, "z");
    let edit = record("2026-01-02T00:00:00Z", 2, "a");
    assert!(compare(&edit, &old).is_gt());
    let mut deleted = old.clone();
    deleted["deletedAt"] = json!("2026-01-03T00:00:00Z");
    deleted["rev"] = json!(3);
    assert!(compare(&deleted, &edit).is_gt());
    let revived = record("2026-01-04T00:00:00Z", 4, "a");
    assert!(compare(&revived, &deleted).is_gt());
    assert!(compare(&record("2026-01-02T09:00:00+09:00", 3, "a"), &edit).is_gt());
    assert!(compare(&record("2026-01-02T00:00:00Z", 2, "b"), &edit).is_gt());
    let a = json!({"tables":{"personal_events":[deleted],"settings":[{"key":"devicePreferences","value":{"density":"compact"}}]}});
    let b = json!({"tables":{"personal_events":[revived,{"id":"b","updatedAt":"2026-01-01T00:00:00Z"}]}});
    assert_eq!(merge(&a, &b), merge(&b, &a));
    assert_eq!(
        merge(&a, &b)["tables"]["personal_events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(merge(&a, &b)["tables"]["settings"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(merge(&a, &b), merge(&merge(&a, &b), &b));
}
