use super::*;
use crate::data::DataStore;
use rusqlite::params;
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf};

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("isct-migrations-{name}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn tables_of(connection: &Connection, schema: &str) -> BTreeSet<String> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT name FROM {schema}.sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
        ))
        .unwrap();
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .filter(|name| {
            !FTS_TABLES
                .iter()
                .any(|fts| name.starts_with(&format!("{fts}_")))
        })
        .collect();
    names
}

fn listed(tables: &[&str]) -> BTreeSet<String> {
    tables.iter().map(|table| (*table).to_string()).collect()
}

/// Every table is created in the file that owns it, and the two lists match
/// the schemas: a table missing from them would escape a cache reset or be
/// counted wrong, and a backup table outside user.db would not be backed up.
#[test]
fn each_table_is_created_in_the_file_that_owns_it() {
    let connection = open_in_memory().unwrap();
    assert_eq!(tables_of(&connection, "main"), listed(USER_TABLES));
    assert_eq!(tables_of(&connection, "cache"), listed(CACHE_TABLES));
    assert_eq!(tables_of(&connection, "sync"), listed(SYNC_TABLES));
    for table in crate::data::USER_BACKUP_TABLES {
        assert!(USER_TABLES.contains(table), "{table}");
    }
}

/// A single-file database as schemas 6–8 left it: both schemas in one file.
fn legacy_school_db(directory: &Path, version: i64) {
    let connection = Connection::open(directory.join(LEGACY_FILE)).unwrap();
    connection
        .execute_batch("PRAGMA journal_mode=WAL;")
        .unwrap();
    connection.execute_batch(USER_SCHEMA).unwrap();
    connection
        .execute_batch(&CACHE_SCHEMA.replace("cache.", ""))
        .unwrap();
    connection
        .execute_batch(&SYNC_SCHEMA.replace("sync.", ""))
        .unwrap();
    connection
        .pragma_update(None, "user_version", version)
        .unwrap();
    for table in LEGACY_USER_TABLES
        .iter()
        .filter(|table| !matches!(**table, "settings" | "course_identity_map"))
    {
        let id = format!("{table}:1");
        connection
            .execute(
                &format!(
                    "INSERT INTO {table}(id,value,hash,updated_at,updated_by,rev,source)
                     VALUES(?1,?2,'hash','2026-10-01T00:00:00Z','device-legacy',3,'manual')"
                ),
                params![id, json!({ "id": id }).to_string()],
            )
            .unwrap();
    }
    connection
        .execute_batch(
            r#"INSERT INTO settings(key,value,hash,updated_at,updated_by,rev)
                 VALUES('deviceId','"device-legacy"','hash','2026-10-01T00:00:00Z','device-legacy',1);
               INSERT INTO settings(key,value,hash,updated_at,updated_by,rev)
                 VALUES('userPreferences','{"departmentId":"cs"}','hash','2026-10-01T00:00:00Z','device-legacy',2);
               INSERT INTO course_identity_map
                 VALUES('lms','42',2026,'2026-202602440','202602440','2026-10-01T00:00:00Z');
               INSERT INTO lms_courses(id,value,hash,updated_at,updated_by,rev)
                 VALUES('lms-course:42','{"id":"lms-course:42","lmsId":42}','hash','2026-10-01T00:00:00Z','school',1);
               INSERT INTO dataset_state(dataset,key,fetched_at)
                 VALUES('lms.course','42','2026-10-01T00:00:00Z');"#,
        )
        .unwrap();
}

/// The split keeps every row of every user table as it was (device id,
/// settings, identity map and records alike), leaves the school caches
/// behind so every key reads as missing, keeps the legacy file as a backup,
/// and happens once: the next start changes nothing.
#[test]
fn a_legacy_school_db_moves_every_user_row_into_user_db_once() {
    for version in [6, 8] {
        let directory = scratch("legacy");
        legacy_school_db(&directory, version);

        let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();

        assert!(changed);
        let backup = directory.join(LEGACY_BACKUP);
        assert!(backup.is_file());
        assert!(!directory.join(LEGACY_FILE).exists());
        assert!(!directory.join(STAGING_FILE).exists());
        let connection = store.test_connection();
        attach_as(&connection, &backup, "legacy").unwrap();
        let count =
            |sql: String| -> i64 { connection.query_row(&sql, [], |row| row.get(0)).unwrap() };
        // The identity map becomes course keys; every other table is copied.
        for table in LEGACY_USER_TABLES
            .iter()
            .filter(|table| **table != "course_identity_map")
        {
            assert!(
                count(format!("SELECT COUNT(*) FROM legacy.{table}")) > 0,
                "{table}"
            );
            assert_eq!(
                count(format!(
                    "SELECT COUNT(*) FROM (SELECT * FROM legacy.{table} EXCEPT SELECT * FROM main.{table})"
                )),
                0,
                "{table} lost or changed rows"
            );
        }
        // The course keeps the id the map gave it, under both its keys.
        assert_eq!(
            count(
                "SELECT COUNT(*) FROM course_keys WHERE course_id='2026-202602440'
                 AND key IN ('lms:42','offering:2026:202602440')"
                    .into()
            ),
            2
        );
        drop(connection);
        assert!(store.table_rows("lms_courses").unwrap().is_empty());
        assert!(store.freshness(None).unwrap().is_empty());
        drop(store);

        let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
        assert!(!changed);
        assert!(backup.is_file());
        assert_eq!(store.user_rows("plans").unwrap().len(), 1);
        drop(store);
        fs::remove_dir_all(directory).unwrap();
    }
}

/// A legacy file of a schema this build cannot read, and a user.db of
/// another version, are refused with their data intact; nothing is renamed
/// and no half-made user.db is left behind.
#[test]
fn unreadable_databases_are_refused_and_left_intact() {
    let directory = scratch("legacy-refused");
    legacy_school_db(&directory, 2);
    assert!(matches!(
        DataStore::initialize_dir(directory.clone()),
        Err(DataError::Compatibility(_))
    ));
    assert!(directory.join(LEGACY_FILE).is_file());
    assert!(!directory.join(LEGACY_BACKUP).exists());
    assert!(!directory.join(USER_FILE).exists());
    assert!(!directory.join(STAGING_FILE).exists());
    fs::remove_dir_all(directory).unwrap();

    for version in [0, 3, 99] {
        let directory = scratch("user-refused");
        let user = directory.join(USER_FILE);
        Connection::open(&user)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES('my data');
                 PRAGMA user_version={version};"
            ))
            .unwrap();
        assert!(matches!(
            DataStore::initialize_dir(directory.clone()),
            Err(DataError::Compatibility(_))
        ));
        let connection = Connection::open(&user).unwrap();
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
}

/// A cache.db of another schema version, or one that is not a database at
/// all, is created again, empty, with its freshness (which would call the
/// lost rows fresh), and user.db keeps every row.
#[test]
fn a_cache_db_of_another_version_or_unreadable_is_rebuilt_and_user_data_stays() {
    let directory = scratch("cache-rebuilt");
    let (store, _) = DataStore::initialize_dir(directory.clone()).unwrap();
    store
        .put_user_record(
            "plans",
            json!({"title":"Study group","start":"2026-10-13T10:00:00+09:00","duration":60,"exdates":[]}),
        )
        .unwrap();
    drop(store);
    let damage: [&dyn Fn(&Path); 2] = [
        &|directory: &Path| {
            let (store, _) = DataStore::initialize_dir(directory.to_path_buf()).unwrap();
            store
                .test_connection()
                .execute_batch(
                    "INSERT INTO lms_courses(id,value,hash,updated_at,updated_by,rev)
                       VALUES('lms-course:42','{}','hash','2026-10-01T00:00:00Z','school',1);
                     INSERT INTO dataset_state(dataset,key,fetched_at)
                       VALUES('lms.course','42','2026-10-01T00:00:00Z');
                     PRAGMA cache.user_version=99;",
                )
                .unwrap();
        },
        &|directory: &Path| fs::write(directory.join(CACHE_FILE), b"not a database").unwrap(),
    ];
    for damage in damage {
        damage(&directory);
        let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
        assert!(changed);
        assert!(store.table_rows("lms_courses").unwrap().is_empty());
        assert!(store.freshness(None).unwrap().is_empty());
        assert_eq!(store.user_rows("plans").unwrap().len(), 1);
    }
    fs::remove_dir_all(directory).unwrap();
}

/// A cache.db of schema 3 held the sync bookkeeping itself. Opening it moves
/// freshness and run records into sync.db once and keeps every school row,
/// so an existing data folder is not fetched again; a lost sync.db alone
/// only makes every key read as missing.
/// A folder from before course keys and merged courses (user.db 1 with its
/// identity map, cache.db 4) upgrades in place: the course keeps the id the
/// map gave it, records that name it still do, the school rows stay, and its
/// merged row is there on the first read.
#[test]
fn a_folder_from_before_course_keys_upgrades_in_place() {
    let directory = scratch("course-keys-upgrade");
    drop(DataStore::initialize_dir(directory.clone()).unwrap());
    let user = Connection::open(directory.join(USER_FILE)).unwrap();
    attach(
        &user,
        &directory.join(CACHE_FILE),
        &directory.join(SYNC_FILE),
    )
    .unwrap();
    let lms = json!({"id":"lms-course:42","lmsId":42,"offeringId":"202602440",
        "shortname":"CSC.T273[2026]","title":{"en":"Compilers"},"year":2026,"quarters":["3Q"],
        "timetables":{},"sections":[],"hidden":false,"sourceKey":"lms:default:course:42",
        "rev":1,"updatedAt":"2026-10-01T00:00:00Z","updatedBy":"lms"});
    user.execute_batch(&format!(
        "DROP TABLE cache.merged_rows; DROP TABLE cache.merge_state; PRAGMA cache.user_version=4;
         DROP TABLE course_keys;
         CREATE TABLE course_identity_map (
           provider TEXT NOT NULL, external_id TEXT NOT NULL, academic_year INTEGER NOT NULL,
           course_id TEXT NOT NULL, offering_id TEXT, updated_at TEXT NOT NULL,
           PRIMARY KEY (provider, external_id, academic_year));
         INSERT INTO course_identity_map
           VALUES('lms','42',2026,'2026-202602440','202602440','2026-10-01T00:00:00Z');
         INSERT INTO cache.lms_courses(id,value,hash,updated_at,updated_by,rev)
           VALUES('lms-course:42','{lms}','hash','2026-10-01T00:00:00Z','lms',1);
         INSERT INTO plan_items(id,value,hash,updated_at,updated_by,rev,source)
           VALUES('plan','{plan}','hash','2026-10-01T00:00:00Z','device-a',3,'manual');
         PRAGMA main.user_version=1;",
        plan = json!({"id":"plan","kind":"plan-item","courseId":"2026-202602440",
            "termId":"2026-3Q","state":"planned","rev":3,
            "updatedAt":"2026-10-01T00:00:00Z","updatedBy":"device-a"}),
    ))
    .unwrap();
    drop(user);

    let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(changed);
    let workspace = store.load_workspace().unwrap();
    let course = workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["lmsCourseId"] == 42)
        .unwrap();
    assert_eq!(course["id"], "2026-202602440");
    assert!(workspace["planItems"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "plan" && row["courseId"] == "2026-202602440"));
    let connection = store.test_connection();
    assert_eq!(tables_of(&connection, "main"), listed(USER_TABLES));
    assert_eq!(tables_of(&connection, "cache"), listed(CACHE_TABLES));
    let keys: Vec<String> = connection
        .prepare("SELECT key FROM course_keys WHERE course_id='2026-202602440' ORDER BY key")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(keys, ["lms:42", "offering:2026:202602440"]);
    drop(connection);
    drop(store);
    let (_, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(!changed);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_cache_db_with_its_bookkeeping_moves_it_into_sync_db_once() {
    let directory = scratch("cache-v3");
    drop(DataStore::initialize_dir(directory.clone()).unwrap());
    fs::remove_file(directory.join(SYNC_FILE)).unwrap();
    let cache = Connection::open(directory.join(CACHE_FILE)).unwrap();
    cache
        .execute_batch(&format!(
            "{}
             INSERT INTO lms_courses(id,value,hash,updated_at,updated_by,rev)
               VALUES('lms-course:42','{{\"id\":\"lms-course:42\",\"lmsId\":42}}','hash','2026-10-01T00:00:00Z','school',1);
             INSERT INTO dataset_state(dataset,key,fetched_at)
               VALUES('lms.course','42','2026-10-01T00:00:00Z');
             INSERT INTO sync_runs(run,dataset,key,started_at,finished_at,outcome)
               VALUES('run-1','lms.course','42','2026-10-01T00:00:00Z','2026-10-01T00:00:01Z','complete');
             PRAGMA user_version=3;",
            SYNC_SCHEMA.replace("sync.", "")
        ))
        .unwrap();
    drop(cache);

    let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(changed);
    assert_eq!(store.table_rows("lms_courses").unwrap().len(), 1);
    assert_eq!(store.freshness(None).unwrap().len(), 1);
    let connection = store.test_connection();
    assert_eq!(tables_of(&connection, "cache"), listed(CACHE_TABLES));
    let runs: i64 = connection
        .query_row("SELECT COUNT(*) FROM sync.sync_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(runs, 1);
    drop(connection);
    drop(store);

    let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(!changed);
    assert_eq!(store.freshness(None).unwrap().len(), 1);
    drop(store);
    fs::remove_file(directory.join(SYNC_FILE)).unwrap();
    let (store, changed) = DataStore::initialize_dir(directory.clone()).unwrap();
    assert!(changed);
    assert!(store.freshness(None).unwrap().is_empty());
    assert_eq!(store.table_rows("lms_courses").unwrap().len(), 1);
    drop(store);
    fs::remove_dir_all(directory).unwrap();
}
