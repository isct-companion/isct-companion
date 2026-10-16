use super::*;

#[test]
fn creates_schema_six() {
    let mut connection = Connection::open_in_memory().unwrap();
    assert!(migrate(&mut connection).unwrap());
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 6);
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
fn rebuilds_a_mismatched_schema() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE obsolete(id TEXT); PRAGMA user_version=2;")
        .unwrap();
    assert!(migrate(&mut connection).unwrap());
    let obsolete: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'obsolete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(obsolete, 0);
}
