use super::*;
use crate::data::DataStore;

fn tables(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// A table missing from `TABLES` is refreshed as everything, which hides the
/// omission from every other test. Both database files are listed here, so a
/// new table fails until it says which views it feeds.
#[test]
fn every_table_in_both_database_files_says_what_it_feeds() {
    let directory =
        std::env::temp_dir().join(format!("isct-core-changes-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    let names = store
        .write(None, |transaction| {
            let mut statement = transaction.prepare(
                "SELECT name FROM main.sqlite_master WHERE type = 'table'
                 UNION SELECT name FROM cache.sqlite_master WHERE type = 'table'",
            )?;
            let names = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, crate::data::DataError>(names)
        })
        .unwrap();
    let unlisted: Vec<_> = names
        .iter()
        .filter(|name| !name.starts_with("sqlite_") && entry(name).is_none())
        .collect();
    assert!(
        unlisted.is_empty(),
        "tables missing from TABLES: {unlisted:?}"
    );
    // Every name a shell can receive is produced by some table.
    let produced = changes_for(&names.into_iter().collect());
    assert_eq!(produced, WorkspaceChange::ALL.into_iter().collect());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn tables_map_to_change_names_and_cloud_capture() {
    // Rows of one commit merge; the FTS index's shadow tables count as the
    // catalog they index.
    assert_eq!(
        changes_for(&tables(&[
            "assignment_marks",
            "assignments",
            "offerings_fts_data"
        ])),
        [Assignments, Finder].into_iter().collect()
    );
    // A run record alone changes nothing a shell shows, and no user data.
    assert!(changes_for(&tables(&["sync_runs"])).is_empty());
    assert!(!touches_user_data(&tables(&[
        "sync_runs",
        "dataset_state",
        "grades"
    ])));
    assert!(touches_user_data(&tables(&["dataset_state", "plan_items"])));
    // An unknown table is over-refreshed and captured, never missed.
    let unknown = tables(&["catalog_staging"]);
    assert_eq!(
        changes_for(&unknown),
        WorkspaceChange::ALL.into_iter().collect()
    );
    assert!(touches_user_data(&unknown));
    // Names round-trip, and the wire spelling is the serde one.
    for change in WorkspaceChange::ALL {
        assert_eq!(WorkspaceChange::parse(change.name()), Some(change));
        assert_eq!(serde_json::to_value(change).unwrap(), change.name());
    }
    assert_eq!(WorkspaceChange::parse("lectures"), None);
}

#[test]
fn a_patch_carries_only_the_fields_its_changes_feed() {
    let snapshot = WorkspaceSnapshot {
        revision: "r1".into(),
        content_revision: "c1".into(),
        workspace: serde_json::json!({
            "courses": [1], "memberships": [], "assignments": [], "materials": [],
            "planItems": [], "terms": [], "currentTerm": "2026-3Q",
            "settings": {"locale": "ja"}, "exams": [], "offerings": [],
        }),
        school_records: serde_json::json!({
            "lmsCourses": [], "grades": [], "freshness": {}, "pageAvailability": {},
        }),
    };
    let keys = |value: &Value| {
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };

    let freshness = patch(snapshot.clone(), &[Freshness]);
    assert_eq!(
        (
            freshness.revision.as_str(),
            freshness.content_revision.as_str()
        ),
        ("r1", "c1")
    );
    assert!(keys(&freshness.workspace).is_empty());
    assert_eq!(
        keys(&freshness.school_records),
        ["freshness", "pageAvailability"]
    );

    // Courses re-read everything the projection rewrites, not just courses.
    let courses = patch(snapshot.clone(), &[Courses, Exams]);
    assert_eq!(
        keys(&courses.workspace),
        [
            "assignments",
            "courses",
            "currentTerm",
            "exams",
            "materials",
            "memberships",
            "planItems",
            "terms"
        ]
    );
    assert_eq!(keys(&courses.school_records), ["lmsCourses"]);

    // Settings shape every projected field: the whole workspace, no records.
    let settings = patch(snapshot.clone(), &[Settings]);
    assert_eq!(settings.workspace, snapshot.workspace);
    assert!(keys(&settings.school_records).is_empty());
}

/// The first background change goes out at once; the rest of its window is
/// merged per source and delivered when the window ends; a quiet window
/// lets the next change through at once again.
#[test]
fn bursts_are_delivered_once_per_window_and_source() {
    let window = Duration::from_millis(400);
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let sink = delivered.clone();
    let coalescer = Coalescer::new(window, move |source, changes| {
        sink.lock().unwrap().push((source, changes));
    });
    let taken = || std::mem::take(&mut *delivered.lock().unwrap());
    use WorkspaceChangeSource::{ReferencePull, SchoolPull};

    coalescer.push(SchoolPull, [Courses].into());
    assert_eq!(taken(), [(SchoolPull, vec![Courses])]);

    coalescer.push(SchoolPull, [Grades].into());
    coalescer.push(ReferencePull, [Calendar].into());
    coalescer.push(SchoolPull, [Courses, Freshness].into());
    assert!(taken().is_empty(), "held until the window ends");

    std::thread::sleep(window * 2);
    assert_eq!(
        taken(),
        [
            (SchoolPull, vec![Courses, Grades, Freshness]),
            (ReferencePull, vec![Calendar]),
        ]
    );

    // The trailing delivery opened a window; once it passes quietly, the
    // trailing thread has ended and the next change is not held.
    std::thread::sleep(window * 2);
    assert!(taken().is_empty());
    coalescer.push(SchoolPull, [Exams].into());
    assert_eq!(taken(), [(SchoolPull, vec![Exams])]);
}
