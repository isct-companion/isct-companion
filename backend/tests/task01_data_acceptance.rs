#![cfg(any())] // Historical v3 whole-snapshot acceptance suite.

#[path = "../src/data/mod.rs"]
mod data;

use data::DataStore;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_database(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "isct-companion-acceptance-{label}-{}-{nonce}.db",
        std::process::id()
    ))
}

fn with_store(label: &str, test: impl FnOnce(&DataStore)) {
    let path = temporary_database(label);
    let store = DataStore::initialize(path.clone())
        .expect("create synthetic test database")
        .0;
    test(&store);
    drop(store);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("db-shm"));
    let _ = fs::remove_file(path.with_extension("db-wal"));
}

fn synthetic_course(offering_id: &str) -> Value {
    json!({
        "id": "course-synthetic",
        "kind": "course",
        "rev": 1,
        "updatedAt": "2026-09-29T00:00:00.000Z",
        "updatedBy": "synthetic-device",
        "termId": "2026-3Q",
        "offeringId": offering_id,
        "code": "SYN.T101",
        "title": "Synthetic Systems",
        "instructor": "Sample Instructor",
        "credits": 1,
        "creditSplit": { "lecture": 1, "exercise": 0, "lab": 0 },
        "instructionLanguage": "en",
        "color": "#445566",
        "meetings": [],
        "source": { "provider": "manual" }
    })
}

#[test]
fn export_includes_only_an_offering_referenced_by_personal_or_school_rows() {
    with_store("referenced-offering", |store| {
        let mut workspace = store.load_workspace().expect("load workspace");
        workspace["courses"] = json!([synthetic_course("offering-referenced")]);
        store.save_workspace(&workspace).expect("save reference");
        store
            .catalog_upsert_offerings(&[
                json!({
                    "id": "offering-referenced",
                    "code": "SYN.T101",
                    "title": { "ja": "合成システム", "en": "Synthetic Systems" }
                }),
                json!({
                    "id": "offering-unreferenced",
                    "code": "SYN.T102",
                    "title": { "ja": "未参照", "en": "Unreferenced" }
                }),
            ])
            .expect("seed catalog");

        let export: Value =
            serde_json::from_str(&store.export_data().expect("export data")).expect("parse export");
        let ids = export["tables"]["offerings"]
            .as_array()
            .expect("offerings array")
            .iter()
            .filter_map(|row| row["id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["offering-referenced"]);
    });
}

#[test]
fn import_validation_is_atomic_when_a_late_table_is_unsupported() {
    with_store("atomic-import", |store| {
        let mut document: Value =
            serde_json::from_str(&store.export_data().expect("base full export"))
                .expect("parse base export");
        document["deviceId"] = Value::String("synthetic-remote".into());
        document["tables"]["timetable_courses"] = json!([synthetic_course("offering-remote")]);
        document["tables"]["unknown_school_table"] = json!([]);

        assert!(store.import_data(&document.to_string()).is_err());
        assert_eq!(
            store.load_workspace().expect("reload")["courses"],
            json!([]),
            "a rejected document must not leave earlier rows behind"
        );
    });
}

#[test]
fn import_rejects_missing_document_metadata_before_writing() {
    with_store("document-shape", |store| {
        let document = json!({
            "schemaVersion": 3,
            "deviceId": "synthetic-remote",
            "tables": { "timetable_courses": [synthetic_course("offering-remote")] }
        });

        assert!(
            store.import_data(&document.to_string()).is_err(),
            "exportedAt is part of the versioned document contract"
        );
        assert_eq!(
            store.load_workspace().expect("reload")["courses"],
            json!([])
        );
    });
}

#[test]
fn use_current_applies_to_settings_as_well_as_collection_rows() {
    with_store("settings-conflict", |store| {
        let mut workspace = store.load_workspace().expect("load workspace");
        workspace["settings"]["appearance"] = Value::String("dark".into());
        workspace["settings"]["cloud"]["conflict"] = Value::String("keep-current".into());
        store
            .save_workspace(&workspace)
            .expect("save local settings");

        let mut incoming_settings = workspace["settings"].clone();
        incoming_settings["appearance"] = Value::String("light".into());
        let mut document: Value =
            serde_json::from_str(&store.export_data().expect("base full export"))
                .expect("parse base export");
        document["exportedAt"] = Value::String("2026-09-29T01:00:00.000Z".into());
        document["deviceId"] = Value::String("synthetic-remote".into());
        let settings_row = document["tables"]["settings"]
            .as_array_mut()
            .expect("versioned settings rows")
            .iter_mut()
            .find(|row| row["key"] == "settings")
            .expect("settings row");
        settings_row["value"] = incoming_settings;
        settings_row["rev"] = Value::from(2);
        settings_row["updatedAt"] = Value::String("2026-09-29T01:00:00.000Z".into());
        settings_row["updatedBy"] = Value::String("synthetic-remote".into());

        let merged = store
            .import_data(&document.to_string())
            .expect("merge import");
        assert_eq!(merged["settings"]["appearance"], "dark");
    });
}

#[test]
fn assignment_marks_keep_personal_state_out_of_lms_assignments_and_clear_cleanly() {
    with_store("assignment-marks", |store| {
        let mut workspace = store.load_workspace().expect("load workspace");
        workspace["assignments"] = json!([{
            "id": "assignment-local-projection",
            "kind": "assignment",
            "rev": 1,
            "updatedAt": "2026-09-29T00:00:00.000Z",
            "updatedBy": "synthetic-device",
            "courseId": "course-local",
            "title": "Local projected assignment",
            "details": "",
            "dueAt": "2026-10-10T09:00:00+09:00",
            "submission": "open",
            "source": { "provider": "lms" },
            "hidden": true,
            "completedAt": "2026-09-29T01:00:00.000Z"
        }]);
        store
            .save_workspace(&workspace)
            .expect("save personal mark");

        let document: Value = serde_json::from_str(&store.export_data().unwrap()).unwrap();
        let assignment = &document["tables"]["assignments"][0];
        assert!(assignment.get("hidden").is_none());
        assert!(assignment.get("completedAt").is_none());
        let marks = document["tables"]["assignment_marks"].as_array().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0]["assignmentId"], "assignment-local-projection");
        assert_eq!(marks[0]["hidden"], true);
        assert_eq!(marks[0]["completedAt"], "2026-09-29T01:00:00.000Z");

        let mut composed = store.load_workspace().expect("compose assignment mark");
        assert_eq!(composed["assignments"][0]["hidden"], true);
        assert_eq!(
            composed["assignments"][0]["completedAt"],
            "2026-09-29T01:00:00.000Z"
        );
        let assignment = composed["assignments"][0].as_object_mut().unwrap();
        assignment.remove("hidden");
        assignment.remove("completedAt");
        store
            .save_workspace(&composed)
            .expect("clear personal mark");

        let cleared = store.load_workspace().expect("load cleared assignment");
        assert!(cleared["assignments"][0].get("hidden").is_none());
        assert!(cleared["assignments"][0].get("completedAt").is_none());
        let cleared_export: Value = serde_json::from_str(&store.export_data().unwrap()).unwrap();
        assert!(cleared_export["tables"]["assignment_marks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|mark| mark.get("deletedAt").and_then(Value::as_str).is_some()));
    });
}
