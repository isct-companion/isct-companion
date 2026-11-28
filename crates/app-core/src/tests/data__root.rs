use super::*;
use crate::data::DataStore;
use serde_json::json;
use std::fs;

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("isct-root-{name}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn event() -> serde_json::Value {
    json!({"title":"Study group","kind":"event","termId":"2026-3Q","room":"","days":[1],"startTime":"10:00","endTime":"11:00"})
}

/// One Core per data folder: a second one (a dev build beside the app) is
/// refused before it opens a database, and gets the folder once the first
/// lets go, as after a crash.
#[test]
fn a_second_core_is_refused_until_the_first_lets_go() {
    let directory = scratch("lock");
    let (store, created) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(created);
    store.put_user_record("personal_events", event()).unwrap();
    assert!(matches!(
        DataStore::initialize_dir(directory.clone()),
        Err(DataError::Open(StoreOpenError::DirectoryInUse))
    ));
    drop(store);
    let (store, created) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(!created, "the active generations are used as they are");
    assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

/// The upgrade before 1.4.0 retired the flat files of the release before
/// generations (an index 1.4.0 wrote can still name them): maintenance deletes
/// them after the grace period, and leaves the ledger's generations, which
/// share `submissions/` with the flat snapshot blobs.
#[test]
fn retired_flat_files_go_after_the_grace_period_and_ledger_generations_stay() {
    let directory = scratch("flat");
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    for file in ["user.db", "cache.db"] {
        fs::write(directory.join(file), b"retired").unwrap();
    }
    fs::create_dir_all(directory.join(LEGACY_BLOBS)).unwrap();
    fs::write(directory.join(LEGACY_BLOBS).join("0a1b2c"), b"old blob").unwrap();
    let ledger = store
        .paths()
        .generation(StoreKind::Submissions, "generation");
    fs::create_dir_all(&ledger).unwrap();
    fs::write(ledger.join("submissions.db"), b"in use").unwrap();
    let now = crate::clock::now();
    store
        .root()
        .index()
        .execute(
            "INSERT INTO retired_store (kind, generation, retired_at) VALUES (?1, 'flat', ?2)",
            params![LEGACY_KIND, now.timestamp()],
        )
        .unwrap();
    assert_eq!(store.root().collect_retired(now).unwrap(), 0);
    assert!(directory.join("user.db").is_file(), "kept during the grace period");
    assert_eq!(
        store
            .root()
            .collect_retired(now + RETIRED_GRACE + chrono::Duration::days(1))
            .unwrap(),
        1
    );
    assert!(!directory.join("user.db").exists());
    assert!(!directory.join("cache.db").exists());
    assert!(!directory.join(LEGACY_BLOBS).join("0a1b2c").exists());
    assert!(
        ledger.join("submissions.db").is_file(),
        "a ledger generation is never legacy"
    );
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}

/// School tables of another format are not read: they are emptied in one
/// transaction (with no freshness, which would call the lost rows fresh), every
/// user row stays, and the file stays where it is. Another user format, or a
/// file that is not a database, is refused with the file as it was: nothing in
/// it may be fetched again.
#[test]
fn school_tables_of_another_format_are_emptied_and_the_rest_refused() {
    let directory = scratch("school-format");
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    store.put_user_record("personal_events", event()).unwrap();
    let path = store.paths().store_db();
    drop(store);
    let set_version = |user: i64, school: i64| {
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch(&format!(
                "INSERT OR REPLACE INTO lms_courses(id,value,hash,updated_at,updated_by,rev)
                   VALUES('lms-course:42','{{}}','hash','2026-10-01T00:00:00Z','school',1);
                 INSERT OR REPLACE INTO dataset_state(dataset,key) VALUES('kyomu.page','grades');
                 PRAGMA user_version={};",
                migrations::store_version(user, school)
            ))
            .unwrap();
    };
    let (user, school) = (
        migrations::USER_SCHEMA_VERSION,
        migrations::SCHOOL_SCHEMA_VERSION,
    );

    set_version(user, school + 1);
    let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(changed);
    assert_eq!(store.paths().store_db(), path);
    assert!(store.table_rows("lms_courses").unwrap().is_empty());
    assert!(store.freshness(None).unwrap().is_empty());
    assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
    drop(store);
    let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(!changed, "the format is current now");
    drop(store);

    set_version(user + 1, school);
    assert!(matches!(
        DataStore::initialize_dir(directory.clone()),
        Err(DataError::Open(StoreOpenError::UnsupportedFormat {
            store: StoreKind::Store,
            ..
        }))
    ));
    assert_eq!(
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("SELECT count(*) FROM lms_courses", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1,
        "a refused store is left as it was"
    );

    fs::write(&path, b"not a database").unwrap();
    assert!(DataStore::initialize_dir(directory.clone()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not a database");
    fs::remove_dir_all(directory).unwrap();
}
