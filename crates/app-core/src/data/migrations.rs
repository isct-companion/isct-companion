//! The schema of the store's one database file. A store generation is created
//! at the current schema (`data::root`); the user tables are not migrated in
//! place.
//!
//! Three groups of tables share the file, so a transaction that writes several
//! is one commit and a read sees one state of all of them. The user tables hold
//! what the person made or chose: the only tables backup and cloud sync read.
//! The cache tables hold every school and reference record and the catalog with
//! its search index. The sync tables hold the sync engine's bookkeeping about
//! those records: freshness and run records. Everything in the last two is
//! fetched again when it is lost, so tables of another school format are not
//! read: they are dropped and created empty
//! (`approach/short-lived-store-rebuild`).
use super::DataError;
use rusqlite::{Connection, Transaction};
use std::path::Path;

pub(super) const STORE_FILE: &str = "store.db";
pub const USER_SCHEMA_VERSION: i64 = 3;
pub const SCHOOL_SCHEMA_VERSION: i64 = 1;
pub(super) const STORE_VERSION: i64 = store_version(USER_SCHEMA_VERSION, SCHOOL_SCHEMA_VERSION);

/// `PRAGMA user_version` of a store: its user and school formats, which change
/// apart and are dealt with apart, in one number that commits with the tables.
pub(super) const fn store_version(user: i64, school: i64) -> i64 {
    user * 1000 + school
}

/// The user and school formats `store_version` holds.
pub(super) const fn store_formats(version: i64) -> (i64, i64) {
    (version / 1000, version % 1000)
}

/// The user tables (`docs/data-ownership.md`): records the person made,
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

/// The cache tables: school and reference records, the catalog and its
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

/// Cache tables derived from the others (`data::course_merge`): a reset
/// does not count them as removed records.
pub(crate) const DERIVED_TABLES: &[&str] = &["merged_rows", "merge_state"];

/// The sync tables: freshness per key and one row per read.
pub(crate) const SYNC_TABLES: &[&str] = &["dataset_state", "sync_runs"];

/// FTS5 virtual tables; SQLite keeps their rows in `<name>_*` shadow tables.
pub(super) const FTS_TABLES: &[&str] = &["offerings_fts"];

/// Every table of the store.
pub(super) fn all_tables() -> std::collections::BTreeSet<String> {
    USER_TABLES
        .iter()
        .chain(CACHE_TABLES)
        .chain(SYNC_TABLES)
        .map(|table| (*table).to_string())
        .collect()
}

pub(super) const USER_SCHEMA: &str = r#"
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

/// Record rows keep their content in `value` only; a field a query filters
/// on is a VIRTUAL generated column over it, indexed, so writers never copy
/// fields into columns and a column cannot drift from its row.
const CACHE_SCHEMA: &str = r#"
CREATE TABLE timetable_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE memberships (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE assignments (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  lms_cmid INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsCmid')) VIRTUAL,
  lms_course_id INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsCourseId')) VIRTUAL,
  submission TEXT GENERATED ALWAYS AS (json_extract(value,'$.submission')) VIRTUAL
);
CREATE INDEX assignments_by_cmid ON assignments(lms_cmid);
CREATE INDEX assignments_by_lms_course ON assignments(lms_course_id);
CREATE TABLE offerings (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  year INTEGER GENERATED ALWAYS AS (json_extract(value,'$.year')) VIRTUAL
);
CREATE INDEX offerings_by_year ON offerings(year);
CREATE VIRTUAL TABLE offerings_fts USING fts5(
  offering_id UNINDEXED, code, title_ja, title_en, instructors_ja, instructors_en
);
CREATE TABLE lms_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  lms_id INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsId')) VIRTUAL
);
CREATE INDEX lms_courses_by_lms_id ON lms_courses(lms_id);
CREATE TABLE lms_items (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT,
  cmid INTEGER GENERATED ALWAYS AS (json_extract(value,'$.cmid')) VIRTUAL,
  lms_course_id INTEGER GENERATED ALWAYS AS (json_extract(value,'$.lmsCourseId')) VIRTUAL,
  url TEXT GENERATED ALWAYS AS (json_extract(value,'$.url')) VIRTUAL,
  resource_url TEXT GENERATED ALWAYS AS (json_extract(value,'$.resourceUrl')) VIRTUAL
);
CREATE INDEX lms_items_by_cmid ON lms_items(cmid);
CREATE INDEX lms_items_by_lms_course ON lms_items(lms_course_id);
CREATE INDEX lms_items_by_url ON lms_items(url);
CREATE INDEX lms_items_by_resource_url ON lms_items(resource_url);
CREATE TABLE registrations (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE registration_states (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE grades (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE requirement_progress (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE gpa_summary (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE gpa_by_term (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE academic_calendar (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE departments (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE recommended_courses (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE important_dates (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE exams (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE class_schedules (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
"#;

/// Courses merged when their sources are written (`data::course_merge`),
/// and whether every course has been merged once.
const MERGED_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS merged_rows (
  kind TEXT NOT NULL, ord TEXT NOT NULL, course_id TEXT NOT NULL, value TEXT NOT NULL,
  PRIMARY KEY (kind, ord)
);
CREATE INDEX IF NOT EXISTS merged_rows_by_course ON merged_rows(course_id);
CREATE TABLE IF NOT EXISTS merge_state (built INTEGER NOT NULL);
"#;

/// Created through the connection that has db attached as `sync`.
/// Freshness per (dataset, key) and one row per read: see `crate::sync_state`.
const SYNC_SCHEMA: &str = r#"
CREATE TABLE dataset_state (
  dataset TEXT NOT NULL, key TEXT NOT NULL,
  fetched_at TEXT, validator TEXT, outcome TEXT, message TEXT,
  failures INTEGER NOT NULL DEFAULT 0, next_eligible TEXT, cursor TEXT,
  last_error TEXT, last_used TEXT,
  PRIMARY KEY (dataset, key)
);
CREATE TABLE sync_runs (
  id INTEGER PRIMARY KEY, run TEXT NOT NULL, dataset TEXT NOT NULL, key TEXT NOT NULL,
  started_at TEXT NOT NULL, finished_at TEXT NOT NULL,
  outcome TEXT NOT NULL, changed INTEGER NOT NULL DEFAULT 0, requests INTEGER,
  bytes INTEGER, source TEXT, error_kind TEXT
);
CREATE INDEX sync_runs_by_finished ON sync_runs(finished_at);
CREATE INDEX sync_runs_by_key ON sync_runs(dataset, key, finished_at);
"#;

/// Creates a store at the current schema, in a new generation's directory, with
/// the user tables of `user_from`, a user.db of the current user format, if
/// given. It is checked before the index can name it.
pub(super) fn create(path: &Path, user_from: Option<&Path>) -> Result<(), DataError> {
    let mut connection = Connection::open(path)?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(USER_SCHEMA)?;
    create_school(&transaction)?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    transaction.commit()?;
    if let Some(source) = user_from {
        let source = source
            .to_str()
            .ok_or_else(|| DataError::storage("The data folder path is not valid UTF-8"))?;
        connection.execute("ATTACH DATABASE ?1 AS source", [source])?;
        let transaction = connection.transaction()?;
        for table in USER_TABLES {
            transaction.execute_batch(&format!(
                "INSERT INTO main.{table} SELECT * FROM source.{table};"
            ))?;
        }
        transaction.commit()?;
        connection.execute_batch("DETACH DATABASE source;")?;
    }
    let check: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(DataError::storage(format!(
            "The new store failed its check: {check}"
        )));
    }
    Ok(())
}

fn create_school(connection: &Connection) -> Result<(), DataError> {
    connection.execute_batch(CACHE_SCHEMA)?;
    connection.execute_batch(MERGED_SCHEMA)?;
    connection.execute_batch(SYNC_SCHEMA)?;
    Ok(())
}

/// Drops every table that is not a user table, of whatever school format, and
/// creates the current ones, inside the caller's transaction: a reset or a
/// format change leaves either the old caches or empty ones. Virtual tables go
/// first; they drop their shadow tables themselves.
pub(super) fn recreate_cache(transaction: &Transaction<'_>) -> Result<(), DataError> {
    let mut statement = transaction.prepare(
        "SELECT name FROM sqlite_master
         WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY sql LIKE 'CREATE VIRTUAL TABLE%' DESC, name",
    )?;
    let tables: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(statement);
    for table in tables
        .iter()
        .filter(|table| !USER_TABLES.contains(&table.as_str()))
    {
        transaction.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\";"))?;
    }
    create_school(transaction)?;
    transaction.pragma_update(None, "user_version", STORE_VERSION)?;
    Ok(())
}

/// The schema in one in-memory connection, for code that writes through a
/// transaction and is tested without a store.
pub fn open_in_memory() -> Result<Connection, DataError> {
    let mut connection = Connection::open_in_memory()?;
    connection.execute_batch(USER_SCHEMA)?;
    create_school(&connection)?;
    connection.pragma_update(None, "user_version", STORE_VERSION)?;
    super::connection::forbid_truncate(&mut connection)?;
    Ok(connection)
}

#[cfg(test)]
#[path = "../tests/data__migrations.rs"]
mod tests;
