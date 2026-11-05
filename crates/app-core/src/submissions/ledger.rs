//! The device-local record of LMS writes: preparations, operation records,
//! the snapshot objects each names, and the assignments to read again once
//! a write settled.
//!
//! It is its own SQLite file, `submissions.db`, in its own store
//! generation (`data::root`). Not user.db: backup and cloud sync read that
//! file whole, and a write in flight on this device means nothing on
//! another. Not the school group: it is replaced whenever its schema
//! changes, and losing a pending record would lift the single-flight guard
//! and let a write whose outcome is unknown be sent again. Only one Core
//! process holds the data folder (`core.lock`), so every record whose
//! `owner` is another run lost its executor. No write transaction is held
//! across a network wait. References to snapshot objects are rows
//! (`preparation_files`, `operation_files`), committed with what names them.
use super::{
    LmsIdentity, Operation, OperationState, Preparation, RemoteState, SnapshotFile,
    SubmissionError, RECEIPT_RETENTION,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Mutex, MutexGuard},
};

pub(super) const FILE: &str = "submissions.db";
/// The ledger format `store-index.db` records for its generation.
pub(super) const FORMAT: i64 = 3;
/// Formats of earlier releases, replaced by an empty ledger: hand-ins are
/// confirmed before upgrading (`approach/short-lived-store-rebuild`).
pub(super) const OLDER_FORMATS: &[i64] = &[1, 2];

const SCHEMA: &str = r#"
CREATE TABLE preparations (
  id TEXT PRIMARY KEY, expires_at INTEGER NOT NULL, value TEXT NOT NULL
);
CREATE TABLE preparation_files (
  preparation_id TEXT NOT NULL REFERENCES preparations(id) ON DELETE CASCADE,
  name TEXT NOT NULL, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
  PRIMARY KEY (preparation_id, name)
);
CREATE INDEX preparation_files_by_object ON preparation_files(sha256);
CREATE TABLE operations (
  id TEXT PRIMARY KEY, preparation_id TEXT NOT NULL,
  site TEXT NOT NULL, user_id INTEGER NOT NULL, assign_id INTEGER NOT NULL,
  key TEXT NOT NULL, state TEXT NOT NULL, owner TEXT NOT NULL,
  next_check INTEGER, checks INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL, value TEXT NOT NULL
);
CREATE UNIQUE INDEX operations_one_unsettled ON operations(site, user_id, assign_id)
  WHERE state NOT IN ('succeeded','failed');
CREATE INDEX operations_by_key ON operations(key);
CREATE INDEX operations_by_preparation ON operations(preparation_id);
CREATE TABLE operation_files (
  operation_id TEXT NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
  name TEXT NOT NULL, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL,
  PRIMARY KEY (operation_id, name)
);
CREATE INDEX operation_files_by_object ON operation_files(sha256);
CREATE TABLE refresh_outbox (
  operation_id TEXT PRIMARY KEY, lms_course_id INTEGER NOT NULL,
  revision INTEGER NOT NULL, next_attempt INTEGER NOT NULL
);
"#;

/// States that end a write; every other state blocks new writes.
const SETTLED: &str = "('succeeded','failed')";

/// How `claim` ended.
pub(super) enum Claim {
    /// The write is recorded with its snapshot references.
    Claimed(Box<Preparation>, Box<Operation>),
    /// A write for the same account and assignment is not settled yet.
    Busy(Box<Operation>),
    /// The preparation expired or is unknown.
    Expired,
}

/// An assignment to read again after a write settled.
pub(super) struct Refresh {
    pub(super) operation: String,
    pub(super) lms_course_id: u64,
    /// The resource revision the write raised; an earlier read is not it.
    pub(super) revision: u64,
}

pub(super) struct Ledger {
    connection: Mutex<Connection>,
}

impl Ledger {
    /// Creates the ledger in a new generation's folder.
    pub(super) fn create(path: &Path) -> Result<(), SubmissionError> {
        let mut connection = connect(path)?;
        let transaction = connection.transaction()?;
        transaction.execute_batch(SCHEMA)?;
        transaction.pragma_update(None, "user_version", FORMAT)?;
        transaction.commit()?;
        Ok(())
    }

    pub(super) fn open(path: &Path) -> Result<Self, SubmissionError> {
        let connection = connect(path)?;
        let found: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if found != FORMAT {
            return Err(SubmissionError::UnsupportedLedger {
                found,
                expected: FORMAT,
            });
        }
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, SubmissionError> {
        self.connection
            .lock()
            .map_err(|_| SubmissionError::Storage("The submission ledger is unavailable".into()))
    }

    pub(super) fn preparation(&self, id: &str) -> Result<Option<Preparation>, SubmissionError> {
        preparation(&*self.lock()?, id)
    }

    /// Records `preparation` with its snapshot references, in one
    /// transaction, unless the same preparation is recorded and unexpired:
    /// then that one answers. Runs under the snapshots' publication lock.
    pub(super) fn publish_preparation(
        &self,
        preparation: &Preparation,
        now: i64,
    ) -> Result<Preparation, SubmissionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = preparation_of(&transaction, &preparation.id)? {
            if existing.expires_at > now {
                return Ok(existing);
            }
        }
        transaction.execute("DELETE FROM preparations WHERE id=?1", [&preparation.id])?;
        transaction.execute(
            "INSERT INTO preparations (id, expires_at, value) VALUES (?1, ?2, ?3)",
            params![
                preparation.id,
                preparation.expires_at,
                serde_json::to_string(preparation)?
            ],
        )?;
        insert_files(&transaction, "preparation_files", "preparation_id", &preparation.id, &preparation.files)?;
        transaction.commit()?;
        Ok(preparation.clone())
    }

    /// Drops a preparation whose write succeeded; its objects stay while
    /// the operation names them.
    pub(super) fn release(&self, preparation: &str) -> Result<(), SubmissionError> {
        self.lock()?
            .execute("DELETE FROM preparations WHERE id=?1", [preparation])?;
        Ok(())
    }

    /// In one IMMEDIATE transaction: reads the preparation, checks it has
    /// not expired and that `check` accepts it, refuses while a write for
    /// its account and assignment is not settled, and records the write
    /// `operation` makes of it with its snapshot references. Once it
    /// commits, the preparation's expiry no longer matters.
    pub(super) fn claim(
        &self,
        preparation_id: &str,
        now: i64,
        owner: &str,
        check: impl FnOnce(&Preparation) -> Result<(), SubmissionError>,
        operation: impl FnOnce(&Preparation) -> Operation,
    ) -> Result<Claim, SubmissionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(preparation) =
            preparation_of(&transaction, preparation_id)?.filter(|found| found.expires_at > now)
        else {
            return Ok(Claim::Expired);
        };
        check(&preparation)?;
        let operation = operation(&preparation);
        let mut busy = operations(
            &transaction,
            &format!(
                "SELECT value FROM operations WHERE site=?1 AND user_id=?2 AND assign_id=?3
                 AND state NOT IN {SETTLED} ORDER BY rowid"
            ),
            params![
                operation.identity.site,
                operation.identity.user_id as i64,
                operation.assign_id as i64
            ],
        )?;
        if !busy.is_empty() {
            // The same write, when it is among them, answers for itself.
            let same = busy.iter().position(|other| other.key == operation.key);
            return Ok(Claim::Busy(Box::new(busy.swap_remove(same.unwrap_or(0)))));
        }
        transaction.execute(
            "INSERT INTO operations (id, preparation_id, site, user_id, assign_id, key, state, owner, updated_at, value)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                operation.id,
                operation.preparation_id,
                operation.identity.site,
                operation.identity.user_id as i64,
                operation.assign_id as i64,
                operation.key,
                operation.state.name(),
                owner,
                operation.updated_at,
                serde_json::to_string(&operation)?,
            ],
        )?;
        insert_files(&transaction, "operation_files", "operation_id", &operation.id, &operation.files)?;
        transaction.commit()?;
        Ok(Claim::Claimed(Box::new(preparation), Box::new(operation)))
    }

    /// Stores `operation` as it is now, owned by `owner`, only while its
    /// recorded state is `from`. False when something else moved it first:
    /// a cancel or recovery. A settled state commits with `refresh`, the
    /// assignment to read again, in the same transaction.
    pub(super) fn transition(
        &self,
        operation: &Operation,
        from: OperationState,
        owner: &str,
        refresh: Option<Refresh>,
        now: i64,
    ) -> Result<bool, SubmissionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE operations SET state=?1, owner=?2, updated_at=?3, value=?4,
               next_check=CASE WHEN ?1='outcome_unknown' THEN COALESCE(next_check, ?7) ELSE NULL END
             WHERE id=?5 AND state=?6",
            params![
                operation.state.name(),
                owner,
                operation.updated_at,
                serde_json::to_string(operation)?,
                operation.id,
                from.name(),
                operation.updated_at + super::CHECK_FIRST,
            ],
        )?;
        if changed == 1 {
            if let Some(refresh) = refresh {
                transaction.execute(
                    "INSERT OR REPLACE INTO refresh_outbox (operation_id, lms_course_id, revision, next_attempt)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        refresh.operation,
                        refresh.lms_course_id as i64,
                        refresh.revision as i64,
                        now
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(changed == 1)
    }

    /// Forgets a claimed write that was refused before anything was sent.
    pub(super) fn withdraw(&self, operation: &str) -> Result<(), SubmissionError> {
        self.lock()?.execute(
            "DELETE FROM operations WHERE id=?1 AND state='prepared'",
            [operation],
        )?;
        Ok(())
    }

    pub(super) fn operation(&self, id: &str) -> Result<Option<Operation>, SubmissionError> {
        Ok(operations(
            &*self.lock()?,
            "SELECT value FROM operations WHERE id=?1",
            params![id],
        )?
        .pop())
    }

    /// The latest write of `preparation`.
    pub(super) fn latest_for(
        &self,
        preparation: &str,
    ) -> Result<Option<Operation>, SubmissionError> {
        Ok(operations(
            &*self.lock()?,
            "SELECT value FROM operations WHERE preparation_id=?1 ORDER BY rowid DESC LIMIT 1",
            params![preparation],
        )?
        .pop())
    }

    /// The write not settled yet for one account and assignment.
    pub(super) fn unsettled(
        &self,
        identity: &LmsIdentity,
        assign_id: u64,
    ) -> Result<Vec<Operation>, SubmissionError> {
        operations(
            &*self.lock()?,
            &format!(
                "SELECT value FROM operations WHERE site=?1 AND user_id=?2 AND assign_id=?3
                 AND state NOT IN {SETTLED} ORDER BY rowid"
            ),
            params![identity.site, identity.user_id as i64, assign_id as i64],
        )
    }

    /// Unsettled writes another run owned: their executor is gone.
    pub(super) fn orphaned(&self, owner: &str) -> Result<Vec<Operation>, SubmissionError> {
        operations(
            &*self.lock()?,
            &format!(
                "SELECT value FROM operations WHERE owner<>?1 AND state NOT IN {SETTLED} ORDER BY rowid"
            ),
            params![owner],
        )
    }

    /// A write of the same content Core confirmed, while the LMS still
    /// holds exactly what it found then.
    pub(super) fn receipt(
        &self,
        key: &str,
        state: &RemoteState,
    ) -> Result<Option<Operation>, SubmissionError> {
        Ok(operations(
            &*self.lock()?,
            "SELECT value FROM operations WHERE key=?1 AND state='succeeded' ORDER BY rowid DESC",
            params![key],
        )?
        .into_iter()
        .find(|operation| operation.verified.as_ref() == Some(state)))
    }

    /// Writes whose outcome is unknown and whose next check has come, with
    /// how many checks each had.
    pub(super) fn checks_due(&self, now: i64) -> Result<Vec<(Operation, u32)>, SubmissionError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT value, checks FROM operations
             WHERE state='outcome_unknown' AND next_check IS NOT NULL AND next_check<=?1
             ORDER BY next_check",
        )?;
        let rows = statement
            .query_map([now], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(value, checks)| Ok((serde_json::from_str(&value)?, checks)))
            .collect()
    }

    /// When an unknown write is read next. `queue` sets `at` only where no
    /// check is queued; otherwise `at` replaces the time and counts a check.
    pub(super) fn schedule_check(
        &self,
        operation: &str,
        at: i64,
        queue: bool,
    ) -> Result<(), SubmissionError> {
        let connection = self.lock()?;
        if queue {
            connection.execute(
                "UPDATE operations SET next_check=COALESCE(next_check, ?2)
                 WHERE id=?1 AND state='outcome_unknown'",
                params![operation, at],
            )?;
        } else {
            connection.execute(
                "UPDATE operations SET next_check=?2, checks=checks+1
                 WHERE id=?1 AND state='outcome_unknown'",
                params![operation, at],
            )?;
        }
        Ok(())
    }

    /// When the next check or refresh is due, in Unix seconds.
    pub(super) fn next_due(&self) -> Result<Option<i64>, SubmissionError> {
        Ok(self.lock()?.query_row(
            "SELECT MIN(at) FROM (
               SELECT MIN(next_check) AS at FROM operations WHERE state='outcome_unknown'
               UNION ALL SELECT MIN(next_attempt) FROM refresh_outbox)",
            [],
            |row| row.get(0),
        )?)
    }

    pub(super) fn refreshes_due(&self, now: i64) -> Result<Vec<Refresh>, SubmissionError> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT operation_id, lms_course_id, revision FROM refresh_outbox
             WHERE next_attempt<=?1 ORDER BY next_attempt",
        )?;
        let rows = statement
            .query_map([now], |row| {
                Ok(Refresh {
                    operation: row.get(0)?,
                    lms_course_id: row.get::<_, i64>(1)? as u64,
                    revision: row.get::<_, i64>(2)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The refresh landed, or comes again at `retry`.
    pub(super) fn refreshed(&self, operation: &str, retry: Option<i64>) -> Result<(), SubmissionError> {
        let connection = self.lock()?;
        match retry {
            None => connection.execute("DELETE FROM refresh_outbox WHERE operation_id=?1", [operation])?,
            Some(at) => connection.execute(
                "UPDATE refresh_outbox SET next_attempt=?2 WHERE operation_id=?1",
                params![operation, at],
            )?,
        };
        Ok(())
    }

    /// Maintenance, first half: drops expired preparations and settled
    /// writes past their retention (never one with a refresh to make), with
    /// their references, in one committed transaction; then answers every
    /// object still referenced.
    pub(super) fn expire(&self, now: i64) -> Result<BTreeSet<String>, SubmissionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("DELETE FROM preparations WHERE expires_at <= ?1", [now])?;
        transaction.execute(
            &format!(
                "DELETE FROM operations WHERE state IN {SETTLED} AND updated_at <= ?1
                 AND id NOT IN (SELECT operation_id FROM refresh_outbox)"
            ),
            [now - RECEIPT_RETENTION],
        )?;
        transaction.commit()?;
        let mut statement = connection.prepare(
            "SELECT sha256 FROM preparation_files UNION SELECT sha256 FROM operation_files",
        )?;
        let referenced = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(referenced)
    }
}

pub(super) fn connect(path: &Path) -> Result<Connection, SubmissionError> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
    )?;
    Ok(connection)
}

fn insert_files(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    id: &str,
    files: &[SnapshotFile],
) -> Result<(), SubmissionError> {
    for file in files {
        transaction.execute(
            &format!("INSERT INTO {table} ({column}, name, sha256, bytes) VALUES (?1, ?2, ?3, ?4)"),
            params![id, file.name, file.sha256, file.bytes as i64],
        )?;
    }
    Ok(())
}

fn preparation(connection: &Connection, id: &str) -> Result<Option<Preparation>, SubmissionError> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM preparations WHERE id=?1", [id], |row| {
            row.get(0)
        })
        .optional()?;
    value
        .map(|value| serde_json::from_str(&value).map_err(SubmissionError::from))
        .transpose()
}

fn preparation_of(
    transaction: &Transaction<'_>,
    id: &str,
) -> Result<Option<Preparation>, SubmissionError> {
    preparation(transaction, id)
}

fn operations(
    connection: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<Operation>, SubmissionError> {
    let mut statement = connection.prepare(sql)?;
    let values = statement
        .query_map(params, |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    values
        .iter()
        .map(|value| serde_json::from_str(value).map_err(SubmissionError::from))
        .collect()
}
