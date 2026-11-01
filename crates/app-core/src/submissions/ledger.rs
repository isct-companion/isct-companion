//! The device-local record of LMS writes: preparations, operation records
//! and the content-addressed snapshots they upload.
//!
//! It is its own SQLite file, `submissions.db`, beside the store's three.
//! Not user.db: backup and cloud sync read that file whole, and a write in
//! flight on this device means nothing on another. Not cache.db or sync.db:
//! both are emptied when their schema changes or the caches are reset, and
//! losing a pending record would lift the single-flight guard and let a
//! write whose outcome is unknown be sent again. Not a JSON file like the
//! reminder log: a dev build and the app can share one data folder, and
//! only SQLite's lock makes "one write per account and assignment" hold
//! across processes. Every statement commits on its own, so no write
//! transaction is ever held across a network wait. Snapshots live in
//! `submissions/` beside it, named by their sha256.
use super::{
    LmsIdentity, Operation, OperationState, Preparation, RemoteState, SnapshotFile,
    SubmissionError, UploadFile, RECEIPT_RETENTION,
};
use crate::workspaces::HandInError;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

const FILE: &str = "submissions.db";
const BLOBS: &str = "submissions";
const SCHEMA_VERSION: i64 = 1;
/// A partial snapshot copy this old was left by a crash.
const STALE_PARTIAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS preparations (
  id TEXT PRIMARY KEY, expires_at INTEGER NOT NULL, value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS operations (
  id TEXT PRIMARY KEY, preparation_id TEXT NOT NULL, account TEXT NOT NULL,
  site TEXT NOT NULL, assign_id INTEGER NOT NULL, key TEXT NOT NULL,
  state TEXT NOT NULL, updated_at INTEGER NOT NULL, value TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS operations_by_assignment ON operations(account, site, assign_id);
CREATE INDEX IF NOT EXISTS operations_by_key ON operations(key);
CREATE INDEX IF NOT EXISTS operations_by_preparation ON operations(preparation_id);
"#;

/// States that end a write; every other state blocks new writes.
const SETTLED: &str = "('succeeded','failed')";

pub(super) enum Claim {
    Claimed,
    /// A write for the same account and assignment is not settled yet.
    Busy(Box<Operation>),
}

pub(super) struct Ledger {
    connection: Mutex<Connection>,
    blobs: PathBuf,
}

impl Ledger {
    pub(super) fn open(directory: &Path) -> Result<Self, SubmissionError> {
        let connection = Connection::open(directory.join(FILE))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        match version {
            0 => {
                connection.execute_batch(SCHEMA)?;
                connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            }
            SCHEMA_VERSION => {}
            other => {
                return Err(SubmissionError::Storage(format!(
                    "{FILE} schema {other} is not supported"
                )))
            }
        }
        Ok(Self {
            connection: Mutex::new(connection),
            blobs: directory.join(BLOBS),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, SubmissionError> {
        self.connection
            .lock()
            .map_err(|_| SubmissionError::Storage("The submission ledger is unavailable".into()))
    }

    pub(super) fn preparation(&self, id: &str) -> Result<Option<Preparation>, SubmissionError> {
        let value: Option<String> = self
            .lock()?
            .query_row("SELECT value FROM preparations WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(SubmissionError::from))
            .transpose()
    }

    pub(super) fn put_preparation(&self, preparation: &Preparation) -> Result<(), SubmissionError> {
        let value = serde_json::to_string(preparation)?;
        self.lock()?.execute(
            "INSERT OR REPLACE INTO preparations (id, expires_at, value) VALUES (?1, ?2, ?3)",
            params![preparation.id, preparation.expires_at, value],
        )?;
        Ok(())
    }

    /// Drops a preparation whose write succeeded; its snapshot goes with
    /// the next `clean`.
    pub(super) fn release(&self, preparation: &str) -> Result<(), SubmissionError> {
        self.lock()?
            .execute("DELETE FROM preparations WHERE id=?1", [preparation])?;
        Ok(())
    }

    /// Records `operation` unless a write for its account and assignment is
    /// not settled yet, in one IMMEDIATE transaction: the check and the
    /// insert cannot interleave with another process's.
    pub(super) fn claim(&self, operation: &Operation) -> Result<Claim, SubmissionError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut busy = operations(
            &transaction,
            &format!(
                "SELECT value FROM operations WHERE account=?1 AND site=?2 AND assign_id=?3
                 AND state NOT IN {SETTLED} ORDER BY rowid"
            ),
            params![
                operation.identity.account,
                operation.identity.site,
                operation.assign_id as i64
            ],
        )?;
        if !busy.is_empty() {
            // The same write, when it is among them, answers for itself.
            let same = busy.iter().position(|other| other.key == operation.key);
            return Ok(Claim::Busy(Box::new(busy.swap_remove(same.unwrap_or(0)))));
        }
        transaction.execute(
            "INSERT INTO operations (id, preparation_id, account, site, assign_id, key, state, updated_at, value)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                operation.id,
                operation.preparation_id,
                operation.identity.account,
                operation.identity.site,
                operation.assign_id as i64,
                operation.key,
                operation.state.name(),
                operation.updated_at,
                serde_json::to_string(operation)?,
            ],
        )?;
        transaction.commit()?;
        Ok(Claim::Claimed)
    }

    /// Stores `operation` as it is now, only while its recorded state is
    /// one of `from`. False when something else moved it first: a cancel,
    /// recovery, or another process.
    pub(super) fn transition(
        &self,
        operation: &Operation,
        from: &[OperationState],
    ) -> Result<bool, SubmissionError> {
        let states = from
            .iter()
            .map(|state| format!("'{}'", state.name()))
            .collect::<Vec<_>>()
            .join(",");
        let changed = self.lock()?.execute(
            &format!(
                "UPDATE operations SET state=?1, updated_at=?2, value=?3
                 WHERE id=?4 AND state IN ({states})"
            ),
            params![
                operation.state.name(),
                operation.updated_at,
                serde_json::to_string(operation)?,
                operation.id
            ],
        )?;
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

    /// Writes not settled yet, oldest first; for one account and
    /// assignment, or all of them.
    pub(super) fn unsettled(
        &self,
        assignment: Option<(&LmsIdentity, u64)>,
    ) -> Result<Vec<Operation>, SubmissionError> {
        let connection = self.lock()?;
        match assignment {
            Some((identity, assign_id)) => operations(
                &connection,
                &format!(
                    "SELECT value FROM operations WHERE account=?1 AND site=?2 AND assign_id=?3
                     AND state NOT IN {SETTLED} ORDER BY rowid"
                ),
                params![identity.account, identity.site, assign_id as i64],
            ),
            None => operations(
                &connection,
                &format!(
                    "SELECT value FROM operations WHERE state NOT IN {SETTLED} ORDER BY rowid"
                ),
                params![],
            ),
        }
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

    /// Drops expired preparations, receipts past their retention and every
    /// snapshot neither a preparation nor an unsettled write still names.
    pub(super) fn clean(&self, now: i64) -> Result<(), SubmissionError> {
        let mut referenced = BTreeSet::new();
        {
            let connection = self.lock()?;
            connection.execute("DELETE FROM preparations WHERE expires_at <= ?1", [now])?;
            connection.execute(
                &format!("DELETE FROM operations WHERE state IN {SETTLED} AND updated_at <= ?1"),
                [now - RECEIPT_RETENTION],
            )?;
            let mut statement = connection.prepare("SELECT value FROM preparations")?;
            for value in statement.query_map([], |row| row.get::<_, String>(0))? {
                let preparation: Preparation = serde_json::from_str(&value?)?;
                referenced.extend(preparation.files.into_iter().map(|file| file.sha256));
            }
            drop(statement);
            for operation in operations(
                &connection,
                &format!("SELECT value FROM operations WHERE state NOT IN {SETTLED}"),
                params![],
            )? {
                referenced.extend(operation.files.into_iter().map(|file| file.sha256));
            }
        }
        let Ok(entries) = fs::read_dir(&self.blobs) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let keep = if name.ends_with(".partial") {
                // Another snapshot may be copying it right now.
                entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_none_or(|age| age < STALE_PARTIAL)
            } else {
                referenced.contains(&name)
            };
            if !keep {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }

    /// Copies the regular files `names` of `submit` into the snapshot store,
    /// hashing the bytes as they are copied, so the digest is that of what
    /// is kept, whatever happens to `submit/` meanwhile. By name.
    pub(super) fn snapshot(
        &self,
        submit: &Path,
        names: &[String],
    ) -> Result<Vec<SnapshotFile>, SubmissionError> {
        if names.is_empty() {
            return Err(HandInError::Empty.into());
        }
        fs::create_dir_all(&self.blobs)?;
        let mut files = Vec::new();
        for name in names {
            let source = submit.join(name);
            let kind = fs::symlink_metadata(&source)?;
            if kind.is_dir() {
                return Err(HandInError::NeedsZip.into());
            }
            if !kind.is_file() {
                return Err(HandInError::FilesOnly.into());
            }
            let (sha256, bytes) = self.keep(&source)?;
            files.push(SnapshotFile {
                name: name.clone(),
                bytes,
                sha256,
            });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(files)
    }

    fn keep(&self, source: &Path) -> Result<(String, u64), SubmissionError> {
        let partial = self.blobs.join(format!("{}.partial", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut input = fs::File::open(source)?;
            let mut output = fs::File::create(&partial)?;
            let mut hasher = Sha256::new();
            let mut buffer = vec![0; 64 * 1024];
            let mut bytes = 0_u64;
            loop {
                let read = input.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                output.write_all(&buffer[..read])?;
                bytes += read as u64;
            }
            output.sync_all()?;
            let sha256 = data_encoding::HEXLOWER.encode(&hasher.finalize());
            let blob = self.blobs.join(&sha256);
            // A damaged copy under this name is replaced, not trusted.
            if blob.exists() && digest_of(&blob).ok() != Some((sha256.clone(), bytes)) {
                fs::remove_file(&blob)?;
            }
            if !blob.exists() {
                fs::rename(&partial, blob)?;
            }
            Ok::<_, SubmissionError>((sha256, bytes))
        })();
        let _ = fs::remove_file(&partial);
        result
    }

    /// Whether every file's snapshot is still there with its size and digest.
    pub(super) fn intact(&self, files: &[SnapshotFile]) -> bool {
        files.iter().all(|file| {
            digest_of(&self.blobs.join(&file.sha256)).ok()
                == Some((file.sha256.clone(), file.bytes))
        })
    }

    /// The snapshot files as the transport uploads them.
    pub(super) fn uploads(&self, files: &[SnapshotFile]) -> Vec<UploadFile> {
        files
            .iter()
            .map(|file| UploadFile {
                name: file.name.clone(),
                path: self.blobs.join(&file.sha256),
                bytes: file.bytes,
            })
            .collect()
    }
}

/// A file's sha256 and size.
fn digest_of(path: &Path) -> std::io::Result<(String, u64)> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes += read as u64;
    }
    Ok((data_encoding::HEXLOWER.encode(&hasher.finalize()), bytes))
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
