use super::*;
use std::collections::BTreeSet;

fn tables_of(connection: &Connection) -> BTreeSet<String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
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

/// The three lists name every table of the schema, once each: a table missing
/// from them would escape a cache reset or be counted wrong, and a backup table
/// outside the user tables would not be backed up.
#[test]
fn each_table_belongs_to_one_group() {
    let connection = open_in_memory().unwrap();
    let groups = [
        listed(USER_TABLES),
        listed(CACHE_TABLES),
        listed(SYNC_TABLES),
    ];
    let every: BTreeSet<String> = groups.iter().flatten().cloned().collect();
    assert_eq!(tables_of(&connection), every);
    assert_eq!(every.len(), groups.iter().map(BTreeSet::len).sum::<usize>());
    for table in crate::data::USER_BACKUP_TABLES {
        assert!(USER_TABLES.contains(table), "{table}");
    }
}
