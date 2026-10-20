//! The database half of a reset (design §10, `docs/data-ownership.md`).
//! School caches are cache.db, whole, plus the settings rows below; the rest
//! of user.db belongs to the person.
use super::*;

/// Rows and bytes a reset removes, or would remove.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Cleared {
    pub(crate) rows: u64,
    pub(crate) bytes: u64,
}

/// Settings rows a school read writes: the 教務 academic identity and the
/// course-search filters captured per year. Every other settings row is the
/// person's or the device's.
const CACHE_SETTINGS: &str = "key='kyomuAcademicIdentity' OR key LIKE 'kyomuSearchOptions:%'";

fn rows_in(connection: &Connection, tables: &[&str]) -> Result<u64, DataError> {
    let mut rows = 0;
    for table in tables {
        rows += connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count.max(0) as u64)
            .map_err(DataError::from)?;
    }
    Ok(rows)
}

fn cache_setting_rows(connection: &Connection) -> Result<u64, DataError> {
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM settings WHERE {CACHE_SETTINGS}"),
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count.max(0) as u64)
        .map_err(DataError::from)
}

/// Bytes the used pages of one file hold (`main` or `cache`), free pages
/// excluded: what emptying it gives back, as a hint for the confirmation
/// panel, never a decision.
fn used_bytes(connection: &Connection, schema: &str) -> Result<u64, DataError> {
    let pragma = |name: &str| {
        connection
            .query_row(&format!("PRAGMA {schema}.{name}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|value| value.max(0) as u64)
            .map_err(DataError::from)
    };
    Ok(pragma("page_count")?.saturating_sub(pragma("freelist_count")?) * pragma("page_size")?)
}

fn estimate(connection: &Connection, everything: bool) -> Result<Cleared, DataError> {
    let mut cleared = Cleared {
        rows: rows_in(connection, migrations::CACHE_TABLES)?,
        bytes: used_bytes(connection, "cache")?,
    };
    if everything {
        cleared.rows += rows_in(connection, migrations::USER_TABLES)?;
        cleared.bytes += used_bytes(connection, "main")?;
    } else {
        cleared.rows += cache_setting_rows(connection)?;
    }
    Ok(cleared)
}

/// Reference refresh bookkeeping inside the person's subscription rows
/// (`fetchedAt`, `dataUrl`, …) describes cache rows that are now gone. The
/// subscription itself (kind, URL, on/off) stays.
fn forget_reference_bookkeeping(transaction: &Transaction<'_>) -> Result<(), DataError> {
    for mut row in read_rows(transaction, "subscriptions")? {
        if row
            .get("deletedAt")
            .is_some_and(|deleted| !deleted.is_null())
        {
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

    /// Empties cache.db under the write lock: every table is dropped and
    /// created again in one transaction, then the file is vacuumed. Returns
    /// the bytes freed. The file is emptied in place rather than deleted
    /// because read connections keep it open, and SQLite removes a WAL file
    /// by name when the last connection to the old file closes, which would
    /// take the new file's log with it.
    pub(crate) fn rebuild_cache(&self) -> Result<u64, DataError> {
        let mut writer = self.connections.writer();
        let before = used_bytes(&writer, "cache")?;
        let transaction =
            writer.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        migrations::recreate_cache(&transaction)?;
        transaction.commit()?;
        // Best effort: a busy reader keeps the free pages for reuse.
        if let Err(error) =
            writer.execute_batch("VACUUM cache; PRAGMA cache.wal_checkpoint(TRUNCATE);")
        {
            log::warn!(target: "isct_core::reset", "cache.db compaction skipped: {error}");
        }
        Ok(before.saturating_sub(used_bytes(&writer, "cache")?))
    }

    /// Empties cache.db, then deletes the cache settings rows and the
    /// reference bookkeeping in user.db. Plans, notes, marks, overrides,
    /// subscriptions, tasks and settings stay. Shells hear the change
    /// through [`DataStore::announce`] once admission reopens.
    pub(crate) fn clear_school_caches(&self) -> Result<Cleared, DataError> {
        let rows = rows_in(&self.open()?, migrations::CACHE_TABLES)?;
        let bytes = self.rebuild_cache()?;
        let settings = self.write(None, |transaction| {
            let settings = cache_setting_rows(transaction)?;
            transaction
                .execute(&format!("DELETE FROM settings WHERE {CACHE_SETTINGS}"), [])
                .map_err(DataError::from)?;
            forget_reference_bookkeeping(transaction)?;
            Ok::<_, DataError>(settings)
        })?;
        Ok(Cleared {
            rows: rows + settings,
            bytes,
        })
    }

    /// Empties cache.db, then deletes every row of user.db, settings and
    /// device identity included, and writes the defaults of a new database
    /// in the same transaction: the first-launch state. No tombstone is
    /// written, so cloud copies and other devices keep their data.
    pub(crate) fn clear_everything(&self) -> Result<Cleared, DataError> {
        let cache_rows = rows_in(&self.open()?, migrations::CACHE_TABLES)?;
        let cache_bytes = self.rebuild_cache()?;
        let user = self.write(None, |transaction| {
            let cleared = Cleared {
                rows: rows_in(transaction, migrations::USER_TABLES)?,
                bytes: used_bytes(transaction, "main")?,
            };
            for table in migrations::USER_TABLES {
                transaction
                    .execute(&format!("DELETE FROM {table}"), [])
                    .map_err(DataError::from)?;
            }
            ensure_defaults_in(transaction)?;
            Ok::<_, DataError>(cleared)
        })?;
        // Best effort: a busy reader keeps the free pages for reuse.
        if let Err(error) = self
            .connections
            .writer()
            .execute_batch("VACUUM main; PRAGMA main.wal_checkpoint(TRUNCATE);")
        {
            log::warn!(target: "isct_core::reset", "user.db compaction skipped: {error}");
        }
        Ok(Cleared {
            rows: cache_rows + user.rows,
            bytes: cache_bytes + user.bytes,
        })
    }

    /// Publishes a change committed without a source, once the caller is
    /// ready for shells to read it. A reset may have changed any table.
    pub(crate) fn announce(&self, source: WorkspaceChangeSource) {
        self.observer
            .committed(source, None, &migrations::all_tables());
    }
}
