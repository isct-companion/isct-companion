//! The connections of one store: every commit queues on one long-lived write
//! connection; reads use a few long-lived read-only connections, all on the
//! store's one file.
//!
//! The file uses WAL with `synchronous=FULL` (`approach/single-file-wal-store`):
//! one log holds every table, so a commit is whole across a crash, a failed
//! write and a concurrent read, and a read keeps its snapshot without holding a
//! commit up. A write always begins IMMEDIATE, because a transaction that reads
//! and then writes would otherwise fail with `SQLITE_BUSY_SNAPSHOT`.
//!
//! A commit that reports an error has not happened as far as this process can
//! tell: it reads the old state, and the next commit writes over its frames. If
//! the error was a failed sync and the process then dies before another commit,
//! the next start can find that commit, whole. Callers treat an error as a
//! rollback, which holds for the run they are in.
//!
//! No pool library: the write connection carries the change hooks, and the
//! order connections close in on reset and exit follows the store, which a
//! general pool's own lifecycle would have to be worked around for. The write
//! connection closes last, which checkpoints and removes the log.

use super::{migrations, root::StorePaths, DataError};
use rusqlite::{
    hooks::{Action, AuthAction, AuthContext, Authorization},
    Connection, OpenFlags, Transaction, TransactionBehavior,
};
use std::{
    collections::BTreeSet,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError},
    time::Duration,
};

/// Read connections kept open. A read that finds all of them busy opens a
/// short-lived one instead of waiting.
const READERS: usize = 3;
/// How long a connection waits for another's lock on a file.
const BUSY: Duration = Duration::from_secs(10);

/// Where a long read's log stops growing back to: the log is cut to this size
/// once a checkpoint has caught up, rather than keeping its largest size.
const LOG_LIMIT_BYTES: u64 = 4 * 1024 * 1024;

/// Apple's `fsync` does not ask the drive to flush, so a commit that must reach
/// the disk asks with `F_FULLFSYNC`. With one sync per commit this costs a few
/// milliseconds, on the write connection only.
#[cfg(target_vendor = "apple")]
const DRIVE_FLUSH: &str = "PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON;";
#[cfg(not(target_vendor = "apple"))]
const DRIVE_FLUSH: &str = "";

pub(super) struct Connections {
    pub(super) path: PathBuf,
    // Fields drop in order: the read connections close first, the write
    // connection last.
    readers: Vec<Mutex<Connection>>,
    writer: Mutex<Connection>,
}

impl Connections {
    /// Opens the write connection, then the read connections, on the file
    /// `data::root` made ready at the current schema.
    pub(super) fn open(paths: &StorePaths) -> Result<Self, DataError> {
        let path = paths.store_db();
        let mut writer = Connection::open(&path)?;
        writer.busy_timeout(BUSY)?;
        writer.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
             PRAGMA journal_size_limit={LOG_LIMIT_BYTES}; {DRIVE_FLUSH}"
        ))?;
        forbid_truncate(&mut writer)?;
        let mut connections = Self {
            path,
            writer: Mutex::new(writer),
            readers: Vec::with_capacity(READERS),
        };
        for _ in 0..READERS {
            let reader = connections.open_reader()?;
            connections.readers.push(Mutex::new(reader));
        }
        Ok(connections)
    }

    /// The write connection. Callers use it inside `spawn_blocking`; every
    /// commit of the store waits here.
    pub(super) fn writer(&self) -> MutexGuard<'_, Connection> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new read-only connection.
    pub(super) fn open_reader(&self) -> Result<Connection, DataError> {
        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(BUSY)?;
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
    connection
        .update_hook(Some(
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
