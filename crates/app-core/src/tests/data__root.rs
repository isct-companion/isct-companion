use super::*;
use crate::data::DataStore;
use serde_json::json;
use std::fs;

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("isct-root-{name}-{}", uuid::Uuid::new_v4()));
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

/// The release before generations kept user.db, cache.db and sync.db flat
/// in the folder. Its user data is copied into the first user generation,
/// its caches only when both are of the current schema, its reminder log
/// moves into `local/`, and the flat files are retired, not deleted: they
/// stay as recovery material until maintenance collects them after the
/// grace period.
#[test]
fn a_flat_folder_moves_into_generations_and_keeps_user_data() {
    let source = scratch("flat-source");
    let (store, _) = DataStore::initialize_dir(source.clone()).unwrap();
    store.put_user_record("personal_events", event()).unwrap();
    store
        .test_connection()
        .execute_batch(
            "INSERT INTO lms_courses(id,value,hash,updated_at,updated_by,rev)
               VALUES('lms-course:42','{}','hash','2026-10-01T00:00:00Z','school',1);",
        )
        .unwrap();
    let paths = store.paths().clone();
    drop(store);
    for current_caches in [true, false] {
        let flat = scratch("flat");
        fs::copy(paths.user_db(), flat.join(migrations::USER_FILE)).unwrap();
        fs::copy(paths.cache_db(), flat.join(migrations::CACHE_FILE)).unwrap();
        fs::copy(paths.sync_db(), flat.join(migrations::SYNC_FILE)).unwrap();
        if !current_caches {
            rusqlite::Connection::open(flat.join(migrations::SYNC_FILE))
                .unwrap()
                .execute_batch("PRAGMA user_version=1;")
                .unwrap();
        }
        fs::write(flat.join(LEGACY_REMINDER_LOG), b"{\"a\":1}").unwrap();
        // The flat layout's snapshot blobs, beside which the ledger's
        // generations now live.
        fs::create_dir_all(flat.join(LEGACY_BLOBS)).unwrap();
        fs::write(flat.join(LEGACY_BLOBS).join("0a1b2c"), b"old blob").unwrap();
        let ledger_generation = flat.join(LEGACY_BLOBS).join("generation");
        fs::create_dir_all(&ledger_generation).unwrap();
        fs::write(ledger_generation.join("submissions.db"), b"in use").unwrap();
        let (store, created) = DataStore::initialize_dir(flat.clone()).unwrap();
        assert!(created);
        assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
        assert_eq!(
            store.table_rows("lms_courses").unwrap().len(),
            usize::from(current_caches),
            "caches are kept only when both are current"
        );
        assert_ne!(store.paths().user_db(), flat.join(migrations::USER_FILE));
        assert!(flat.join(migrations::USER_FILE).is_file(), "retired, not deleted");
        assert_eq!(
            fs::read(store.paths().local(crate::schedule::REMINDER_LOG)).unwrap(),
            b"{\"a\":1}"
        );
        let now = chrono::Utc::now();
        assert_eq!(store.root().collect_retired(now).unwrap(), 0);
        assert!(flat.join(migrations::USER_FILE).is_file());
        assert_eq!(
            store
                .root()
                .collect_retired(now + RETIRED_GRACE + chrono::Duration::days(1))
                .unwrap(),
            1
        );
        assert!(!flat.join(migrations::USER_FILE).exists());
        assert!(!flat.join(migrations::CACHE_FILE).exists());
        assert!(!flat.join(LEGACY_BLOBS).join("0a1b2c").exists());
        assert!(
            ledger_generation.join("submissions.db").is_file(),
            "a ledger generation is never legacy"
        );
        assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
        drop(store);
        fs::remove_dir_all(flat).unwrap();
    }
    fs::remove_dir_all(source).unwrap();
}

/// A user.db of a schema this version does not read (none, an older one,
/// a newer one) is refused with its data intact, and the index names no
/// generation, so the next start refuses again rather than opening an
/// empty store.
#[test]
fn an_unreadable_user_db_is_refused_and_left_intact() {
    for version in [0, 1, 2, 4, 99] {
        let directory = scratch("user-refused");
        let user = directory.join(migrations::USER_FILE);
        rusqlite::Connection::open(&user)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES('my data');
                 PRAGMA user_version={version};"
            ))
            .unwrap();
        for _ in 0..2 {
            assert!(matches!(
                DataStore::initialize_dir(directory.clone()),
                Err(DataError::Open(StoreOpenError::UnsupportedFormat {
                    store: StoreKind::User,
                    ..
                }))
            ));
        }
        let connection = rusqlite::Connection::open(&user).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT value FROM retained", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "my data"
        );
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        drop(connection);
        fs::remove_dir_all(directory).unwrap();
    }
    // A folder of the single-file era: its data is in school.db, which
    // this version does not read; it is refused, not opened empty.
    let directory = scratch("school-db");
    rusqlite::Connection::open(directory.join(LEGACY_SCHOOL_DB))
        .unwrap()
        .execute_batch("PRAGMA user_version=8; CREATE TABLE plan_items(id TEXT);")
        .unwrap();
    assert!(matches!(
        DataStore::initialize_dir(directory.clone()),
        Err(DataError::Open(StoreOpenError::UnsupportedFormat {
            store: StoreKind::User,
            found: 8,
            ..
        }))
    ));
    assert!(!directory.join(migrations::USER_FILE).exists());
    fs::remove_dir_all(directory).unwrap();
}

/// A school group of another schema, or one that is not a database at all,
/// is not read: a new, empty generation replaces it in one index
/// transaction (with no freshness, which would call the lost rows fresh),
/// user.db keeps every row, and the old generation is retired for
/// maintenance, never deleted while in use.
#[test]
fn a_school_group_of_another_format_is_replaced_by_a_new_generation() {
    let directory = scratch("school-rebuilt");
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    store.put_user_record("personal_events", event()).unwrap();
    drop(store);
    let damage: [&dyn Fn(&StorePaths); 2] = [
        &|paths: &StorePaths| {
            let connection = rusqlite::Connection::open(paths.cache_db()).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO lms_courses(id,value,hash,updated_at,updated_by,rev)
                       VALUES('lms-course:42','{}','hash','2026-10-01T00:00:00Z','school',1);
                     PRAGMA user_version=99;",
                )
                .unwrap();
        },
        &|paths: &StorePaths| fs::write(paths.cache_db(), b"not a database").unwrap(),
    ];
    for damage in damage {
        let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
        let old = store.paths().clone();
        drop(store);
        damage(&old);
        let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
        assert!(changed);
        assert_ne!(store.paths().cache_db(), old.cache_db());
        assert!(old.cache_db().is_file(), "the old generation stays until collected");
        assert!(store.table_rows("lms_courses").unwrap().is_empty());
        assert!(store.freshness(None).unwrap().is_empty());
        assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
        store
            .root()
            .collect_retired(chrono::Utc::now() + RETIRED_GRACE + chrono::Duration::days(1))
            .unwrap();
        assert!(!old.cache_db().exists());
        assert!(store.paths().cache_db().is_file());
        drop(store);
    }
    fs::remove_dir_all(directory).unwrap();
}
