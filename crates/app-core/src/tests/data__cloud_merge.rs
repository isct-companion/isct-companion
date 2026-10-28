use super::*;
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
fn restore_generation_prevents_stale_device_resurrection() {
    let old = json!({"tables":{"personal_events":[{"id":"p","rev":999,"updatedAt":"2099-01-01T00:00:00Z"}]},"localNotes":[{"id":"n"}]});
    let restored =
        json!({"sync":{"generation":{"counter":1,"id":"restore-a"}},"tables":{},"localNotes":[]});
    assert!(merge(&old, &restored)["tables"]["personal_events"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        contents(&merge(&old, &restored)),
        contents(&merge(&restored, &old))
    );
    assert!(merge(&old, &restored)["localNotes"]
        .as_array()
        .unwrap()
        .is_empty());
    let mut simultaneous = restored.clone();
    simultaneous["sync"]["generation"]["id"] = json!("restore-b");
    assert_eq!(
        contents(&merge(&restored, &simultaneous)),
        contents(&merge(&simultaneous, &restored))
    );
    // A restore in one workspace says nothing about another workspace's
    // records: they are joined, under the first-sorting id and its generation.
    let mut separate = old.clone();
    separate["sync"] = json!({"workspaceId":"b"});
    let mut restored = restored.clone();
    restored["sync"]["workspaceId"] = json!("a");
    let joined = merge(&separate, &restored);
    assert_eq!(joined, merge(&restored, &separate));
    assert_eq!(joined["sync"], restored["sync"]);
    assert_eq!(
        joined["tables"]["personal_events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(joined["localNotes"].as_array().unwrap().len(), 1);
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
    let a = json!({"tables":{"personal_events":[deleted],"settings":[{"key":"devicePreferences","value":{"density":"compact"}}]},"localNotes":[]});
    let b = json!({"tables":{"personal_events":[revived,{"id":"b","updatedAt":"2026-01-01T00:00:00Z"}]},"localNotes":[]});
    assert_eq!(contents(&merge(&a, &b)), contents(&merge(&b, &a)));
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
    assert_eq!(
        contents(&merge(&a, &b)),
        contents(&merge(&merge(&a, &b), &b))
    );
}
