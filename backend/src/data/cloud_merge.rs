//! Pure, platform-independent last-writer-wins merge for user-owned records.
use serde_json::{json, Value};
use std::{cmp::Ordering, collections::BTreeMap};

fn time(row: &Value) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    ["updatedAt", "deletedAt"]
        .iter()
        .filter_map(|key| {
            row.get(key)
                .and_then(Value::as_str)
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        })
        .max()
}

pub fn compare(a: &Value, b: &Value) -> Ordering {
    // Revisions advance from the imported record, so a subsequent edit still
    // wins if the device clock moved backwards. Concurrent equal revisions
    // use timestamps and a deterministic actor tie-break.
    a["rev"]
        .as_u64()
        .unwrap_or(0)
        .cmp(&b["rev"].as_u64().unwrap_or(0))
        .then_with(|| time(a).cmp(&time(b)))
        .then_with(|| {
            a["updatedBy"]
                .as_str()
                .unwrap_or("")
                .cmp(b["updatedBy"].as_str().unwrap_or(""))
        })
        // Malformed/legacy clocks can collide even on one device. A canonical
        // payload tie-break keeps the operation commutative in that case too.
        .then_with(|| a.to_string().cmp(&b.to_string()))
}

fn rows(a: &Value, b: &Value, key: &str) -> Value {
    let mut records = BTreeMap::<String, Value>::new();
    for row in [a, b].into_iter().filter_map(Value::as_array).flatten() {
        let Some(id) = row[key].as_str() else {
            continue;
        };
        if records.get(id).is_none_or(|old| compare(row, old).is_gt()) {
            records.insert(id.into(), row.clone());
        }
    }
    Value::Array(records.into_values().collect())
}

pub fn generation(document: &Value) -> (u64, String) {
    (
        document["sync"]["generation"]["counter"]
            .as_u64()
            .unwrap_or(0),
        document["sync"]["generation"]["id"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
    )
}

pub fn version(document: &Value) -> Result<String, String> {
    let normalized = merge(document, document);
    crate::data::versioned::content_hash(&contents(&normalized)).map(|hash| format!("v1-{hash}"))
}

pub fn merge(a: &Value, b: &Value) -> Value {
    // A restore starts a new generation; rows from the old branch cannot reappear.
    let (a, b) = match generation(a).cmp(&generation(b)) {
        Ordering::Less => (b, b),
        Ordering::Greater => (a, a),
        Ordering::Equal => (a, b),
    };
    let mut merged = a.clone();
    if merged["sync"].is_null() && !b["sync"].is_null() {
        merged["sync"] = b["sync"].clone();
    }
    let mut tables = serde_json::Map::new();
    // This whitelist is also used by backup/import: device state and school
    // caches cannot enter sync even through an older or foreign cloud copy.
    for table in crate::data::USER_BACKUP_TABLES {
        tables.insert(
            (*table).into(),
            rows(&a["tables"][table], &b["tables"][table], "id"),
        );
    }
    let mut settings = rows(&a["tables"]["settings"], &b["tables"]["settings"], "key");
    settings.as_array_mut().unwrap().retain(|row| {
        row["key"]
            .as_str()
            .is_some_and(|key| crate::data::USER_BACKUP_SETTINGS.contains(&key))
    });
    for setting in settings.as_array_mut().unwrap() {
        sanitize_setting(setting);
    }
    tables.insert("settings".into(), settings);
    merged["tables"] = Value::Object(tables);
    merged["localNotes"] = rows(&a["localNotes"], &b["localNotes"], "id");
    merged
}

pub fn sanitize_setting(row: &mut Value) {
    if row["key"] == "userPreferences" {
        if let Some(value) = row["value"].as_object_mut() {
            value.retain(|key, _| {
                matches!(
                    key.as_str(),
                    "departmentId" | "notifications" | "customization"
                )
            });
            if let Some(notifications) = value
                .get_mut("notifications")
                .and_then(Value::as_object_mut)
            {
                notifications.retain(|key, _| {
                    matches!(key.as_str(), "classes" | "assignments" | "classLeadMinutes")
                });
            }
        }
        // Settings carry their content hash separately from their clocks.
        if let Ok(hash) = crate::data::versioned::content_hash(&row["value"]) {
            row["hash"] = Value::String(hash);
        }
    }
}

pub fn contents(document: &Value) -> Value {
    json!({"tables":document["tables"], "localNotes":document["localNotes"], "generation": generation(document)})
}

#[cfg(test)]
mod tests {
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
    fn content_versions_ignore_export_time_device_and_row_order() {
        let a = json!({"exportedAt":"yesterday", "deviceId":"a", "tables":{"plans":[{"id":"b","rev":1},{"id":"a","rev":1}]},"localNotes":[]});
        let b = json!({"exportedAt":"today", "deviceId":"b", "tables":{"plans":[{"id":"a","rev":1},{"id":"b","rev":1}]},"localNotes":[]});
        assert_eq!(version(&a).unwrap(), version(&b).unwrap());
        let mut c = b.clone();
        c["tables"]["plans"][0]["rev"] = json!(2);
        assert_ne!(version(&a).unwrap(), version(&c).unwrap());
    }
    #[test]
    fn restore_generation_prevents_stale_device_resurrection() {
        let old = json!({"tables":{"plans":[{"id":"p","rev":999,"updatedAt":"2099-01-01T00:00:00Z"}]},"localNotes":[{"id":"n"}]});
        let restored = json!({"sync":{"generation":{"counter":1,"id":"restore-a"}},"tables":{},"localNotes":[]});
        assert!(merge(&old, &restored)["tables"]["plans"]
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
        let a = json!({"tables":{"plans":[deleted],"settings":[{"key":"devicePreferences","value":{"density":"compact"}}]},"localNotes":[]});
        let b = json!({"tables":{"plans":[revived,{"id":"b","updatedAt":"2026-01-01T00:00:00Z"}]},"localNotes":[]});
        assert_eq!(contents(&merge(&a, &b)), contents(&merge(&b, &a)));
        assert_eq!(
            merge(&a, &b)["tables"]["plans"].as_array().unwrap().len(),
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
}
