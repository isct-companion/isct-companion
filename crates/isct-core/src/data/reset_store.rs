//! The database half of a reset (design §10, `docs/data-ownership.md`):
//! which rows are a school cache that a sync reads again, and which belong
//! to the person.
use super::*;

/// Rows and bytes a reset removes, or would remove.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Cleared {
    pub(crate) rows: u64,
    pub(crate) bytes: u64,
}

/// School-cache tables beyond the record tables in `SCHOOL_CACHE_TABLES`:
/// シラバス details, search-only catalog rows, the catalog index, freshness
/// and run records. Clearing `dataset_state` makes every key read as missing.
const SCHOOL_CACHE_EXTRA: &[&str] = &[
    "offering_content",
    "catalog_search_items",
    "offerings_fts",
    "dataset_state",
    "sync_runs",
];

/// FTS5 keeps the index of `offerings_fts` in these shadow tables.
const FTS_SHADOWS: &[&str] = &[
    "offerings_fts_data",
    "offerings_fts_idx",
    "offerings_fts_content",
    "offerings_fts_docsize",
    "offerings_fts_config",
];

/// Settings rows a school read writes: the 教務 academic identity and the
/// course-search filters captured per year. Every other settings row is the
/// person's or the device's.
const CACHE_SETTINGS: &str = "key='kyomuAcademicIdentity' OR key LIKE 'kyomuSearchOptions:%'";

/// Kept by a school-cache reset although derived from school ids:
/// `course_identity_map` pins the course id that plans, plan items and course
/// overrides refer to. Rebuilding it from a later sync that has not yet seen
/// an offering id would give the same course another id.
fn school_cache_tables() -> Vec<String> {
    SCHOOL_CACHE_TABLES
        .iter()
        .chain(SCHOOL_CACHE_EXTRA)
        .map(|table| (*table).to_string())
        .collect()
}

/// Every table of the schema. FTS shadow tables are cleared through their
/// virtual table, never directly.
fn every_table(connection: &Connection) -> Result<Vec<String>, DataError> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table'
             AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'offerings_fts_%'
             ORDER BY name",
        )
        .map_err(DataError::from)?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(DataError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DataError::from);
    names
}

fn row_count(connection: &Connection, table: &str) -> Result<u64, DataError> {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|count| count.max(0) as u64)
        .map_err(DataError::from)
}

/// Pages the tables and their indexes occupy, from SQLite's `dbstat`
/// (compiled into the bundled SQLite). Without it the estimate counts rows
/// only; the bytes are a hint for the confirmation panel, never a decision.
fn table_bytes(connection: &Connection, tables: &[String]) -> u64 {
    let names = tables.iter().map(String::as_str).chain(
        tables
            .iter()
            .any(|table| table == "offerings_fts")
            .then_some(FTS_SHADOWS)
            .into_iter()
            .flatten()
            .copied(),
    );
    let mut bytes = 0u64;
    for name in names {
        match connection.query_row(
            "SELECT COALESCE(SUM(d.pgsize),0) FROM dbstat d
             JOIN sqlite_master m ON m.name=d.name WHERE m.tbl_name=?1",
            [name],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(size) => bytes += size.max(0) as u64,
            Err(error) => {
                log::warn!(target: "isct_core::reset", "Table sizes are unavailable: {error}");
                return 0;
            }
        }
    }
    bytes
}

fn estimate(connection: &Connection, everything: bool) -> Result<Cleared, DataError> {
    let tables = if everything {
        every_table(connection)?
    } else {
        school_cache_tables()
    };
    let mut rows = 0;
    for table in &tables {
        rows += row_count(connection, table)?;
    }
    if !everything {
        rows += connection
            .query_row(
                &format!("SELECT COUNT(*) FROM settings WHERE {CACHE_SETTINGS}"),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as u64)
            .map_err(DataError::from)?;
    }
    Ok(Cleared {
        rows,
        bytes: table_bytes(connection, &tables),
    })
}

/// Rows are counted by [`estimate`] in the same transaction, not from the
/// change counter, which SQLite does not keep alike for FTS tables.
fn delete_all(transaction: &Transaction<'_>, tables: &[String]) -> Result<(), DataError> {
    for table in tables {
        transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(DataError::from)?;
    }
    Ok(())
}

/// Reference refresh bookkeeping inside the person's subscription rows
/// (`fetchedAt`, `dataUrl`, …) describes cache rows that are now gone. The
/// subscription itself (kind, URL, on/off) stays.
fn forget_reference_bookkeeping(transaction: &Transaction<'_>) -> Result<(), DataError> {
    for mut row in read_rows(transaction, "subscriptions")? {
        if row.get("deletedAt").is_some_and(|deleted| !deleted.is_null()) {
            continue;
        }
        let Some(object) = row.as_object_mut() else {
            continue;
        };
        let mut touched = false;
        for field in bookkeeping_fields("subscriptions") {
            touched |= object.remove(*field).is_some();
        }
        if touched {
            put_user_record_in_transaction(transaction, "subscriptions", row, "reset", None)?;
        }
    }
    Ok(())
}

impl DataStore {
    /// What a reset of this scope would remove from the database now.
    pub(crate) fn reset_estimate(&self, everything: bool) -> Result<Cleared, DataError> {
        estimate(&self.open()?, everything)
    }

    /// Deletes every school-cache row, the cache settings rows, every
    /// freshness and run record, in one transaction. Plans, notes, marks,
    /// overrides, subscriptions, tasks and settings stay. Shells hear the
    /// change through [`DataStore::announce`] once admission reopens.
    pub(crate) fn clear_school_caches(&self) -> Result<Cleared, DataError> {
        self.write(None, |transaction| {
            let cleared = estimate(transaction, false)?;
            delete_all(transaction, &school_cache_tables())?;
            transaction
                .execute(&format!("DELETE FROM settings WHERE {CACHE_SETTINGS}"), [])
                .map_err(DataError::from)?;
            forget_reference_bookkeeping(transaction)?;
            Ok(cleared)
        })
    }

    /// Deletes every row of every table, settings and device identity
    /// included, then writes the defaults of a new database in the same
    /// transaction: the first-launch state. No tombstone is written, so cloud
    /// copies and other devices keep their data.
    pub(crate) fn clear_everything(&self) -> Result<Cleared, DataError> {
        self.write(None, |transaction| {
            let cleared = estimate(transaction, true)?;
            delete_all(transaction, &every_table(transaction)?)?;
            ensure_defaults_in(transaction)?;
            Ok(cleared)
        })
    }

    /// Returns freed pages to the file system after a reset. Best effort: a
    /// busy database keeps its free pages for reuse.
    pub(crate) fn compact(&self) {
        let result = Connection::open(&self.path).and_then(|connection| {
            connection.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        });
        if let Err(error) = result {
            log::warn!(target: "isct_core::reset", "Database compaction skipped: {error}");
        }
    }

    /// Publishes a change committed without a source, once the caller is
    /// ready for shells to read it.
    pub(crate) fn announce(&self, source: WorkspaceChangeSource) {
        self.observer.committed(source, None);
    }
}
