//! Schemas of the three database files and the only code that changes them.
//! Migrations run once, when a store opens (`DataStore::initialize_dir`).
//!
//! user.db holds what the person made or chose: the only file backup and
//! cloud sync read. cache.db holds every school and reference record and the
//! catalog with its search index. sync.db holds the sync engine's
//! bookkeeping about those records: freshness and run records. A sync that
//! finds nothing new commits to sync.db only, so cache.db's data version,
//! which the view cache is keyed on, moves only when records change.
//! Everything in cache.db and sync.db is fetched again when it is lost, so
//! a file of another schema version is deleted and created again rather
//! than migrated; freshness without its records would say they are fresh,
//! so sync.db is emptied whenever cache.db is.
use super::connection::remove_database;
use super::DataError;
use rusqlite::{Connection, Transaction};
use std::path::Path;

pub(super) const USER_FILE: &str = "user.db";
pub(super) const CACHE_FILE: &str = "cache.db";
pub(super) const SYNC_FILE: &str = "sync.db";
pub const USER_SCHEMA_VERSION: i64 = 2;
/// user.db with `course_identity_map`, before `course_keys`; new and legacy
/// files are created at it and upgraded at once.
const USER_IDENTITY_MAP_VERSION: i64 = 1;
pub const CACHE_SCHEMA_VERSION: i64 = 5;
/// cache.db before merged courses: upgraded in place (`MERGED_SCHEMA`).
const CACHE_WITHOUT_MERGE_VERSION: i64 = 4;
pub const SYNC_SCHEMA_VERSION: i64 = 1;
/// cache.db with the sync bookkeeping still inside: moved to sync.db once.
const CACHE_WITH_SYNC_VERSION: i64 = 3;

/// The single file of schemas 6–8, before the split.
const LEGACY_FILE: &str = "school.db";
const LEGACY_VERSIONS: std::ops::RangeInclusive<i64> = 6..=8;
/// The legacy file once its user tables are in user.db, kept as a backup.
const LEGACY_BACKUP: &str = "school.db.pre-split";
/// user.db while it is being filled from the legacy file.
const STAGING_FILE: &str = "user.db.migrating";

/// Tables of user.db (`docs/data-ownership.md`): records the person made,
/// settings, and `course_keys`, which pins the internal course ids plans,
/// plan items and overrides refer to; rebuilding it from a later sync that
/// has not seen an offering id yet would give the same course another id.
pub(crate) const USER_TABLES: &[&str] = &[
    "manual_courses",
    "course_keys",
    "manual_memberships",
    "course_overrides",
    "plan_items",
    "personal_events",
    "calendar_exceptions",
    "assignment_marks",
    "settings",
    "intensive_schedules",
    "plans",
    "subscriptions",
    "schedule_overrides",
    "scheduled_tasks",
];

/// The user tables of schema 1, which a legacy file is copied into.
const LEGACY_USER_TABLES: &[&str] = &[
    "manual_courses",
    "course_identity_map",
    "manual_memberships",
    "course_overrides",
    "plan_items",
    "personal_events",
    "calendar_exceptions",
    "assignment_marks",
    "settings",
    "intensive_schedules",
    "plans",
    "subscriptions",
    "schedule_overrides",
    "scheduled_tasks",
];

/// user.db 1 → 2: one internal id per course with its external keys
/// (`data::course_keys`).
const COURSE_KEYS_SCHEMA: &str = r#"
CREATE TABLE course_keys (
  key TEXT PRIMARY KEY, course_id TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX course_keys_by_course ON course_keys(course_id);
"#;

/// Tables of cache.db: school and reference records, the catalog and its
/// index.
pub(crate) const CACHE_TABLES: &[&str] = &[
    "timetable_courses",
    "memberships",
    "assignments",
    "offerings",
    "offerings_fts",
    "lms_courses",
    "lms_items",
    "registrations",
    "registration_states",
    "grades",
    "requirement_progress",
    "gpa_summary",
    "gpa_by_term",
    "academic_calendar",
    "departments",
    "recommended_courses",
    "important_dates",
    "exams",
    "class_schedules",
    "merged_rows",
    "merge_state",
];

/// Tables of cache.db derived from the others (`data::course_merge`): a reset
/// does not count them as removed records.
pub(crate) const DERIVED_TABLES: &[&str] = &["merged_rows", "merge_state"];

/// Tables of sync.db: freshness per key and one row per read.
pub(crate) const SYNC_TABLES: &[&str] = &["dataset_state", "sync_runs"];

/// FTS5 virtual tables; SQLite keeps their rows in `<name>_*` shadow tables.
pub(super) const FTS_TABLES: &[&str] = &["offerings_fts"];

/// Every table of the three files.
pub(super) fn all_tables() -> std::collections::BTreeSet<String> {
    USER_TABLES
        .iter()
        .chain(CACHE_TABLES)
        .chain(SYNC_TABLES)
        .map(|table| (*table).to_string())
        .collect()
}

const USER_SCHEMA: &str = r#"
CREATE TABLE manual_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE course_identity_map (
  provider TEXT NOT NULL, external_id TEXT NOT NULL, academic_year INTEGER NOT NULL,
  course_id TEXT NOT NULL, offering_id TEXT, updated_at TEXT NOT NULL,
  PRIMARY KEY (provider, external_id, academic_year)
);
CREATE INDEX course_identity_by_course ON course_identity_map(course_id);
CREATE INDEX course_identity_by_offering
  ON course_identity_map(academic_year, offering_id)
  WHERE offering_id IS NOT NULL;
CREATE TABLE manual_memberships (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE course_overrides (
  id TEXT PRIMARY KEY, source_key TEXT UNIQUE, course_id TEXT, title TEXT, instructor TEXT, color TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT
);
CREATE TABLE plan_items (
  id TEXT PRIMARY KEY, term TEXT, offering_id TEXT, course_id TEXT, state TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT
);
CREATE TABLE personal_events (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE calendar_exceptions (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE assignment_marks (
  id TEXT PRIMARY KEY, source_key TEXT UNIQUE, assignment_id TEXT, hidden INTEGER NOT NULL DEFAULT 0,
  completed_at TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE settings (
  key TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE intensive_schedules (
  id TEXT PRIMARY KEY, course_id TEXT UNIQUE, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE plans (
  id TEXT PRIMARY KEY, title TEXT, course_id TEXT, room TEXT, start TEXT,
  duration INTEGER, rrule TEXT, exdates TEXT, reminder TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT
);
CREATE TABLE subscriptions (
  id TEXT PRIMARY KEY, url TEXT, kind TEXT, enabled INTEGER, subscription_hash TEXT,
  fetched_at TEXT, last_error TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE schedule_overrides (
  id TEXT PRIMARY KEY, target_key TEXT UNIQUE, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE scheduled_tasks (
  id TEXT PRIMARY KEY, name TEXT, action TEXT, hook_id TEXT, schedule TEXT,
  enabled INTEGER, last_run_at TEXT, last_result TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT
);
"#;

/// Created through the connection that has cache.db attached as `cache`.
/// Record rows keep their content in `value` only; a field a query filters
/// on is a VIRTUAL generated column over it, indexed, so writers never copy
/// fields into columns and a column cannot drift from its row.
const CACHE_SCHEMA: &str = r#"
CREATE TABLE cache.timetable_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.memberships (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.assignments (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  lms_cmid INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsCmid')) VIRTUAL,
  lms_course_id INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsCourseId')) VIRTUAL,
  submission TEXT GENERATED ALWAYS AS (json_extract(value,'$.submission')) VIRTUAL
);
CREATE INDEX cache.assignments_by_cmid ON assignments(lms_cmid);
CREATE INDEX cache.assignments_by_lms_course ON assignments(lms_course_id);
CREATE TABLE cache.offerings (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  year INTEGER GENERATED ALWAYS AS (json_extract(value,'$.year')) VIRTUAL
);
CREATE INDEX cache.offerings_by_year ON offerings(year);
CREATE VIRTUAL TABLE cache.offerings_fts USING fts5(
  offering_id UNINDEXED, code, title_ja, title_en, instructors_ja, instructors_en
);
CREATE TABLE cache.lms_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  lms_id INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsId')) VIRTUAL
);
CREATE INDEX cache.lms_courses_by_lms_id ON lms_courses(lms_id);
CREATE TABLE cache.lms_items (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  cmid INTEGER GENERATED ALWAYS AS (json_extract(value,'$.cmid')) VIRTUAL,
  lms_course_id INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsCourseId')) VIRTUAL,
  url TEXT GENERATED ALWAYS AS (json_extract(value,'$.url')) VIRTUAL,
  resource_url TEXT GENERATED ALWAYS AS (json_extract(value,'$.resourceUrl')) VIRTUAL
);
CREATE INDEX cache.lms_items_by_cmid ON lms_items(cmid);
CREATE INDEX cache.lms_items_by_lms_course ON lms_items(lms_course_id);
CREATE INDEX cache.lms_items_by_url ON lms_items(url);
CREATE INDEX cache.lms_items_by_resource_url ON lms_items(resource_url);
CREATE TABLE cache.registrations (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.registration_states (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.grades (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.requirement_progress (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.gpa_summary (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.gpa_by_term (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.academic_calendar (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.departments (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.recommended_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.important_dates (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.exams (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.class_schedules (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
"#;

/// Courses merged when their sources are written (`data::course_merge`),
/// and whether every course has been merged once.
const MERGED_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS cache.merged_rows (
  kind TEXT NOT NULL, ord TEXT NOT NULL, course_id TEXT NOT NULL, value TEXT NOT NULL,
  PRIMARY KEY (kind, ord)
);
CREATE INDEX IF NOT EXISTS cache.merged_rows_by_course ON merged_rows(course_id);
CREATE TABLE IF NOT EXISTS cache.merge_state (built INTEGER NOT NULL);
"#;

/// Created through the connection that has sync.db attached as `sync`.
/// Freshness per (dataset, key) and one row per read: see `crate::sync_state`.
const SYNC_SCHEMA: &str = r#"
CREATE TABLE sync.dataset_state (
  dataset TEXT NOT NULL, key TEXT NOT NULL, account TEXT NOT NULL DEFAULT '',
  fetched_at TEXT, validator TEXT, outcome TEXT, message TEXT,
  failures INTEGER NOT NULL DEFAULT 0, next_eligible TEXT, cursor TEXT,
  last_error TEXT, last_used TEXT,
  PRIMARY KEY (dataset, key)
);
CREATE TABLE sync.sync_runs (
  id INTEGER PRIMARY KEY, run TEXT NOT NULL, dataset TEXT NOT NULL, key TEXT NOT NULL,
  account TEXT NOT NULL DEFAULT '', started_at TEXT NOT NULL, finished_at TEXT NOT NULL,
  outcome TEXT NOT NULL, changed INTEGER NOT NULL DEFAULT 0, requests INTEGER,
  bytes INTEGER, source TEXT, error_kind TEXT
);
CREATE INDEX sync.sync_runs_by_finished ON sync_runs(finished_at);
CREATE INDEX sync.sync_runs_by_key ON sync_runs(dataset, key, finished_at);
"#;

/// Creates the user schema in a new user.db, at schema 1; `upgrade_user`
/// brings it to the current one once the caches are attached. A file of
/// another version, or one with tables but no version, is refused and left
/// intact.
pub(super) fn migrate_user(connection: &mut Connection) -> Result<bool, DataError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if (USER_IDENTITY_MAP_VERSION..=USER_SCHEMA_VERSION).contains(&version) {
        return Ok(false);
    }
    let populated: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
        [],
        |row| row.get(0),
    )?;
    if version != 0 || populated {
        return Err(DataError::compatibility(format!(
            "user.db schema {version} is not supported. Your data was left intact."
        )));
    }
    let transaction = connection.transaction()?;
    transaction.execute_batch(USER_SCHEMA)?;
    transaction.pragma_update(None, "user_version", USER_IDENTITY_MAP_VERSION)?;
    transaction.commit()?;
    Ok(true)
}

/// Upgrades user.db in place, in one transaction with cache.db attached
/// (moving a course id rewrites the school rows that name it); on failure
/// the file stays as it was. Returns whether it changed.
pub(super) fn upgrade_user(connection: &mut Connection) -> Result<bool, DataError> {
    let version: i64 = connection.query_row("PRAGMA main.user_version", [], |row| row.get(0))?;
    if version == USER_SCHEMA_VERSION {
        return Ok(false);
    }
    let transaction = connection.transaction()?;
    if version == USER_IDENTITY_MAP_VERSION {
        transaction.execute_batch(COURSE_KEYS_SCHEMA)?;
        super::course_keys::migrate_identity_map(&transaction)?;
        super::course_keys::key_manual_courses(&transaction)?;
        transaction.execute_batch("DROP TABLE course_identity_map;")?;
    }
    transaction.pragma_update(Some("main"), "user_version", USER_SCHEMA_VERSION)?;
    transaction.commit()?;
    log::info!(target: "app_core::data", "Upgraded user.db from schema {version}");
    Ok(true)
}

fn attach_as(connection: &Connection, path: &Path, schema: &str) -> Result<(), DataError> {
    let path = path
        .to_str()
        .ok_or_else(|| DataError::storage("The data folder path is not valid UTF-8"))?;
    connection.execute(&format!("ATTACH DATABASE ?1 AS {schema}"), [path])?;
    Ok(())
}

/// Attaches cache.db as `cache` and sync.db as `sync`, the way every
/// connection of a store sees them.
pub(super) fn attach(connection: &Connection, cache: &Path, sync: &Path) -> Result<(), DataError> {
    attach_as(connection, cache, "cache")?;
    attach_as(connection, sync, "sync")
}

/// Attaches cache.db and sync.db to the write connection. A file of another
/// schema version, or one SQLite cannot read, is deleted and created again:
/// its rows are fetched again, and with no freshness rows every key reads as
/// missing. A cache.db of schema 3 moves its bookkeeping into sync.db
/// instead. Returns whether a schema was created.
pub(super) fn attach_caches(
    connection: &mut Connection,
    cache: &Path,
    sync: &Path,
) -> Result<bool, DataError> {
    let sync_created = attach_checked(connection, sync, "sync", SYNC_SCHEMA_VERSION)?;
    let cache_version = attach_checked(connection, cache, "cache", CACHE_SCHEMA_VERSION)?;
    let transaction = connection.transaction()?;
    if sync_created == Attached::Created && cache_version != Attached::Created {
        create_sync_schema(&transaction)?;
    }
    let created = match cache_version {
        Attached::Current => sync_created == Attached::Created,
        Attached::Created => {
            create_cache_schema(&transaction)?;
            // Freshness of the records that were lost would keep them from
            // being fetched again.
            recreate_sync(&transaction)?;
            true
        }
        Attached::WithoutMerge => {
            transaction.execute_batch(MERGED_SCHEMA)?;
            transaction.execute_batch(&format!(
                "PRAGMA cache.user_version={CACHE_SCHEMA_VERSION};"
            ))?;
            log::info!(target: "app_core::data", "Added merged courses to cache.db");
            sync_created == Attached::Created
        }
        Attached::Old => {
            if sync_created == Attached::Created {
                // `SELECT *` matches: the bookkeeping kept its columns.
                for table in SYNC_TABLES {
                    transaction.execute_batch(&format!(
                        "INSERT INTO sync.{table} SELECT * FROM cache.{table};"
                    ))?;
                }
            }
            for table in SYNC_TABLES {
                transaction.execute_batch(&format!("DROP TABLE cache.{table};"))?;
            }
            transaction.execute_batch(MERGED_SCHEMA)?;
            transaction.execute_batch(&format!(
                "PRAGMA cache.user_version={CACHE_SCHEMA_VERSION};"
            ))?;
            log::info!(target: "app_core::data", "Moved the sync bookkeeping of cache.db into sync.db");
            sync_created == Attached::Created
        }
    };
    transaction.commit()?;
    Ok(created)
}

#[derive(Clone, Copy, PartialEq)]
enum Attached {
    Current,
    /// cache.db of schema 3: its bookkeeping moves to sync.db.
    Old,
    /// cache.db of schema 4: its school rows stay and merged courses are
    /// added; they are merged on the store's first write.
    WithoutMerge,
    /// A new empty file, without its schema yet.
    Created,
}

/// Attaches `path` as `schema`; a file of another version (but cache.db of
/// schema 3) or one SQLite cannot read is replaced by an empty one.
fn attach_checked(
    connection: &mut Connection,
    path: &Path,
    schema: &str,
    current: i64,
) -> Result<Attached, DataError> {
    let found = attach_as(connection, path, schema).and_then(|()| {
        connection
            .query_row(&format!("PRAGMA {schema}.user_version"), [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(DataError::from)
    });
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(schema);
    match found {
        Ok(version) if version == current => return Ok(Attached::Current),
        Ok(CACHE_WITH_SYNC_VERSION) if schema == "cache" => return Ok(Attached::Old),
        Ok(CACHE_WITHOUT_MERGE_VERSION) if schema == "cache" => return Ok(Attached::WithoutMerge),
        Ok(0) => {}
        Ok(version) => log::info!(
            target: "app_core::data",
            "{file} has schema {version}, not {current}; creating it again"
        ),
        Err(error) => log::warn!(
            target: "app_core::data",
            "{file} cannot be read ({error}); creating it again"
        ),
    }
    // Only this connection exists while the store opens, so the files can go.
    let _ = connection.execute(&format!("DETACH DATABASE {schema}"), []);
    remove_database(path)?;
    attach_as(connection, path, schema)?;
    connection.execute_batch(&format!("PRAGMA {schema}.journal_mode=WAL;"))?;
    Ok(Attached::Created)
}

fn create_cache_schema(connection: &Connection) -> Result<(), DataError> {
    connection.execute_batch(CACHE_SCHEMA)?;
    connection.execute_batch(MERGED_SCHEMA)?;
    connection.execute_batch(&format!(
        "PRAGMA cache.user_version={CACHE_SCHEMA_VERSION};"
    ))?;
    Ok(())
}

fn create_sync_schema(connection: &Connection) -> Result<(), DataError> {
    connection.execute_batch(SYNC_SCHEMA)?;
    connection.execute_batch(&format!("PRAGMA sync.user_version={SYNC_SCHEMA_VERSION};"))?;
    Ok(())
}

/// Drops the tables of `schema`, virtual tables first (they drop their
/// shadow tables themselves).
fn drop_tables(transaction: &Transaction<'_>, schema: &str) -> Result<(), DataError> {
    let mut statement = transaction.prepare(&format!(
        "SELECT name FROM {schema}.sqlite_master
         WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY sql LIKE 'CREATE VIRTUAL TABLE%' DESC, name"
    ))?;
    let tables: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(statement);
    for table in tables {
        transaction.execute_batch(&format!("DROP TABLE IF EXISTS {schema}.\"{table}\";"))?;
    }
    Ok(())
}

fn recreate_sync(transaction: &Transaction<'_>) -> Result<(), DataError> {
    drop_tables(transaction, "sync")?;
    create_sync_schema(transaction)
}

/// Drops every table of cache.db and sync.db and creates the schemas again,
/// inside the caller's transaction: a reset leaves either the old caches or
/// empty ones.
pub(super) fn recreate_cache(transaction: &Transaction<'_>) -> Result<(), DataError> {
    drop_tables(transaction, "cache")?;
    create_cache_schema(transaction)?;
    recreate_sync(transaction)
}

/// The three schemas in one in-memory connection, for code that writes
/// through a transaction and is tested without a store.
pub fn open_in_memory() -> Result<Connection, DataError> {
    let mut connection = Connection::open_in_memory()?;
    migrate_user(&mut connection)?;
    connection.execute("ATTACH DATABASE ':memory:' AS cache", [])?;
    connection.execute("ATTACH DATABASE ':memory:' AS sync", [])?;
    create_cache_schema(&connection)?;
    create_sync_schema(&connection)?;
    upgrade_user(&mut connection)?;
    super::connection::forbid_truncate(&connection);
    Ok(connection)
}

/// Splits a `school.db` of schema 6–8 in `directory`, once: every user table
/// is copied into a new user.db in one transaction, school caches stay
/// behind (they are fetched again: every key reads as missing), and the
/// legacy file is renamed `school.db.pre-split` as a backup. On failure
/// nothing is renamed, no user.db exists, and the error says so.
pub(super) fn migrate_legacy(directory: &Path) -> Result<bool, DataError> {
    let legacy = directory.join(LEGACY_FILE);
    let user = directory.join(USER_FILE);
    if user.exists() || !legacy.exists() {
        return Ok(false);
    }
    let staging = directory.join(STAGING_FILE);
    let copied = remove_database(&staging).and_then(|()| copy_user_tables(&legacy, &staging));
    if let Err(error) = copied {
        let _ = remove_database(&staging);
        return Err(match error {
            DataError::Compatibility(message) => DataError::Compatibility(message),
            error => DataError::storage(format!(
                "school.db could not be moved into user.db and was left as it was: {error}"
            )),
        });
    }
    std::fs::rename(&staging, &user).map_err(|error| {
        DataError::storage(format!(
            "school.db could not be moved into user.db and was left as it was: {error}"
        ))
    })?;
    // user.db now holds the person's data; the legacy file only backs it up,
    // so a failed rename is reported but does not stop the app.
    let rename = |from: &str, to: &str| {
        let from = directory.join(from);
        if from.exists() {
            if let Err(error) = std::fs::rename(&from, directory.join(to)) {
                log::warn!(target: "app_core::data", "{} was not renamed: {error}", from.display());
            }
        }
    };
    for suffix in ["", "-wal", "-shm"] {
        rename(
            &format!("{LEGACY_FILE}{suffix}"),
            &format!("{LEGACY_BACKUP}{suffix}"),
        );
    }
    // The reminder log is named after the database file
    // (`schedule::reminders::fired_path`); keep it, so nothing fires twice.
    rename(
        &format!("{LEGACY_FILE}.reminders-fired.json"),
        &format!("{USER_FILE}.reminders-fired.json"),
    );
    log::info!(target: "app_core::data", "Moved the user tables of school.db into user.db");
    Ok(true)
}

fn copy_user_tables(legacy: &Path, staging: &Path) -> Result<(), DataError> {
    let mut connection = Connection::open(staging)?;
    attach_as(&connection, legacy, "legacy")?;
    let version: i64 = connection.query_row("PRAGMA legacy.user_version", [], |row| row.get(0))?;
    if !LEGACY_VERSIONS.contains(&version) {
        return Err(DataError::compatibility(format!(
            "school.db has schema {version}, which this app cannot read. It was left as it was."
        )));
    }
    let transaction = connection.transaction()?;
    transaction.execute_batch(USER_SCHEMA)?;
    for table in LEGACY_USER_TABLES {
        let columns = shared_columns(&transaction, table)?;
        transaction.execute(
            &format!("INSERT INTO main.{table} ({columns}) SELECT {columns} FROM legacy.{table}"),
            [],
        )?;
    }
    transaction.pragma_update(None, "user_version", USER_IDENTITY_MAP_VERSION)?;
    transaction.commit()?;
    connection.execute("DETACH DATABASE legacy", [])?;
    Ok(())
}

fn columns(connection: &Connection, schema: &str, table: &str) -> Result<Vec<String>, DataError> {
    let mut statement = connection.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
    let names: Vec<String> = statement
        .query_map([], |row| row.get(1))?
        .collect::<Result<_, _>>()?;
    Ok(names)
}

/// The columns of a user table both files have, quoted for SQL.
fn shared_columns(connection: &Connection, table: &str) -> Result<String, DataError> {
    let legacy = columns(connection, "legacy", table)?;
    if legacy.is_empty() {
        return Err(DataError::storage(format!(
            "school.db has no {table} table"
        )));
    }
    Ok(columns(connection, "main", table)?
        .into_iter()
        .filter(|column| legacy.contains(column))
        .map(|column| format!("\"{column}\""))
        .collect::<Vec<_>>()
        .join(","))
}

#[cfg(test)]
#[path = "../tests/data__migrations.rs"]
mod tests;
