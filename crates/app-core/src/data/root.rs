//! The data folder's single owner and the generations of its stores
//! (`approach/data-root-single-owner`, `approach/store-generation-publication`).
//!
//! ```text
//! <data>/core.lock                       held by one Core process
//! <data>/store-index.db                  which generation of each store is active
//! <data>/store/<generation>/store.db     every table of the workspace
//! <data>/submissions/<generation>/…      the LMS write ledger (agent builds)
//! <data>/local/…                         this device's fixed files
//! ```
//!
//! [`StoreRoot::open`] takes an OS exclusive lock on `core.lock` before any
//! database is opened, recovered or read from the network, and holds it as
//! long as anything holds the root: every connection and task of a store
//! keeps an `Arc` of it. A second process, a dev build included, gets
//! [`StoreOpenError::DirectoryInUse`]; the lock is never taken over by PID or
//! age, and a crash releases it with the process.
//!
//! A store is replaced the way an OTA update replaces a system: the new
//! generation is built in its own directory, checked and synced to disk, and
//! only then does one `store-index.db` transaction make it active and retire
//! the old one. A failure leaves the index, and the old generation in use,
//! as they were. No database file in use is renamed or deleted; retired
//! generations go in low-frequency maintenance ([`StoreRoot::collect_retired`]).
//! Every path comes from [`StorePaths`].
//!
//! The user tables are not migrated in place: another user format is refused
//! with the file intact. School tables of another format are only fetched data,
//! so they are dropped and created empty in one transaction of the store.
//!
//! A folder with no active store gets a new, empty one; files of the layouts
//! before 1.4.0 are not read. A database file is never renamed over another: a
//! log left beside the old file would be read as the new file's.
use super::{migrations, DataError};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

const LOCK_FILE: &str = "core.lock";
const INDEX_FILE: &str = "store-index.db";
const INDEX_FORMAT: i64 = 1;
const LOCAL_DIR: &str = "local";
/// How long a retired generation stays as recovery material.
pub(crate) const RETIRED_GRACE: chrono::Duration = chrono::Duration::days(7);
/// The flat files of the release before generations, retired as one by the
/// upgrade that copied them into a store; an index 1.4.0 wrote may still name
/// them until maintenance collects them.
const LEGACY_KIND: &str = "legacy";
const LEGACY_FILES: &[&str] = &[
    "user.db",
    "cache.db",
    "sync.db",
    "submissions.db",
    "school.db.pre-split",
];
/// The flat layout kept snapshot blobs as plain files in `submissions/`,
/// which now also holds the ledger's generations: only its files go.
const LEGACY_BLOBS: &str = "submissions";

const INDEX_SCHEMA: &str = r#"
CREATE TABLE active_store (
  kind TEXT PRIMARY KEY, generation TEXT NOT NULL, format INTEGER NOT NULL
);
CREATE TABLE retired_store (
  kind TEXT NOT NULL, generation TEXT NOT NULL, retired_at INTEGER NOT NULL,
  PRIMARY KEY (kind, generation)
);
CREATE TABLE maintenance (
  key TEXT PRIMARY KEY, last_success INTEGER, next_due INTEGER NOT NULL
);
"#;

/// The stores whose generations the index selects.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoreKind {
    /// `store-index.db` itself.
    Index,
    /// store.db: every table of the workspace.
    Store,
    Submissions,
}

impl StoreKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Store => "store",
            Self::Submissions => "submissions",
        }
    }
}

/// Why a data folder could not be opened. Nothing in the folder changed.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StoreOpenError {
    /// Another Core process, a dev build included, holds the folder.
    #[error("Another copy of the app is using this data folder. Quit it and try again.")]
    DirectoryInUse,
    #[error("The {} store has format {found}, which this version cannot read (it reads {expected}). Your data was left intact.", .store.name())]
    UnsupportedFormat {
        store: StoreKind,
        found: i64,
        expected: i64,
    },
    /// The index names a generation whose files are gone.
    #[error("The active {} store is missing; the data folder needs recovery.", .store.name())]
    RecoveryRequired { store: StoreKind },
    #[error("The data folder could not be opened: {0}")]
    Io(String),
}

impl From<std::io::Error> for StoreOpenError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<rusqlite::Error> for StoreOpenError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<DataError> for StoreOpenError {
    fn from(error: DataError) -> Self {
        Self::Io(error.to_string())
    }
}

/// Every path of a data folder, for the generations active now.
#[derive(Clone, Debug)]
pub struct StorePaths {
    root: PathBuf,
    store: PathBuf,
}

impl StorePaths {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn store_db(&self) -> PathBuf {
        self.store.join(migrations::STORE_FILE)
    }

    /// A fixed file of this device, outside every generation.
    pub fn local(&self, name: &str) -> PathBuf {
        self.root.join(LOCAL_DIR).join(name)
    }

    /// The directory of one generation of `kind`.
    pub fn generation(&self, kind: StoreKind, generation: &str) -> PathBuf {
        self.root.join(kind.name()).join(generation)
    }
}

/// One store generation the index selects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generation {
    pub id: String,
    pub format: i64,
}

/// A generation built and synced to disk, not active yet.
pub(crate) struct Candidate {
    pub(crate) kind: StoreKind,
    pub(crate) generation: Generation,
}

/// The owner of one data folder. See the module comment.
pub struct StoreRoot {
    paths: StorePaths,
    /// Names this process's run: staging and operation owners carry it.
    boot: String,
    index: Mutex<Connection>,
    /// Whether this open created the store or emptied its school tables.
    created: bool,
    // Last: the lock outlives the index connection above.
    _lock: fs::File,
}

impl std::fmt::Debug for StoreRoot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoreRoot")
            .field("root", &self.paths.root)
            .field("boot", &self.boot)
            .finish_non_exhaustive()
    }
}

impl StoreRoot {
    /// Takes the folder and makes its store generation ready.
    pub fn open(directory: PathBuf) -> Result<Arc<Self>, StoreOpenError> {
        fs::create_dir_all(&directory)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(LOCK_FILE))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(StoreOpenError::DirectoryInUse),
            Err(fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let index = open_index(&directory.join(INDEX_FILE))?;
        let mut root = Self {
            paths: StorePaths {
                root: directory.clone(),
                store: PathBuf::new(),
            },
            boot: uuid::Uuid::now_v7().to_string(),
            index: Mutex::new(index),
            created: false,
            _lock: lock,
        };
        root.select()?;
        Ok(Arc::new(root))
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    pub fn boot(&self) -> &str {
        &self.boot
    }

    pub(crate) fn created(&self) -> bool {
        self.created
    }

    fn index(&self) -> MutexGuard<'_, Connection> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The generation of `kind` the index makes active.
    pub(crate) fn active(&self, kind: StoreKind) -> Result<Option<Generation>, StoreOpenError> {
        Ok(self
            .index()
            .query_row(
                "SELECT generation, format FROM active_store WHERE kind=?1",
                [kind.name()],
                |row| {
                    Ok(Generation {
                        id: row.get(0)?,
                        format: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// A new, empty generation directory of `kind`.
    pub(crate) fn new_generation(
        &self,
        kind: StoreKind,
    ) -> Result<(String, PathBuf), StoreOpenError> {
        let id = uuid::Uuid::now_v7().to_string();
        let directory = self.paths.generation(kind, &id);
        fs::create_dir_all(&directory)?;
        Ok((id, directory))
    }

    /// Makes every candidate active and retires what it replaces, in one
    /// index transaction.
    pub(crate) fn publish(&self, candidates: &[Candidate]) -> Result<(), StoreOpenError> {
        let now = crate::clock::now().timestamp();
        let mut index = self.index();
        let transaction =
            index.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for candidate in candidates {
            let replaced: Option<String> = transaction
                .query_row(
                    "SELECT generation FROM active_store WHERE kind=?1",
                    [candidate.kind.name()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(replaced) = replaced {
                transaction.execute(
                    "INSERT OR REPLACE INTO retired_store (kind, generation, retired_at) VALUES (?1, ?2, ?3)",
                    params![candidate.kind.name(), replaced, now],
                )?;
            }
            transaction.execute(
                "INSERT OR REPLACE INTO active_store (kind, generation, format) VALUES (?1, ?2, ?3)",
                params![
                    candidate.kind.name(),
                    candidate.generation.id,
                    candidate.generation.format
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Selects the store generation, or builds and publishes an empty first
    /// one.
    fn select(&mut self) -> Result<(), StoreOpenError> {
        if let Some(active) = self.active(StoreKind::Store)? {
            let directory = self.paths.generation(StoreKind::Store, &active.id);
            let path = directory.join(migrations::STORE_FILE);
            if !path.is_file() {
                return Err(StoreOpenError::RecoveryRequired {
                    store: StoreKind::Store,
                });
            }
            self.created = make_school_current(&path)?;
            self.paths.store = directory;
            return Ok(());
        }
        let (id, directory) = self.new_generation(StoreKind::Store)?;
        let path = directory.join(migrations::STORE_FILE);
        let built = migrations::create(&path).map_err(StoreOpenError::from);
        finish_candidate(&directory, built)?;
        log::info!(target: "app_core::data", "Built store generation {id}");
        self.publish(&[Candidate {
            kind: StoreKind::Store,
            generation: Generation {
                id,
                format: migrations::STORE_VERSION,
            },
        }])?;
        self.created = true;
        self.paths.store = directory;
        Ok(())
    }

    /// When `key`'s maintenance is next due, in Unix seconds.
    pub(crate) fn maintenance_due(&self, key: &str) -> Result<Option<i64>, DataError> {
        Ok(self
            .index()
            .query_row(
                "SELECT next_due FROM maintenance WHERE key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn schedule_maintenance(
        &self,
        key: &str,
        succeeded_at: Option<i64>,
        next_due: i64,
    ) -> Result<(), DataError> {
        self.index().execute(
            "INSERT INTO maintenance (key, last_success, next_due) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
               last_success=COALESCE(excluded.last_success, last_success),
               next_due=excluded.next_due",
            params![key, succeeded_at, next_due],
        )?;
        Ok(())
    }

    /// Deletes generations retired before `now - RETIRED_GRACE`. This
    /// process never opens a retired generation, so none is in use. A row
    /// goes only once its files have.
    pub(crate) fn collect_retired(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, DataError> {
        let before = (now - RETIRED_GRACE).timestamp();
        let retired: Vec<(String, String)> = {
            let index = self.index();
            let mut statement = index
                .prepare("SELECT kind, generation FROM retired_store WHERE retired_at <= ?1")?;
            let rows = statement
                .query_map([before], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut collected = 0;
        for (kind, generation) in retired {
            let removed = if kind == LEGACY_KIND {
                self.remove_legacy()
            } else if generation.is_empty() || generation.contains(['/', '\\', '.']) {
                Ok(())
            } else {
                match fs::remove_dir_all(self.paths.root.join(&kind).join(&generation)) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                    _ => Ok(()),
                }
            };
            match removed {
                Ok(()) => {
                    self.index().execute(
                        "DELETE FROM retired_store WHERE kind=?1 AND generation=?2",
                        params![kind, generation],
                    )?;
                    collected += 1;
                }
                Err(error) => {
                    log::warn!(target: "app_core::data", "Retired {kind} {generation} stays: {error}")
                }
            }
        }
        Ok(collected)
    }

    fn remove_legacy(&self) -> std::io::Result<()> {
        for file in LEGACY_FILES {
            // A flat ledger of a format no release wrote stays as it is.
            let path = self.paths.root.join(file);
            if *file == "submissions.db"
                && path.exists()
                && !matches!(file_version(&path), Ok(1 | 2))
            {
                continue;
            }
            for suffix in ["", "-wal", "-shm", "-journal"] {
                match fs::remove_file(self.paths.root.join(format!("{file}{suffix}"))) {
                    Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                        return Err(error)
                    }
                    _ => {}
                }
            }
        }
        let blobs = match fs::read_dir(self.paths.root.join(LEGACY_BLOBS)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in blobs {
            let entry = entry?;
            // Directories are generations of the ledger.
            if entry.file_type()?.is_file() {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}

fn open_index(path: &Path) -> Result<Connection, StoreOpenError> {
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    match version {
        INDEX_FORMAT => Ok(connection),
        0 => {
            let populated: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table')",
                [],
                |row| row.get(0),
            )?;
            if populated {
                return Err(StoreOpenError::UnsupportedFormat {
                    store: StoreKind::Index,
                    found: 0,
                    expected: INDEX_FORMAT,
                });
            }
            let transaction = connection.transaction()?;
            transaction.execute_batch(INDEX_SCHEMA)?;
            transaction.pragma_update(None, "user_version", INDEX_FORMAT)?;
            transaction.commit()?;
            Ok(connection)
        }
        found => Err(StoreOpenError::UnsupportedFormat {
            store: StoreKind::Index,
            found,
            expected: INDEX_FORMAT,
        }),
    }
}

/// `PRAGMA user_version` of a database file, opened read-only.
pub(crate) fn file_version(path: &Path) -> Result<i64, rusqlite::Error> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// Refuses a store of another user format, and empties school tables of another
/// format in one transaction (with no freshness, which would call the lost rows
/// fresh). Returns whether it emptied them.
fn make_school_current(path: &Path) -> Result<bool, StoreOpenError> {
    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let (user, school) = migrations::store_formats(version);
    if user != migrations::USER_SCHEMA_VERSION {
        return Err(StoreOpenError::UnsupportedFormat {
            store: StoreKind::Store,
            found: user,
            expected: migrations::USER_SCHEMA_VERSION,
        });
    }
    if school == migrations::SCHOOL_SCHEMA_VERSION {
        return Ok(false);
    }
    let transaction = connection.transaction()?;
    migrations::recreate_cache(&transaction)?;
    transaction.commit()?;
    log::info!(target: "app_core::data", "School tables of format {school} were emptied");
    Ok(true)
}

/// A candidate's files are synced before the index can name it; a failed
/// build leaves its directory for maintenance and the index as it was.
pub(crate) fn finish_candidate(
    directory: &Path,
    built: Result<(), StoreOpenError>,
) -> Result<(), StoreOpenError> {
    let synced = built.and_then(|()| {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.is_file() {
                crate::storage::sync_file(&path)?;
            }
        }
        crate::storage::sync_directory(directory)?;
        if let Some(parent) = directory.parent() {
            crate::storage::sync_directory(parent)?;
        }
        Ok(())
    });
    if synced.is_err() {
        let _ = fs::remove_dir_all(directory);
    }
    synced
}

#[cfg(test)]
#[path = "../tests/data__root.rs"]
mod tests;
