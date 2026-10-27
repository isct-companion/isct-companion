use super::*;
use crate::data::DataStore;

fn tables(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

/// A table missing from `TABLES` is refreshed as everything, which hides the
/// omission from every other test. Every database file is listed here, so a
/// new table fails until it says which views it feeds.
#[test]
fn every_table_in_every_database_file_says_what_it_feeds() {
    let directory = std::env::temp_dir().join(format!("app-core-changes-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    let names = store
        .write(None, |transaction| {
            let mut statement = transaction.prepare(
                "SELECT name FROM main.sqlite_master WHERE type = 'table'
                 UNION SELECT name FROM cache.sqlite_master WHERE type = 'table'
                 UNION SELECT name FROM sync.sqlite_master WHERE type = 'table'",
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

/// A records query reads again on every change that feeds one of its
/// fields (`feeding`): the course projection rewrites more than courses,
/// calendar rows move the current term, and settings feed everything.
#[test]
fn records_read_again_on_every_change_that_feeds_their_fields() {
    let feeding = |fields: &[&str]| -> BTreeSet<WorkspaceChange> {
        feeding(
            &fields
                .iter()
                .map(|field| (*field).to_owned())
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .collect()
    };
    let set = |changes: &[WorkspaceChange]| changes.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        feeding(&["freshness", "pageAvailability"]),
        set(&[Freshness, Settings])
    );
    // The projection rewrites course ids in the rows that point at courses;
    // the exam timetable stays in Core and feeds no record.
    for field in [
        "assignments",
        "courses",
        "currentTerm",
        "materials",
        "memberships",
        "planItems",
        "terms",
    ] {
        let fed = feeding(&[field]);
        assert!(fed.contains(&Courses), "{field}");
        assert!(!fed.contains(&Exams), "{field}");
    }
    assert_eq!(fed_fields(Exams), Some(vec![]));
    assert!(feeding(&["currentTerm"]).contains(&Calendar));
    assert!(!feeding(&["courses"]).contains(&Calendar));
    assert_eq!(feeding(&["lmsCourses"]), set(&[Courses, Settings]));
    // Settings shape every projected field.
    assert_eq!(fed_fields(Settings), None);
    assert!(feeding(&["settings"]).contains(&Settings));
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
