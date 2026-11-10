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

/// A user.db as the builds before one store wrote it: the user tables of
/// `store`, at the user format.
fn old_user_db(path: &Path, store: &Path) {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection.execute_batch(migrations::USER_SCHEMA).unwrap();
    connection
        .execute("ATTACH DATABASE ?1 AS seeded", [store.to_str().unwrap()])
        .unwrap();
    for table in migrations::USER_TABLES {
        connection
            .execute_batch(&format!(
                "INSERT INTO {table} SELECT * FROM seeded.{table};"
            ))
            .unwrap();
    }
    connection
        .execute_batch(&format!(
            "DETACH DATABASE seeded; PRAGMA user_version={};",
            migrations::USER_SCHEMA_VERSION
        ))
        .unwrap();
}

/// A store holding one personal event, closed: what an upgrade carries over.
fn seeded() -> (PathBuf, PathBuf) {
    let directory = scratch("seed");
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    store.put_user_record("personal_events", event()).unwrap();
    let path = store.paths().store_db();
    drop(store);
    (directory, path)
}

/// The release before generations kept user.db, cache.db and sync.db flat in
/// the folder. Its user data is copied into the first store generation, its
/// school data is not (it is fetched again), its reminder log moves into
/// `local/`, and the flat files are retired, not deleted: they stay as recovery
/// material until maintenance collects them after the grace period.
#[test]
fn a_flat_folder_moves_into_a_store_and_keeps_user_data() {
    let (source, seed) = seeded();
    let flat = scratch("flat");
    old_user_db(&flat.join(LEGACY_USER_FILE), &seed);
    fs::write(flat.join("cache.db"), b"old school data").unwrap();
    fs::write(flat.join("sync.db"), b"old school data").unwrap();
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
    assert!(store.table_rows("lms_courses").unwrap().is_empty());
    assert!(
        flat.join(LEGACY_USER_FILE).is_file(),
        "retired, not deleted"
    );
    assert_eq!(
        fs::read(store.paths().local(crate::schedule::REMINDER_LOG)).unwrap(),
        b"{\"a\":1}"
    );
    let now = chrono::Utc::now();
    assert_eq!(store.root().collect_retired(now).unwrap(), 0);
    assert!(flat.join(LEGACY_USER_FILE).is_file());
    assert_eq!(
        store
            .root()
            .collect_retired(now + RETIRED_GRACE + chrono::Duration::days(1))
            .unwrap(),
        1
    );
    assert!(!flat.join(LEGACY_USER_FILE).exists());
    assert!(!flat.join("cache.db").exists());
    assert!(!flat.join(LEGACY_BLOBS).join("0a1b2c").exists());
    assert!(
        ledger_generation.join("submissions.db").is_file(),
        "a ledger generation is never legacy"
    );
    assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
    drop(store);
    // Opening it again finds the store as it is, and carries nothing over twice.
    let (store, created) = DataStore::initialize_dir(flat.clone()).unwrap();
    assert!(!created);
    assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
    drop(store);
    fs::remove_dir_all(flat).unwrap();
    fs::remove_dir_all(source).unwrap();
}

/// The build before one store kept user.db and the school group in generations
/// of their own, named by the index. The user tables come over and both old
/// generations are retired; flat files of an earlier upgrade are not read.
#[test]
fn generations_of_the_build_before_become_one_store() {
    let (source, seed) = seeded();
    let directory = scratch("earlier");
    let user = directory.join(EARLIER_USER).join("old-user");
    let school = directory.join(EARLIER_SCHOOL).join("old-school");
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&school).unwrap();
    old_user_db(&user.join(LEGACY_USER_FILE), &seed);
    fs::write(school.join("cache.db"), b"old school data").unwrap();
    fs::write(
        directory.join(LEGACY_USER_FILE),
        b"retired by an earlier upgrade",
    )
    .unwrap();
    {
        let index = open_index(&directory.join(INDEX_FILE)).unwrap();
        index
            .execute_batch(
                "INSERT INTO active_store VALUES ('user', 'old-user', 3), ('school', 'old-school', 502);",
            )
            .unwrap();
    }
    let (store, created) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(created);
    assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
    assert_eq!(
        store
            .root()
            .collect_retired(chrono::Utc::now() + RETIRED_GRACE + chrono::Duration::days(1))
            .unwrap(),
        2
    );
    assert!(!user.exists());
    assert!(!school.exists());
    assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
    fs::remove_dir_all(source).unwrap();
}

/// A flat user.db as 1.2.2 (format 1: the identity map, no course keys, the
/// `plans` table) or 1.2.3 (format 2: course keys, still `plans`) wrote it.
/// The new store gets every user row, course keys that give each course the
/// id the map gave it, and no `plans`; the old file is only read.
#[test]
fn a_user_db_of_an_older_user_format_is_upgraded_in_the_new_store() {
    let plan = json!({"id":"plan","kind":"plan-item","courseId":"2026-202602440",
        "termId":"2026-3Q","state":"planned","rev":3,
        "updatedAt":"2026-10-01T00:00:00Z","updatedBy":"device-a"});
    let manual = json!({"id":"manual-1","title":"Reading group","termId":"2026-3Q",
        "offeringId":"202602431","rev":1,"updatedAt":"2026-10-01T00:00:00Z","updatedBy":"device-a"});
    for version in [1, 2] {
        let directory = scratch("user-upgrade");
        let user = directory.join(LEGACY_USER_FILE);
        let identity = if version == 1 {
            "DROP TABLE course_keys;
             CREATE TABLE course_identity_map (
               provider TEXT NOT NULL, external_id TEXT NOT NULL, academic_year INTEGER NOT NULL,
               course_id TEXT NOT NULL, offering_id TEXT, updated_at TEXT NOT NULL,
               PRIMARY KEY (provider, external_id, academic_year));
             INSERT INTO course_identity_map
               VALUES('lms','42',2026,'2026-202602440','202602440','2026-10-01T00:00:00Z');"
        } else {
            "INSERT INTO course_keys VALUES
               ('lms:42','2026-202602440','2026-10-01T00:00:00Z'),
               ('offering:2026:202602440','2026-202602440','2026-10-01T00:00:00Z'),
               ('manual:manual-1','manual-1','2026-10-01T00:00:00Z'),
               ('offering:2026:202602431','manual-1','2026-10-01T00:00:00Z');"
        };
        let connection = rusqlite::Connection::open(&user).unwrap();
        connection.execute_batch(migrations::USER_SCHEMA).unwrap();
        connection
            .execute_batch(&format!(
                "{identity}
                 CREATE TABLE plans (
                   id TEXT PRIMARY KEY, title TEXT, course_id TEXT, room TEXT, start TEXT,
                   duration INTEGER, rrule TEXT, exdates TEXT, reminder TEXT,
                   value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
                   updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT);
                 INSERT INTO plans(id,value,hash,updated_at,updated_by,rev)
                   VALUES('old-plan','{{}}','hash','2026-10-01T00:00:00Z','device-a',1);
                 INSERT INTO plan_items(id,course_id,value,hash,updated_at,updated_by,rev,source)
                   VALUES('plan','2026-202602440','{plan}','hash','2026-10-01T00:00:00Z','device-a',3,'manual');
                 INSERT INTO manual_courses(id,value,hash,updated_at,updated_by,rev,source)
                   VALUES('manual-1','{manual}','hash','2026-10-01T00:00:00Z','device-a',1,'manual');
                 INSERT INTO personal_events(id,value,hash,updated_at,updated_by,rev)
                   VALUES('event','{event}','hash','2026-10-01T00:00:00Z','device-a',1);
                 PRAGMA user_version={version};",
                event = event(),
            ))
            .unwrap();
        drop(connection);

        let (store, created) = DataStore::initialize_dir(directory.clone()).unwrap();
        assert!(created, "format {version}");
        assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
        assert_eq!(store.user_rows("manual_courses").unwrap().len(), 1);
        let connection = store.test_connection();
        let plan_course: String = connection
            .query_row(
                "SELECT course_id FROM plan_items WHERE id='plan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(plan_course, "2026-202602440");
        let keys: Vec<(String, String)> = connection
            .prepare("SELECT key, course_id FROM course_keys ORDER BY key")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected = [
            ("lms:42", "2026-202602440"),
            ("manual:manual-1", "manual-1"),
            ("offering:2026:202602431", "manual-1"),
            ("offering:2026:202602440", "2026-202602440"),
        ];
        assert_eq!(
            keys,
            expected.map(|(key, id)| (key.to_owned(), id.to_owned())),
            "format {version}"
        );
        let leftovers: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE name IN ('plans','course_identity_map')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leftovers, 0);
        drop(connection);
        drop(store);
        let source_version: i64 = rusqlite::Connection::open(&user)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(source_version, version, "the old file is only read");
        let (store, created) = DataStore::initialize_dir(directory.clone()).unwrap();
        assert!(!created);
        assert_eq!(store.user_rows("personal_events").unwrap().len(), 1);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }
}

/// A user.db of a schema this version does not read (none, a newer one) is
/// refused with its data intact, and the index names no generation, so the
/// next start refuses again rather than opening an empty store.
#[test]
fn an_unreadable_user_db_is_refused_and_left_intact() {
    for version in [0, 4, 99] {
        let directory = scratch("user-refused");
        let user = directory.join(LEGACY_USER_FILE);
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
                    store: StoreKind::Store,
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
            store: StoreKind::Store,
            found: 8,
            ..
        }))
    ));
    assert!(!directory.join(LEGACY_USER_FILE).exists());
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
