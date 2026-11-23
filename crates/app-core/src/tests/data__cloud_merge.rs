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

#[test]
fn deleting_the_last_legacy_draft_wins_over_both_old_device_ids() {
    let snapshot = |second_deleted: bool| {
        let mut a = record("2026-01-04T00:00:00Z", 4, "first");
        a["deletedAt"] = a["updatedAt"].clone();
        let mut b = if second_deleted {
            let mut b = record("2026-01-06T00:00:00Z", 2, "second");
            b["deletedAt"] = b["updatedAt"].clone();
            b
        } else {
            record("2026-01-05T00:00:00Z", 1, "second")
        };
        b["id"] = "b".into();
        for row in [&mut a, &mut b] {
            row["termId"] = "2026-3Q".into();
            row["offeringId"] = "202630010".into();
        }
        json!({"tables":{"plan_items":[a,b]}})
    };
    let older = snapshot(false);
    let deleted = snapshot(true);
    let merged = merge(&older, &deleted);
    assert_eq!(merged["tables"], merge(&deleted, &older)["tables"]);
    let plans = merged["tables"]["plan_items"].as_array().unwrap();
    assert_eq!(plans.len(), 1);
    assert!(plans[0]["deletedAt"].is_string());
    assert_eq!(merge(&merged, &older)["tables"], merged["tables"]);
    // A different device edited the stable record before it ever saw the
    // second legacy id. Old snapshots may have arbitrarily many revisions.
    let mut modern = merged.clone();
    let plan = &mut modern["tables"]["plan_items"][0];
    plan.as_object_mut().unwrap().remove("baselineRevision");
    plan["rev"] = 1.into();
    assert_eq!(merge(&older, &modern)["tables"], modern["tables"]);
    let mut mixed = older.clone();
    mixed["tables"]["plan_items"]
        .as_array_mut()
        .unwrap()
        .push(modern["tables"]["plan_items"][0].clone());
    assert_eq!(merge(&mixed, &older)["tables"], modern["tables"]);
    // A newer old snapshot can still update a device that only inherited
    // its baseline, even if both forms are in the same input document.
    let inherited = merge(&older, &older);
    let mut mixed = deleted.clone();
    mixed["tables"]["plan_items"]
        .as_array_mut()
        .unwrap()
        .push(inherited["tables"]["plan_items"][0].clone());
    assert_eq!(merge(&mixed, &older)["tables"], merged["tables"]);
}
