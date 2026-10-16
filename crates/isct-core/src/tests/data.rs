use super::*;

#[test]
fn corrupt_stored_rows_are_storage_failures() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE plans (id TEXT, value TEXT);
             CREATE TABLE assignments (id TEXT, value TEXT, missing_since TEXT);
             INSERT INTO plans VALUES ('broken', 'not JSON');
             INSERT INTO assignments VALUES ('broken', 'not JSON', NULL);",
        )
        .unwrap();
    for result in [
        read_rows(&connection, "plans"),
        read_visible_rows(&connection, "assignments"),
    ] {
        let error = result.unwrap_err();
        assert!(matches!(error, DataError::Storage(_)));
        assert!(error.to_string().ends_with("contains invalid JSON"));
    }
}
use std::fs;

fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("isct-companion-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

#[test]
fn configured_clock_fills_legacy_rows_without_retiming_explicit_meetings() {
    let mut teaching_periods = crate::schedule::default_teaching_periods();
    teaching_periods[2].start_time = "13:00".into();
    let mut workspace = json!({
        "academicProfile":{"teachingPeriods":teaching_periods},
        "courses":[{"meetings":[
            {"periods":[5,6,7,8], "startTime":"", "endTime":""},
            {"periods":[], "startTime":"07:30", "endTime":"09:00"}
        ]}]
    });
    normalize_meeting_clocks(&mut workspace).unwrap();
    assert_eq!(workspace["courses"][0]["meetings"][0]["startTime"], "13:00");
    assert_eq!(workspace["courses"][0]["meetings"][0]["endTime"], "17:05");
    assert_eq!(workspace["courses"][0]["meetings"][1]["startTime"], "07:30");
    assert_eq!(workspace["courses"][0]["meetings"][1]["endTime"], "09:00");
}

#[test]
fn reference_poll_clock_preserves_other_device_preferences() {
    let store = store("reference-poll");
    store
        .put_workspace_setting(
            "devicePreferences",
            json!({
                "appearance":"dark", "locale":"ja", "subscriptionRefresh":{
                    "frequency":"daily", "allowCellular":true
                }
            }),
            "test",
        )
        .unwrap();
    store.record_reference_poll().unwrap();
    let workspace = store.load_workspace().unwrap();
    assert_eq!(workspace["settings"]["appearance"], "dark");
    assert_eq!(workspace["settings"]["locale"], "ja");
    assert_eq!(
        workspace["settings"]["subscriptionRefresh"]["allowCellular"],
        true
    );
    assert!(DateTime::parse_from_rfc3339(
        workspace["settings"]["subscriptionRefresh"]["lastCheckedAt"]
            .as_str()
            .unwrap()
    )
    .is_ok());
}

#[test]
fn restore_generation_is_committed_with_replacement_and_rejects_old_uploads() {
    let store = store("restore-generation");
    store
        .put_user_record(
            "plans",
            json!({"title":"Later","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
            "test",
        )
        .unwrap();
    let old: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
    let mut restored = old.clone();
    restored["tables"]["plans"] = json!([]);
    restored["sync"] =
        json!({"workspaceId":"workspace-a","generation":{"counter":1,"id":"restore-a"}});
    store.merge_cloud_data(&restored).unwrap();
    store.merge_cloud_data(&old).unwrap();
    assert!(store.user_rows("plans").unwrap().is_empty());
    let current: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
    assert_eq!(current["sync"], restored["sync"]);
    assert!(store.load_workspace().unwrap()["deviceId"].is_string());
}
#[test]
fn cloud_merge_preserves_offline_edits_device_state_and_concurrent_local_writes() {
    let a = store("cloud-a");
    let b = store("cloud-b");
    a.put_workspace_setting("devicePreferences", json!({"density":"compact"}), "test")
        .unwrap();
    b.put_workspace_setting(
        "devicePreferences",
        json!({"density":"comfortable"}),
        "test",
    )
    .unwrap();
    let row_a = a
        .put_user_record(
            "plans",
            json!({"title":"A","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
            "desktop",
        )
        .unwrap();
    b.put_user_record(
        "plans",
        json!({"title":"B","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
        "desktop",
    )
    .unwrap();
    let exported_a: Value = serde_json::from_str(&a.export_data(json!([])).unwrap()).unwrap();
    let exported_b: Value = serde_json::from_str(&b.export_data(json!([])).unwrap()).unwrap();
    a.delete_user_record("plans", row_a["id"].as_str().unwrap(), "desktop")
        .unwrap();
    a.merge_cloud_data(&cloud_merge::merge(&exported_a, &exported_b))
        .unwrap();
    let latest: Value = serde_json::from_str(&a.export_data(json!([])).unwrap()).unwrap();
    b.merge_cloud_data(&latest).unwrap();
    assert_eq!(
        a.user_rows("plans")
            .unwrap()
            .iter()
            .filter(|row| row.get("deletedAt").is_none())
            .count(),
        1
    );
    assert_eq!(a.user_rows("plans").unwrap(), b.user_rows("plans").unwrap());
    assert_eq!(
        a.load_workspace().unwrap()["settings"]["density"],
        "compact"
    );
    assert_eq!(
        b.load_workspace().unwrap()["settings"]["density"],
        "comfortable"
    );
    assert_eq!(latest["tables"]["plans"].as_array().unwrap().len(), 2); // includes tombstone
    let _ = fs::remove_file(a.path());
    let _ = fs::remove_file(b.path());
}

#[test]
fn keeps_a_resolved_folder_listing_once_and_reads_it_back() {
    let store = store("folder-files");
    let url = "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=9";
    {
        let connection = store.open().unwrap();
        connection
            .execute(
                "INSERT INTO lms_items (id,cmid,value,hash,updated_at,updated_by,rev,source)
                     VALUES ('m9',9,?1,'h','2026-10-01T00:00:00Z','lms',1,'lms')",
                [json!({"id":"m9","cmid":9,"module":"folder","url":url}).to_string()],
            )
            .unwrap();
    }
    let files = vec![crate::contracts::FolderFile {
        name: "20260928.zip".into(),
        url: "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_folder/content/0/20260928.zip"
            .into(),
    }];
    assert!(store.update_lms_folder_files(9, url, &files).unwrap());
    // The same listing again is not a change.
    assert!(!store.update_lms_folder_files(9, url, &files).unwrap());
    let row = store.lms_material_for_url(url).unwrap().unwrap();
    assert_eq!(row["rev"], 2);
    assert_eq!(row["folderFiles"][0]["name"], "20260928.zip");
    // A different source URL never takes another material's listing.
    assert!(!store
        .update_lms_folder_files(
            9,
            "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=10",
            &files
        )
        .unwrap());
}

#[test]
fn summary_metadata_keeps_other_search_languages() {
    let mut summary = json!({
        "title": {"en": "Assembly Language"},
        "instructors": [{"en": "Teacher"}]
    });
    let search = json!({
        "title": {"ja": "アセンブリ言語", "en": "Old English"},
        "instructors": [{"ja": "教員", "en": "Old Teacher"}]
    });
    fill_missing(&mut summary, &search);
    assert_eq!(summary["title"]["en"], "Assembly Language");
    assert_eq!(summary["title"]["ja"], "アセンブリ言語");
    assert_eq!(summary["instructors"][0]["ja"], "教員");
}

#[test]
fn current_term_uses_profile_then_calendar_then_selected_term() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 4).unwrap();
    let profile = json!({
        "academicProfile": {"years":[{"academicYear":2026,"quarterStarts":{
            "1Q":"2026-04-01","2Q":"2026-06-05","3Q":"2026-10-01","4Q":"2026-12-01"
        }}]},
        "academicCalendar":[{"date":"2026-06-01","type":"quarter-start","academicYear":2026,"quarter":"2Q"}]
    });
    assert_eq!(current_term_for(&profile, date).as_deref(), Some("2026-1Q"));
    let calendar = json!({
        "academicProfile":{"years":[]},
        "academicCalendar":[{"date":"2026-06-01","type":"quarter-start","academicYear":2026,"quarter":"2Q"}]
    });
    assert_eq!(
        current_term_for(&calendar, date).as_deref(),
        Some("2026-2Q")
    );
    assert_eq!(
        current_term_for(
            &json!({
                "selectedTerm":"2027-3Q",
                "academicProfile":{"years":[{"academicYear":2027,"quarterStarts":{
                    "1Q":"","2Q":"","3Q":"","4Q":""
                }}]},
                "academicCalendar":[]
            }),
            chrono::NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()
        ),
        Some("2027-3Q".to_string())
    );
}

#[test]
fn installs_reference_refresh_as_a_user_controllable_scheduled_task() {
    let store = store("reference-refresh-task");
    let workspace = store.load_workspace().unwrap();
    let tasks = workspace["scheduledTasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    let reference = tasks
        .iter()
        .find(|task| task["id"] == "builtin:reference-subscriptions")
        .unwrap();
    assert_eq!(reference["action"], "reference-refresh");
    assert_eq!(reference["enabled"], true);
    let lms = tasks
        .iter()
        .find(|task| task["id"] == "builtin:lms-background-refresh")
        .unwrap();
    assert_eq!(lms["action"], "lms-refresh");
    assert_eq!(lms["enabled"], false);

    store
        .delete_user_record("scheduled_tasks", "builtin:reference-subscriptions", "test")
        .unwrap();
    store.ensure_defaults().unwrap();
    let workspace = store.load_workspace().unwrap();
    let tasks = workspace["scheduledTasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks
        .iter()
        .find(|task| task["id"] == "builtin:reference-subscriptions")
        .unwrap()["deletedAt"]
        .is_string());
    let _ = fs::remove_file(store.path());
}

#[test]
fn record_mutations_never_delete_absent_rows() {
    let store = store("record-mutations");
    let first = store
        .put_user_record(
            "plans",
            json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
            "test",
        )
        .unwrap();
    let second = store
        .put_user_record(
            "plans",
            json!({"title":"Two","start":"2026-10-07T10:00:00+09:00","duration":60,"exdates":[]}),
            "test",
        )
        .unwrap();
    let mut changed = first.clone();
    changed["title"] = json!("Updated");
    store.put_user_record("plans", changed, "test").unwrap();
    let rows = store.user_rows("plans").unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row["id"] == second["id"]));
    let _ = fs::remove_file(store.path());
}

#[test]
fn checked_record_mutations_reject_stale_writers() {
    let store = store("record-conflict");
    let row = store
        .put_user_record(
            "plans",
            json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
            "test",
        )
        .unwrap();
    let mut first = row.clone();
    first["title"] = json!("First writer");
    store
        .put_user_record_checked("plans", first, "first", Some(1))
        .unwrap();
    let mut stale = row;
    stale["title"] = json!("Stale writer");
    let error = store
        .put_user_record_checked("plans", stale, "stale", Some(1))
        .unwrap_err();
    assert!(error.to_string().contains("revision conflict"));
    assert_eq!(
        store.user_rows("plans").unwrap()[0]["title"],
        "First writer"
    );
    let _ = fs::remove_file(store.path());
}

#[test]
fn unchanged_checked_put_returns_the_stored_revision() {
    let store = store("record-unchanged");
    let row = store
        .put_user_record(
            "plans",
            json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
            "test",
        )
        .unwrap();
    let mut edited = row.clone();
    edited["title"] = json!("Edited");
    let edited = store
        .put_user_record_checked("plans", edited, "desktop", Some(1))
        .unwrap();
    assert_eq!(edited["rev"], 2);
    let same = store
        .put_user_record_checked("plans", edited.clone(), "desktop", Some(2))
        .unwrap();
    let stored = store.user_rows("plans").unwrap()[0].clone();
    assert_eq!(same["rev"], stored["rev"]);
    assert_eq!(same, stored);
    assert_eq!(same["rev"], 2);
    let mut next = same;
    next["title"] = json!("Edited again");
    let next = store
        .put_user_record_checked("plans", next, "desktop", Some(2))
        .unwrap();
    assert_eq!(next["rev"], 3);
    assert_eq!(
        store.user_rows("plans").unwrap()[0]["title"],
        "Edited again"
    );
    let _ = fs::remove_file(store.path());
}

fn insert_school_assignment(store: &DataStore) {
    let mut connection = store.open().unwrap();
    let transaction = connection.transaction().unwrap();
    upsert_row(
        &transaction,
        "assignments",
        &json!({
            "id":"assignment-1", "kind":"assignment", "courseId":"course-1",
            "title":"Work", "details":"", "dueAt":null,
            "sourceKey":"lms:default:assignment:42", "source":{"provider":"lms"},
            "rev":1, "updatedAt":"2026-10-06T00:00:00Z", "updatedBy":"lms"
        }),
        true,
    )
    .unwrap();
    transaction.commit().unwrap();
}

#[test]
fn course_identity_is_shared_by_every_provider() {
    let store = store("course-identity");
    let mut connection = store.open().unwrap();
    let transaction = connection.transaction().unwrap();
    let lms = resolve_course_id(&transaction, "lms", "42", 2026, Some("202602440")).unwrap();
    let syllabus = resolve_course_id(
        &transaction,
        "syllabus",
        "202602440",
        2026,
        Some("202602440"),
    )
    .unwrap();
    let kyomu =
        resolve_course_id(&transaction, "kyomu", "202602440", 2026, Some("202602440")).unwrap();
    transaction.commit().unwrap();
    assert_eq!(lms, "2026-202602440");
    assert_eq!(syllabus, lms);
    assert_eq!(kyomu, lms);
    let rows: i64 = store
        .open()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM course_identity_map WHERE course_id='2026-202602440'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 3);
    let _ = fs::remove_file(store.path());
}

#[test]
fn catalog_list_includes_search_rows_until_details_replace_them() {
    let store = store("catalog-search-fallback");
    store.catalog_upsert_search_items(&[json!({
            "id":"202602427", "year":2026, "code":"CSC.T263",
            "title":{"en":"Assembly Language"}, "instructors":[],
            "quarters":["3Q"], "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/2026/CSC.T263/202602427"
        })]).unwrap();
    let rows = store.catalog_search("").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], "202602427");
    assert_eq!(rows[0]["detailed"], Value::Null);
    let _ = fs::remove_file(store.path());
}

#[test]
fn assignment_pin_round_trips_as_a_syncable_overlay() {
    let store = store("pinned-assignment");
    insert_school_assignment(&store);
    store
        .put_user_record(
            "assignment_marks",
            json!({"sourceKey":"lms:default:assignment:42","hidden":false,"pinned":true}),
            "test",
        )
        .unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"][0]["pinned"],
        true
    );
    let exported: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
    assert_eq!(exported["tables"]["assignment_marks"][0]["pinned"], true);
    store
        .put_user_record(
            "assignment_marks",
            json!({"sourceKey":"lms:default:assignment:42","hidden":false,"pinned":false}),
            "test",
        )
        .unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"][0]["pinned"],
        false
    );
}

#[test]
fn overlay_survives_school_disappearance_and_reappearance() {
    let store = store("overlay-lifetime");
    insert_school_assignment(&store);
    store
        .put_user_record(
            "assignment_marks",
            json!({
                "sourceKey":"lms:default:assignment:42", "hidden":true
            }),
            "test",
        )
        .unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"][0]["hidden"],
        true
    );

    let connection = store.open().unwrap();
    connection.execute(
            "UPDATE assignments SET missing_since='2026-10-07T00:00:00Z',value=json_set(value,'$.missingSince','2026-10-07T00:00:00Z') WHERE id='assignment-1'",
            [],
        ).unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(store.user_rows("assignment_marks").unwrap().len(), 1);

    connection.execute(
            "UPDATE assignments SET missing_since=NULL,value=json_remove(value,'$.missingSince') WHERE id='assignment-1'",
            [],
        ).unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"][0]["hidden"],
        true
    );
    let _ = fs::remove_file(store.path());
}

#[test]
fn local_backup_contains_user_data_overlays_and_notes_but_no_school_cache() {
    let store = store("backup-boundary");
    insert_school_assignment(&store);
    store
        .put_user_record(
            "plans",
            json!({
                "title":"Study", "start":"2026-10-06T10:00:00+09:00", "duration":60, "exdates":[]
            }),
            "test",
        )
        .unwrap();
    store
        .put_user_record(
            "assignment_marks",
            json!({
                "sourceKey":"lms:default:assignment:42", "hidden":true
            }),
            "test",
        )
        .unwrap();
    let document: Value = serde_json::from_str(
        &store
            .export_data(json!([{
                "id":"018f0000-0000-7000-8000-000000000001", "term":"2026-3Q",
                "course":"TEST", "section":"section-1", "text":"# local", "deleted":false
            }]))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(document["schemaVersion"], 6);
    assert_eq!(document["localNotes"][0]["text"], "# local");
    assert!(document.pointer("/tables/assignments").is_none());
    assert!(document.pointer("/tables/lms_items").is_none());
    assert_eq!(document["tables"]["plans"].as_array().unwrap().len(), 1);
    assert_eq!(
        document["tables"]["assignment_marks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let _ = fs::remove_file(store.path());
}

#[test]
fn invalid_study_history_rolls_back_local_restore() {
    let store = store("restore-study-history");
    store
        .put_user_record(
            "plans",
            json!({
                "title":"Keep me", "start":"2026-10-06T10:00:00+09:00",
                "duration":60, "exdates":[]
            }),
            "test",
        )
        .unwrap();
    let before = store.load_workspace().unwrap();
    let backup: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
    for invalid in [
        Value::Null,
        json!({"admittedTerm":"2026-1Q"}),
        json!([{
            "stage":"undergraduate", "entryKind":"regular", "entryGrade":1,
            "admittedTerm":"2026-spring"
        }]),
        json!([
            {"stage":"undergraduate", "entryKind":"regular", "entryGrade":1,
             "admittedTerm":"2026-1Q"},
            {"stage":"master", "entryKind":"regular", "entryGrade":1,
             "admittedTerm":"2026-1Q"}
        ]),
    ] {
        let mut document = backup.clone();
        let profile = document["tables"]["settings"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["key"] == "academicProfile")
            .unwrap();
        profile["value"]["studyEnrollments"] = invalid;
        let parsed = parse_import(&document.to_string()).unwrap();
        assert!(matches!(
            store.import_data(&parsed),
            Err(DataError::InvalidInput(_))
        ));
        let after = store.load_workspace().unwrap();
        assert_eq!(after["academicProfile"], before["academicProfile"]);
        assert_eq!(after["plans"], before["plans"]);
    }
    let _ = fs::remove_file(store.path());
}

#[test]
fn invalid_study_history_rolls_back_cloud_replacement() {
    let store = store("cloud-study-history");
    store
        .put_user_record(
            "plans",
            json!({
                "title":"Keep me", "start":"2026-10-06T10:00:00+09:00",
                "duration":60, "exdates":[]
            }),
            "test",
        )
        .unwrap();
    let before = store.load_workspace().unwrap();
    let mut remote: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
    remote["sync"] = json!({"workspaceId":"workspace-a","generation":{"counter":1,"id":"remote"}});
    let profile = remote["tables"]["settings"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["key"] == "academicProfile")
        .unwrap();
    profile["value"]["studyEnrollments"] = Value::Null;
    assert!(matches!(
        store.merge_cloud_data(&remote),
        Err(DataError::InvalidInput(_))
    ));
    let after = store.load_workspace().unwrap();
    assert_eq!(after["academicProfile"], before["academicProfile"]);
    assert_eq!(after["plans"], before["plans"]);
    let _ = fs::remove_file(store.path());
}

#[test]
fn legacy_backup_without_study_history_still_restores() {
    let source = store("legacy-study-source");
    source
        .put_workspace_setting(
            "academicProfile",
            json!({"name":"Legacy","admissionYear":2024,"gradePath":[],"years":[]}),
            "test",
        )
        .unwrap();
    let document = parse_import(&source.export_data(json!([])).unwrap()).unwrap();
    let target = store("legacy-study-target");
    let restored = target.import_data(&document).unwrap();
    assert_eq!(restored["academicProfile"]["name"], "Legacy");
    assert!(restored["academicProfile"]
        .get("studyEnrollments")
        .is_none());
    let _ = fs::remove_file(source.path());
    let _ = fs::remove_file(target.path());
}

#[test]
fn backup_from_another_schema_is_rejected_by_its_version() {
    let store = store("backup-schema");
    let mut document: Value = serde_json::from_str(&store.export_data(json!([])).unwrap()).unwrap();
    document["schemaVersion"] = json!(5);
    // Older backups also carried localNotes in a different shape.
    document["localNotes"] = json!({ "notes": [] });
    assert_eq!(
        parse_import(&document.to_string()).unwrap_err().to_string(),
        "the backup uses schema version 5; this app reads version 6"
    );
    let _ = fs::remove_file(store.path());
}
