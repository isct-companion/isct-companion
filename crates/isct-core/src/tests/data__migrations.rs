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
        .pragma_update(None, "user_version", version)
        .unwrap();
    for table in USER_TABLES
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
               INSERT INTO lms_courses(id,lms_id,value,hash,updated_at,updated_by,rev)
                 VALUES('lms-course:42',42,'{"id":"lms-course:42"}','hash','2026-10-01T00:00:00Z','school',1);
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
        for table in USER_TABLES {
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

    for version in [0, 2, 99] {
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
/// all, is created again, empty, and user.db keeps every row.
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
                    "INSERT INTO lms_courses(id,lms_id,value,hash,updated_at,updated_by,rev)
                       VALUES('lms-course:42',42,'{}','hash','2026-10-01T00:00:00Z','school',1);
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
        assert_eq!(store.user_rows("plans").unwrap().len(), 1);
    }
    fs::remove_dir_all(directory).unwrap();
}
