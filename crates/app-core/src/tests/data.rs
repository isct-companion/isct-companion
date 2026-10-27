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
    let path = std::env::temp_dir().join(format!("isct-companion-{name}-{}", Uuid::new_v4()));
    DataStore::initialize_dir(path).unwrap().0
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

// Early releases offered OneDrive and may have stored it as the backup
// provider. Such a row must still load, and saving the settings again must
// not be refused for a value the person can no longer change.
#[test]
fn a_retired_cloud_provider_reads_and_saves_as_none() {
    let store = store("retired-provider");
    let mut cloud = store.load_workspace().unwrap()["settings"]["cloud"].clone();
    cloud["provider"] = json!("onedrive");
    cloud["conflict"] = json!("keep-current");
    store
        .write(None, |transaction| {
            records::save_setting(transaction, "cloud", &cloud, "old-device").map(|_| ())
        })
        .unwrap();
    let workspace = store.load_workspace().unwrap();
    assert_eq!(workspace["settings"]["cloud"]["provider"], "none");
    assert_eq!(workspace["settings"]["cloud"]["conflict"], "keep-current");
    store.put_workspace_setting("cloud", cloud).unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["settings"]["cloud"]["provider"],
        "none"
    );
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
        )
        .unwrap();
    let old: Value = store.export_document(json!([])).unwrap();
    let mut restored = old.clone();
    restored["tables"]["plans"] = json!([]);
    restored["sync"] =
        json!({"workspaceId":"workspace-a","generation":{"counter":1,"id":"restore-a"}});
    store
        .merge_cloud_data(&restored, crate::data::WorkspaceChangeSource::CloudSync)
        .unwrap();
    store
        .merge_cloud_data(&old, crate::data::WorkspaceChangeSource::CloudSync)
        .unwrap();
    assert!(store.user_rows("plans").unwrap().is_empty());
    let current: Value = store.export_document(json!([])).unwrap();
    assert_eq!(current["sync"], restored["sync"]);
    assert!(store.load_workspace().unwrap()["deviceId"].is_string());
}
#[test]
fn cloud_merge_preserves_offline_edits_device_state_and_concurrent_local_writes() {
    let a = store("cloud-a");
    let b = store("cloud-b");
    a.put_workspace_setting("devicePreferences", json!({"density":"compact"}))
        .unwrap();
    b.put_workspace_setting("devicePreferences", json!({"density":"comfortable"}))
        .unwrap();
    let row_a = a
        .put_user_record(
            "plans",
            json!({"title":"A","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
        )
        .unwrap();
    b.put_user_record(
        "plans",
        json!({"title":"B","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
    )
    .unwrap();
    let exported_a: Value = a.export_document(json!([])).unwrap();
    let exported_b: Value = b.export_document(json!([])).unwrap();
    a.delete_user_record("plans", row_a["id"].as_str().unwrap())
        .unwrap();
    a.merge_cloud_data(
        &cloud_merge::merge(&exported_a, &exported_b),
        crate::data::WorkspaceChangeSource::CloudSync,
    )
    .unwrap();
    let latest: Value = a.export_document(json!([])).unwrap();
    b.merge_cloud_data(&latest, crate::data::WorkspaceChangeSource::CloudSync)
        .unwrap();
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
    let _ = fs::remove_dir_all(a.directory());
    let _ = fs::remove_dir_all(b.directory());
}

#[test]
fn keeps_a_resolved_folder_listing_once_and_reads_it_back() {
    let store = store("folder-files");
    let url = "https://lms.s.isct.ac.jp/2025/mod/folder/view.php?id=9";
    {
        let connection = store.test_connection();
        connection
            .execute(
                "INSERT INTO lms_items (id,value,hash,updated_at,updated_by,rev,source)
                     VALUES ('m9',?1,'h','2026-10-01T00:00:00Z','lms',1,'lms')",
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
        .delete_user_record("scheduled_tasks", "builtin:reference-subscriptions")
        .unwrap();
    // The next launch writes defaults again; a deleted built-in stays deleted.
    let store = DataStore::initialize_dir(store.directory().to_path_buf())
        .unwrap()
        .0;
    let workspace = store.load_workspace().unwrap();
    let tasks = workspace["scheduledTasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks
        .iter()
        .find(|task| task["id"] == "builtin:reference-subscriptions")
        .unwrap()["deletedAt"]
        .is_string());
    let _ = fs::remove_dir_all(store.directory());
}

#[test]
fn checked_record_mutations_reject_stale_writers() {
    let store = store("record-conflict");
    let row = store
        .put_user_record(
            "plans",
            json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
        )
        .unwrap();
    let untouched = store
        .put_user_record(
            "plans",
            json!({"title":"Two","start":"2026-10-07T10:00:00+09:00","duration":60,"exdates":[]}),
        )
        .unwrap();
    let mut first = row.clone();
    first["title"] = json!("First writer");
    store
        .put_user_record_checked(
            "plans",
            first,
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(1),
        )
        .unwrap();
    let mut stale = row;
    stale["title"] = json!("Stale writer");
    let error = store
        .put_user_record_checked(
            "plans",
            stale,
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(1),
        )
        .unwrap_err();
    assert!(error.to_string().contains("revision conflict"));
    // Rows are written one at a time: a write never deletes the rows it omits.
    let rows = store.user_rows("plans").unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|row| row["id"] == untouched["id"] && row["title"] == "Two"));
    assert!(rows.iter().any(|row| row["title"] == "First writer"));
    let _ = fs::remove_dir_all(store.directory());
}

#[test]
fn unchanged_checked_put_returns_the_stored_revision() {
    let store = store("record-unchanged");
    let row = store
        .put_user_record(
            "plans",
            json!({"title":"One","start":"2026-10-06T10:00:00+09:00","duration":60,"exdates":[]}),
        )
        .unwrap();
    let mut edited = row.clone();
    edited["title"] = json!("Edited");
    let edited = store
        .put_user_record_checked(
            "plans",
            edited,
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(1),
        )
        .unwrap();
    assert_eq!(edited["rev"], 2);
    let same = store
        .put_user_record_checked(
            "plans",
            edited.clone(),
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(2),
        )
        .unwrap();
    let stored = store.user_rows("plans").unwrap()[0].clone();
    assert_eq!(same["rev"], stored["rev"]);
    assert_eq!(same, stored);
    assert_eq!(same["rev"], 2);
    let mut next = same;
    next["title"] = json!("Edited again");
    let next = store
        .put_user_record_checked(
            "plans",
            next,
            crate::data::WorkspaceChangeSource::UserIntent,
            Some(2),
        )
        .unwrap();
    assert_eq!(next["rev"], 3);
    assert_eq!(
        store.user_rows("plans").unwrap()[0]["title"],
        "Edited again"
    );
    let _ = fs::remove_dir_all(store.directory());
}

fn insert_school_assignment(store: &DataStore) {
    let mut connection = store.test_connection();
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
fn seeing_a_recent_item_records_its_version_and_drops_entries_past_the_window() {
    let store = store("recent-seen");
    insert_school_assignment(&store);
    let fresh = chrono::Utc::now().to_rfc3339();
    let mut connection = store.test_connection();
    let transaction = connection.transaction().unwrap();
    upsert_row(
        &transaction,
        "assignments",
        &json!({
            "id":"assignment-2", "kind":"assignment", "courseId":"course-1",
            "title":"Graded", "details":"", "dueAt":null,
            "sourceKey":"lms:default:assignment:43", "source":{"provider":"lms"},
            "rev":1, "updatedAt":fresh, "updatedBy":"lms"
        }),
        true,
    )
    .unwrap();
    transaction.commit().unwrap();
    let service = crate::workspace::WorkspaceService::new(store.clone());
    let mark = |id: &str| {
        service.mutate(crate::workspace::WorkspaceMutation {
            expected_revision: service.snapshot().unwrap().revision,
            intent: crate::workspace::WorkspaceIntent::MarkRecentSeen {
                list: crate::schedule::RecentKind::Grade,
                id: id.into(),
            },
        })
    };
    let first = mark("assignment-1").unwrap();
    assert_eq!(
        first.workspace["recentSeen"]["grade:assignment-1"],
        "2026-10-06T00:00:00Z"
    );
    // The first entry is older than the recent window, so it cannot show
    // again and the next mark drops it.
    let second = mark("assignment-2").unwrap();
    assert_eq!(
        second.workspace["recentSeen"],
        json!({"grade:assignment-2": fresh})
    );
    assert!(mark("missing").is_err());
}

#[test]
fn course_identity_is_shared_by_every_provider() {
    let store = store("course-identity");
    let mut connection = store.test_connection();
    let transaction = connection.transaction().unwrap();
    let lms = resolve_course_id(&transaction, "lms", "42", 2026, Some("202602440")).unwrap();
    let kyomu =
        resolve_course_id(&transaction, "kyomu", "202602440", 2026, Some("202602440")).unwrap();
    transaction.commit().unwrap();
    assert_eq!(lms, "2026-202602440");
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
    assert_eq!(rows, 2);
    let _ = fs::remove_dir_all(store.directory());
}

#[test]
fn assignment_pin_round_trips_as_a_syncable_overlay() {
    let store = store("pinned-assignment");
    insert_school_assignment(&store);
    store
        .put_user_record(
            "assignment_marks",
            json!({"sourceKey":"lms:default:assignment:42","hidden":false,"pinned":true}),
        )
        .unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"][0]["pinned"],
        true
    );
    let exported: Value = store.export_document(json!([])).unwrap();
    assert_eq!(exported["tables"]["assignment_marks"][0]["pinned"], true);
    store
        .put_user_record(
            "assignment_marks",
            json!({"sourceKey":"lms:default:assignment:42","hidden":false,"pinned":false}),
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
        )
        .unwrap();
    assert_eq!(
        store.load_workspace().unwrap()["assignments"][0]["hidden"],
        true
    );

    let connection = store.test_connection();
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
    let _ = fs::remove_dir_all(store.directory());
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
        )
        .unwrap();
    store
        .put_user_record(
            "assignment_marks",
            json!({
                "sourceKey":"lms:default:assignment:42", "hidden":true
            }),
        )
        .unwrap();
    let document = store
        .export_document(json!([{
            "id":"018f0000-0000-7000-8000-000000000001", "term":"2026-3Q",
            "course":"TEST", "section":"section-1", "text":"# local", "deleted":false
        }]))
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
    let _ = fs::remove_dir_all(store.directory());
}

#[test]
fn invalid_study_history_rolls_back_restore_and_cloud_replacement() {
    let store = store("cloud-study-history");
    store
        .put_user_record(
            "plans",
            json!({
                "title":"Keep me", "start":"2026-10-06T10:00:00+09:00",
                "duration":60, "exdates":[]
            }),
        )
        .unwrap();
    let before = store.load_workspace().unwrap();
    let mut backup = store.export_document(json!([])).unwrap();
    backup["sync"] = json!({"workspaceId":"workspace-a","generation":{"counter":1,"id":"remote"}});
    // Restoring a local backup and replacing from the cloud both apply through
    // `merge_cloud_data`, so each invalid history must leave the store as it was.
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
        let mut remote = backup.clone();
        let profile = remote["tables"]["settings"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["key"] == "academicProfile")
            .unwrap();
        profile["value"]["studyEnrollments"] = invalid;
        assert!(matches!(
            store.merge_cloud_data(&remote, crate::data::WorkspaceChangeSource::CloudSync),
            Err(DataError::InvalidInput(_))
        ));
        let after = store.load_workspace().unwrap();
        assert_eq!(after["academicProfile"], before["academicProfile"]);
        assert_eq!(after["plans"], before["plans"]);
    }
    let _ = fs::remove_dir_all(store.directory());
}

#[test]
fn legacy_backup_without_study_history_still_restores() {
    let source = store("legacy-study-source");
    source
        .put_workspace_setting(
            "academicProfile",
            json!({"name":"Legacy","admissionYear":2024,"gradePath":[],"years":[]}),
        )
        .unwrap();
    let mut document =
        parse_import(&source.export_document(json!([])).unwrap().to_string()).unwrap();
    document["sync"] =
        json!({"workspaceId":"workspace-a","generation":{"counter":1,"id":"restore"}});
    let target = store("legacy-study-target");
    target
        .merge_cloud_data(&document, crate::data::WorkspaceChangeSource::CloudSync)
        .unwrap();
    let restored = target.load_workspace().unwrap();
    assert_eq!(restored["academicProfile"]["name"], "Legacy");
    assert!(restored["academicProfile"]
        .get("studyEnrollments")
        .is_none());
    let _ = fs::remove_dir_all(source.directory());
    let _ = fs::remove_dir_all(target.directory());
}

#[test]
fn backup_from_another_schema_is_rejected_by_its_version() {
    let store = store("backup-schema");
    let mut document: Value = store.export_document(json!([])).unwrap();
    document["schemaVersion"] = json!(5);
    // Older backups also carried localNotes in a different shape.
    document["localNotes"] = json!({ "notes": [] });
    assert_eq!(
        parse_import(&document.to_string()).unwrap_err().to_string(),
        "the backup uses schema version 5; this app reads version 6"
    );
    let _ = fs::remove_dir_all(store.directory());
}

#[derive(Default)]
struct HeardCommits(std::sync::Mutex<Vec<std::collections::BTreeSet<String>>>);

impl CommitObserver for HeardCommits {
    fn committed(
        &self,
        _source: WorkspaceChangeSource,
        _revision: Option<String>,
        tables: &std::collections::BTreeSet<String>,
    ) {
        self.0.lock().unwrap().push(tables.clone());
    }
}

/// Each commit reports the tables it changed, in both files, by the names
/// SQL uses: a table emptied without `WHERE` counts, an FTS index reports its
/// virtual table rather than its shadow tables, and a commit that changed
/// nothing is not reported.
#[test]
fn commits_report_the_tables_they_changed() {
    let store = store("changed-tables");
    let heard = Arc::new(HeardCommits::default());
    store.observe(heard.clone());
    let write = |sql: &str| {
        store
            .write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
                transaction.execute_batch(sql).map_err(DataError::from)
            })
            .unwrap()
    };
    write(
        "INSERT INTO offerings_fts(offering_id,code,title_ja,title_en,instructors_ja,instructors_en)
           VALUES('2026-202601234','MTH.A201','線形代数学第一','Linear Algebra I','','');
         INSERT INTO course_identity_map
           VALUES('lms','42',2026,'2026-lms-42',NULL,'2026-10-01T00:00:00Z');",
    );
    write("DELETE FROM course_identity_map");
    write("DELETE FROM course_identity_map");

    let names = |tables: &[&str]| -> std::collections::BTreeSet<String> {
        tables.iter().map(|table| (*table).to_string()).collect()
    };
    assert_eq!(
        *heard.0.lock().unwrap(),
        vec![
            names(&["course_identity_map", "offerings_fts"]),
            names(&["course_identity_map"]),
        ]
    );
    let _ = fs::remove_dir_all(store.directory());
}

/// Freshness lives in sync.db, so a sync commit that only records it reaches
/// the next snapshot over the cached workspace: same workspace, new content
/// revision, same intent revision. A cache.db commit still rebuilds it.
#[test]
fn a_freshness_commit_refreshes_the_snapshot_without_rebuilding_its_workspace() {
    use crate::sync_state::{Commit, Dataset, Key, Outcome};
    let store = store("sync-records");
    let ((workspace, _, revision, _), content) = store.workspace_snapshot().unwrap();
    store
        .write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
            let commit = Commit {
                account: "",
                validator: None,
                outcome: Outcome::Complete,
                message: None,
            };
            crate::sync_state::commit(
                transaction,
                &Key::new(Dataset::KyomuPage, "grades"),
                commit,
                Utc::now(),
            )
        })
        .unwrap();

    let ((refreshed, records, refreshed_revision, _), refreshed_content) =
        store.workspace_snapshot().unwrap();
    assert!(Arc::ptr_eq(&workspace, &refreshed));
    assert_eq!(refreshed_revision, revision);
    assert_ne!(refreshed_content, content);
    assert!(records["freshness"].get("kyomu.page/grades").is_some());
    assert_eq!(
        records["pageAvailability"]["kyomu:grades"]["state"],
        "available"
    );

    store
        .write(Some(WorkspaceChangeSource::SchoolPull), |transaction| {
            transaction
                .execute_batch(
                    "INSERT INTO grades(id,value,hash,updated_at,updated_by,rev)
                       VALUES('grade:1','{\"id\":\"grade:1\"}','hash','2026-10-01T00:00:00Z','kyomu',1);",
                )
                .map_err(DataError::from)
        })
        .unwrap();
    let ((rebuilt, records, _, _), _) = store.workspace_snapshot().unwrap();
    assert!(!Arc::ptr_eq(&refreshed, &rebuilt));
    assert_eq!(records["grades"].as_array().map(Vec::len), Some(1));
    let _ = fs::remove_dir_all(store.directory());
}
