//! Schemas of the three database files. A store generation is created at
//! the current schemas (`data::root`); nothing is migrated in place.
//!
//! user.db holds what the person made or chose: the only file backup and
//! cloud sync read. cache.db holds every school and reference record and the
//! catalog with its search index. sync.db holds the sync engine's
//! bookkeeping about those records: freshness and run records. A sync that
//! finds nothing new commits to sync.db only, so cache.db's data version,
//! which the view cache is keyed on, moves only when records change.
//! Everything in cache.db and sync.db is fetched again when it is lost, so
//! a school group of another schema is not read: a new, empty generation
//! replaces it (`approach/short-lived-store-rebuild`).
use super::DataError;
use rusqlite::{Connection, Transaction};
use std::path::Path;

pub(super) const USER_FILE: &str = "user.db";
pub(super) const CACHE_FILE: &str = "cache.db";
pub(super) const SYNC_FILE: &str = "sync.db";
pub const USER_SCHEMA_VERSION: i64 = 3;
pub const CACHE_SCHEMA_VERSION: i64 = 5;
pub const SYNC_SCHEMA_VERSION: i64 = 2;

/// The school group's format as `store-index.db` records it.
pub(super) const fn school_format() -> i64 {
    CACHE_SCHEMA_VERSION * 100 + SYNC_SCHEMA_VERSION
}

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
    "subscriptions",
    "schedule_overrides",
    "scheduled_tasks",
];

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
CREATE TABLE course_keys (
  key TEXT PRIMARY KEY, course_id TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX course_keys_by_course ON course_keys(course_id);
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
  dataset TEXT NOT NULL, key TEXT NOT NULL,
  fetched_at TEXT, validator TEXT, outcome TEXT, message TEXT,
  failures INTEGER NOT NULL DEFAULT 0, next_eligible TEXT, cursor TEXT,
  last_error TEXT, last_used TEXT,
  PRIMARY KEY (dataset, key)
);
CREATE TABLE sync.sync_runs (
  id INTEGER PRIMARY KEY, run TEXT NOT NULL, dataset TEXT NOT NULL, key TEXT NOT NULL,
  started_at TEXT NOT NULL, finished_at TEXT NOT NULL,
  outcome TEXT NOT NULL, changed INTEGER NOT NULL DEFAULT 0, requests INTEGER,
  bytes INTEGER, source TEXT, error_kind TEXT
);
CREATE INDEX sync.sync_runs_by_finished ON sync_runs(finished_at);
CREATE INDEX sync.sync_runs_by_key ON sync_runs(dataset, key, finished_at);
"#;


/// Creates user.db at the current schema, in a new generation's directory.
pub(super) fn create_user(path: &Path) -> Result<(), DataError> {
    let mut connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA journal_mode=DELETE;")?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(USER_SCHEMA)?;
    transaction.pragma_update(None, "user_version", USER_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

/// Creates cache.db and sync.db at the current schemas, in a new
/// generation's directory.
pub(super) fn create_school(cache: &Path, sync: &Path) -> Result<(), DataError> {
    let mut connection = Connection::open_in_memory()?;
    attach(&connection, cache, sync)?;
    connection.execute_batch("PRAGMA cache.journal_mode=DELETE; PRAGMA sync.journal_mode=DELETE;")?;
    let transaction = connection.transaction()?;
    create_cache_schema(&transaction)?;
    create_sync_schema(&transaction)?;
    transaction.commit()?;
    Ok(())
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
    connection.execute_batch(USER_SCHEMA)?;
    connection.pragma_update(None, "user_version", USER_SCHEMA_VERSION)?;
    connection.execute("ATTACH DATABASE ':memory:' AS cache", [])?;
    connection.execute("ATTACH DATABASE ':memory:' AS sync", [])?;
    create_cache_schema(&connection)?;
    create_sync_schema(&connection)?;
    super::connection::forbid_truncate(&mut connection)?;
    Ok(connection)
}

#[cfg(test)]
#[path = "../tests/data__migrations.rs"]
mod tests;
