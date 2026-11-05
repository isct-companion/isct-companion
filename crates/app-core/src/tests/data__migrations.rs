use super::*;
use std::collections::BTreeSet;

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
