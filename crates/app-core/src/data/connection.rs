//! The connections of one store. Every commit queues on one
//! long-lived write connection; reads use a few long-lived read-only
//! connections. Each connection opens user.db as `main` and attaches
//! cache.db as `cache` and sync.db as `sync`. Table names are unique across
//! the three files, so SQL never names the schema.
//!
//! No pool library: the write connection carries the change hooks, and the
//! order connections close in on reset and exit follows the store, which a
//! general pool's own lifecycle would have to be worked around for.

use super::{migrations, DataError};
use rusqlite::{
    hooks::{Action, AuthAction, AuthContext, Authorization},
    Connection, OpenFlags, Transaction, TransactionBehavior,
};
use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError},
};

/// Read connections kept open. A read that finds all of them busy opens a
/// short-lived one instead of waiting.
const READERS: usize = 3;

/// The files of a store, inside one directory.
pub(super) struct Files {
    pub(super) user: PathBuf,
    pub(super) cache: PathBuf,
    pub(super) sync: PathBuf,
}

impl Files {
    pub(super) fn in_directory(directory: PathBuf) -> Self {
        Self {
            user: directory.join(migrations::USER_FILE),
            cache: directory.join(migrations::CACHE_FILE),
            sync: directory.join(migrations::SYNC_FILE),
        }
    }
}

pub(super) struct Connections {
    pub(super) files: Files,
    // Fields drop in order: the read connections close first, so the write
    // connection is the last one and checkpoints and removes the WAL files.
    readers: Vec<Mutex<Connection>>,
    writer: Mutex<Connection>,
}

impl Connections {
    /// Opens the write connection, creating or migrating both schemas (the
    /// only place migrations run), then the read connections. Also returns
    /// whether a schema was created or rebuilt.
    pub(super) fn open(files: Files) -> Result<(Self, bool), DataError> {
        let mut writer = Connection::open(&files.user)?;
        writer.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let user_created = migrations::migrate_user(&mut writer)?;
        let cache_created = migrations::attach_caches(&mut writer, &files.cache, &files.sync)?;
        let user_upgraded = migrations::upgrade_user(&mut writer)?;
        // Both are fetched again when lost, so they trade the last commits
        // before a power loss for fewer syncs. A power loss can then keep a
        // key's freshness and lose its rows; the key reads stale until it is
        // read again, as after any lost cache commit.
        writer.execute_batch(
            "PRAGMA cache.journal_mode=WAL; PRAGMA cache.synchronous=NORMAL;
             PRAGMA sync.journal_mode=WAL; PRAGMA sync.synchronous=NORMAL;",
        )?;
        forbid_truncate(&mut writer)?;
        let mut connections = Self {
            files,
            writer: Mutex::new(writer),
            readers: Vec::with_capacity(READERS),
        };
        for _ in 0..READERS {
            let reader = connections.open_reader()?;
            connections.readers.push(Mutex::new(reader));
        }
        Ok((connections, user_created || cache_created || user_upgraded))
    }

    /// The write connection. Callers use it inside `spawn_blocking`; every
    /// commit of the store waits here.
    pub(super) fn writer(&self) -> MutexGuard<'_, Connection> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new read-only connection to the three files.
    pub(super) fn open_reader(&self) -> Result<Connection, DataError> {
        let connection = Connection::open_with_flags(
            &self.files.user,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        migrations::attach(&connection, &self.files.cache, &self.files.sync)?;
        Ok(connection)
    }

    /// A free pooled read connection, or a short-lived one when all are busy
    /// (possibly on this thread, so waiting could deadlock).
    pub(super) fn reader(&self) -> Result<Reader<'_>, DataError> {
        for slot in &self.readers {
            match slot.try_lock() {
                Ok(connection) => return Ok(Reader::Pooled(connection)),
                Err(TryLockError::Poisoned(poisoned)) => {
                    return Ok(Reader::Pooled(poisoned.into_inner()))
                }
                Err(TryLockError::WouldBlock) => {}
            }
        }
        Ok(Reader::Extra(self.open_reader()?))
    }
}

/// A read connection lent by [`Connections::reader`].
pub(crate) enum Reader<'a> {
    Pooled(MutexGuard<'a, Connection>),
    Extra(Connection),
}

impl Deref for Reader<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match self {
            Self::Pooled(connection) => connection,
            Self::Extra(connection) => connection,
        }
    }
}

impl DerefMut for Reader<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        match self {
            Self::Pooled(connection) => connection,
            Self::Extra(connection) => connection,
        }
    }
}

/// SQLite skips the update hook when it empties a table with the truncate
/// optimization (`DELETE FROM t` without `WHERE`). Answering `Ignore` to a
/// delete turns that optimization off; every row is still deleted, one by
/// one, and reported. `DROP TABLE` also asks for a delete of the dropped
/// table, so a schema rebuild lifts the guard first ([`allow_drops`]).
pub(super) fn forbid_truncate(connection: &mut Connection) -> Result<(), DataError> {
    connection.authorizer(Some(|context: AuthContext<'_>| match context.action {
        AuthAction::Delete { table_name } if !table_name.starts_with("sqlite_") => {
            Authorization::Ignore
        }
        _ => Authorization::Allow,
    }))?;
    Ok(())
}

/// Lifts [`forbid_truncate`] so `DROP TABLE` works; the caller installs it
/// again when the rebuild ends, whether it succeeded or not.
pub(super) fn allow_drops(connection: &mut Connection) -> Result<(), DataError> {
    connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
    Ok(())
}

/// FTS5 keeps a virtual table's rows in shadow tables named after it; a
/// change is reported under the virtual table's name.
fn reported_table(table: &str) -> &str {
    for fts in migrations::FTS_TABLES {
        if table
            .strip_prefix(*fts)
            .is_some_and(|rest| rest.starts_with('_'))
        {
            return fts;
        }
    }
    table
}

/// Runs `operation` in an IMMEDIATE transaction on `connection` and commits
/// it. Also returns the tables it inserted into, updated or deleted from
/// (SQLite's update hook, unqualified names); empty when nothing changed.
/// The store's writes commit through this; tests on an in-memory database
/// (`migrations::open_in_memory`) use it to commit the same way.
pub(crate) fn commit_on<T, E: From<DataError>>(
    connection: &mut Connection,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
) -> Result<(T, BTreeSet<String>), E> {
    commit_then(connection, operation, |_, value, _| Ok(value))
}

/// As `commit_on`, with `then` run on the transaction once the courses the
/// operation fed are merged, before it commits: what it reads is what
/// commits. It hears the tables written so far, merges included.
pub(crate) fn commit_then<T, U, E: From<DataError>>(
    connection: &mut Connection,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    then: impl FnOnce(&Transaction<'_>, T, &BTreeSet<String>) -> Result<U, E>,
) -> Result<(U, BTreeSet<String>), E> {
    let touched: Arc<Mutex<Vec<super::course_merge::Touched>>> = Arc::default();
    let sink = touched.clone();
    connection.update_hook(Some(
        move |action: Action, _: &str, table: &str, rowid: i64| {
            sink.lock().unwrap_or_else(PoisonError::into_inner).push((
                action,
                reported_table(table).to_string(),
                rowid,
            ));
        },
    ))
    .map_err(DataError::from)?;
    let result: Result<U, E> = (|| {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(DataError::from)?;
        let value = operation(&transaction)?;
        // Courses the write fed are merged again before it commits.
        let written = touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        super::course_merge::refresh(&transaction, &written)?;
        let tables = touched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, table, _)| table.clone())
            .collect();
        let value = then(&transaction, value, &tables)?;
        transaction.commit().map_err(DataError::from)?;
        Ok(value)
    })();
    // The write has already committed or rolled back; a hook that fails to
    // detach only keeps recording into a list nobody reads.
    let _ = connection.update_hook(None::<fn(Action, &str, &str, i64)>);
    let tables = std::mem::take(&mut *touched.lock().unwrap_or_else(PoisonError::into_inner))
        .into_iter()
        .map(|(_, table, _)| table)
        .collect();
    result.map(|value| (value, tables))
}

/// Deletes a database file with its WAL, shared-memory and rollback files.
/// Only for a file no connection has open: SQLite removes a WAL file by name
/// when the last connection to its database closes.
pub(super) fn remove_database(path: &Path) -> Result<(), DataError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        match std::fs::remove_file(PathBuf::from(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
