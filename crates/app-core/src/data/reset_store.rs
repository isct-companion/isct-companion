//! The database half of a reset (`docs/data-ownership.md`).
//! School caches are the cache and sync tables, whole, plus the settings rows
//! below; the rest of the user tables belongs to the person.
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

/// Bytes the used pages of the store hold, free pages excluded: what emptying
/// it gives back, as a hint for the confirmation panel, never a decision.
fn used_bytes(connection: &Connection) -> Result<u64, DataError> {
    let pragma = |name: &str| {
        connection
            .query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0))
            .map(|value| value.max(0) as u64)
            .map_err(DataError::from)
    };
    Ok(pragma("page_count")?.saturating_sub(pragma("freelist_count")?) * pragma("page_size")?)
}

/// Rows of the cache and sync tables, which a reset empties together.
fn cache_rows(connection: &Connection) -> Result<u64, DataError> {
    let school: Vec<&str> = migrations::CACHE_TABLES
        .iter()
        .copied()
        .filter(|table| !migrations::DERIVED_TABLES.contains(table))
        .collect();
    Ok(rows_in(connection, &school)? + rows_in(connection, migrations::SYNC_TABLES)?)
}

/// Bytes the cache and sync tables hold, their indexes and search index
/// included: every table but the user tables.
fn cache_bytes(connection: &Connection) -> Result<u64, DataError> {
    let user = migrations::USER_TABLES
        .iter()
        .map(|table| format!("'{table}'"))
        .collect::<Vec<_>>()
        .join(",");
    connection
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(stat.pgsize), 0) FROM dbstat AS stat
                 JOIN sqlite_master AS object ON object.name = stat.name
                 WHERE object.tbl_name NOT IN ({user})"
            ),
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|bytes| bytes.max(0) as u64)
        .map_err(DataError::from)
}

fn estimate(connection: &Connection, everything: bool) -> Result<Cleared, DataError> {
    let mut cleared = Cleared {
        rows: cache_rows(connection)?,
        bytes: cache_bytes(connection)?,
    };
    if everything {
        cleared.rows += rows_in(connection, migrations::USER_TABLES)?;
        cleared.bytes = used_bytes(connection)?;
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
        estimate(&*self.open()?, everything)
    }

    /// Empties the cache and sync tables under the write lock: every one is
    /// dropped and created again in one transaction, then the file is
    /// vacuumed. Returns the bytes freed.
    pub(crate) fn rebuild_cache(&self) -> Result<u64, DataError> {
        let mut writer = self.connections.writer();
        let before = cache_bytes(&writer)?;
        connection::allow_drops(&mut writer)?;
        let rebuilt = writer
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(DataError::from)
            .and_then(|transaction| {
                migrations::recreate_cache(&transaction)?;
                // The person's own courses stay; they are merged again.
                super::course_merge::rebuild(&transaction)?;
                transaction.commit().map_err(DataError::from)
            });
        let guarded = connection::forbid_truncate(&mut writer);
        rebuilt?;
        guarded?;
        // Dropped tables reach no update hook.
        self.committed_tables(&migrations::all_tables());
        // Best effort: a busy reader keeps the free pages for reuse.
        if let Err(error) = writer.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);") {
            log::warn!(target: "app_core::reset", "The store compaction was skipped: {error}");
        }
        Ok(before.saturating_sub(cache_bytes(&writer)?))
    }

    /// Empties the cache and sync tables, then deletes the cache settings rows
    /// and the reference bookkeeping in the user tables. Plans, notes, marks,
    /// overrides, subscriptions, tasks and settings stay. Shells hear the change
    /// through [`DataStore::announce`] once admission reopens.
    pub(crate) fn clear_school_caches(&self) -> Result<Cleared, DataError> {
        let rows = cache_rows(&*self.open()?)?;
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

    /// Empties the cache and sync tables, then deletes every user row,
    /// settings and device identity included, and writes the defaults of a
    /// new database in the same transaction: the first-launch state. No tombstone is
    /// written, so cloud copies and other devices keep their data.
    pub(crate) fn clear_everything(&self) -> Result<Cleared, DataError> {
        let cache_rows = cache_rows(&*self.open()?)?;
        let cache_bytes = self.rebuild_cache()?;
        let user = self.write(None, |transaction| {
            let cleared = Cleared {
                rows: rows_in(transaction, migrations::USER_TABLES)?,
                bytes: used_bytes(transaction)?,
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
            .execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
        {
            log::warn!(target: "app_core::reset", "The store compaction was skipped: {error}");
        }
        Ok(Cleared {
            rows: cache_rows + user.rows,
            bytes: cache_bytes + user.bytes,
        })
    }

    /// Publishes a change committed without a source, once the caller is
    /// ready for shells to read it. A reset may have changed any table.
    pub(crate) fn announce(&self, source: WorkspaceChangeSource) {
        self.commit_seq.advance();
        self.observer
            .committed(source, None, &migrations::all_tables());
    }
}
