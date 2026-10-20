use super::*;

#[test]
fn creates_schema_eight() {
    let mut connection = Connection::open_in_memory().unwrap();
    assert!(migrate(&mut connection).unwrap());
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 8);
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'timetable_courses'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let identities: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'course_identity_map'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(identities, 1);
}

#[test]
fn upgrades_schema_six_without_losing_stored_data() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    connection.execute_batch(r#"DROP TABLE class_schedules; DROP TABLE dataset_state; DROP TABLE sync_runs;
      CREATE TABLE import_runs(id TEXT PRIMARY KEY, provider TEXT NOT NULL, scope TEXT NOT NULL, state TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT, message TEXT);
      PRAGMA user_version=6;
      INSERT INTO settings(key,value,hash,updated_at,updated_by,rev) VALUES('userPreferences','{"departmentId":"cs"}','hash','now','device',1);
      INSERT INTO manual_courses(id,value,hash,updated_at,updated_by,rev) VALUES('course','{}','hash','now','device',1);"#).unwrap();
    assert!(migrate(&mut connection).unwrap());
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM settings WHERE key='userPreferences'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        r#"{"departmentId":"cs"}"#
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM manual_courses", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    // Import runs are gone; freshness starts empty, so every key is missing.
    let tables = |name: &str| -> i64 {
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(tables("import_runs"), 0);
    assert_eq!(tables("dataset_state"), 1);
    assert_eq!(tables("class_schedules"), 1);
    assert!(!migrate(&mut connection).unwrap());
}

#[test]
fn unsupported_databases_are_left_intact() {
    for version in [0, 2, 9, 99] {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(&format!("CREATE TABLE retained(value TEXT); INSERT INTO retained VALUES('my data'); PRAGMA user_version={version};")).unwrap();
        assert!(matches!(
            migrate(&mut connection),
            Err(DataError::Compatibility(_))
        ));
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
    }
}
