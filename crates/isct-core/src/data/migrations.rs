//! Schemas of the two database files and the only code that changes them.
//! Migrations run once, when a store opens (`DataStore::initialize_dir`).
//!
//! user.db holds what the person made or chose: the only file backup and
//! cloud sync read. cache.db holds every school and reference cache, the
//! catalog with its search index, freshness and run records; everything in
//! it is fetched again when it is lost, so a cache.db of another schema
//! version is deleted and created again rather than migrated.
use super::connection::remove_database;
use super::DataError;
use rusqlite::{Connection, Transaction};
use std::path::Path;

pub(super) const USER_FILE: &str = "user.db";
pub(super) const CACHE_FILE: &str = "cache.db";
pub const USER_SCHEMA_VERSION: i64 = 1;
pub const CACHE_SCHEMA_VERSION: i64 = 1;

/// The single file of schemas 6–8, before the split.
const LEGACY_FILE: &str = "school.db";
const LEGACY_VERSIONS: std::ops::RangeInclusive<i64> = 6..=8;
/// The legacy file once its user tables are in user.db, kept as a backup.
const LEGACY_BACKUP: &str = "school.db.pre-split";
/// user.db while it is being filled from the legacy file.
const STAGING_FILE: &str = "user.db.migrating";

/// Tables of user.db (`docs/data-ownership.md`): records the person made,
/// settings, and `course_identity_map`, which pins the course ids plans,
/// plan items and overrides refer to; rebuilding it from a later sync that
/// has not seen an offering id yet would give the same course another id.
pub(crate) const USER_TABLES: &[&str] = &[
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

/// Tables of cache.db: school and reference records, シラバス details,
/// search-only catalog rows and the catalog index, freshness and runs.
pub(crate) const CACHE_TABLES: &[&str] = &[
    "timetable_courses",
    "memberships",
    "assignments",
    "offerings",
    "offering_content",
    "catalog_search_items",
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
    "dataset_state",
    "sync_runs",
];

/// FTS5 virtual tables; SQLite keeps their rows in `<name>_*` shadow tables.
pub(super) const FTS_TABLES: &[&str] = &["offerings_fts"];

/// Every table of both files.
pub(super) fn all_tables() -> std::collections::BTreeSet<String> {
    USER_TABLES
        .iter()
        .chain(CACHE_TABLES)
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
/// Freshness per (dataset, key) and one row per read: see `crate::sync_state`.
const CACHE_SCHEMA: &str = r#"
CREATE TABLE cache.timetable_courses (
  id TEXT PRIMARY KEY, term TEXT, offering_id TEXT, names TEXT, title TEXT,
  instructor TEXT, code TEXT, instruction_language TEXT, credits REAL,
  credit_split TEXT, meetings TEXT, color TEXT, url TEXT, source_value TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.memberships (
  id TEXT PRIMARY KEY, term TEXT, course_id TEXT, status TEXT, evidence TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.assignments (
  id TEXT PRIMARY KEY, course_id TEXT, title TEXT, details TEXT, due_at TEXT,
  source_url TEXT, lms_cmid INTEGER, lms_course_id INTEGER, submission TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.offerings (
  id TEXT PRIMARY KEY, year INTEGER, quarters TEXT, code TEXT, class_code TEXT,
  unit_code TEXT, title TEXT, instructors TEXT, instruction_language TEXT,
  format TEXT, media TEXT, class_label TEXT, credits TEXT, schedule TEXT,
  academic_unit TEXT, intensive INTEGER, syllabus_url TEXT, source_updated TEXT,
  fetched_at TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.offering_content (
  offering_id TEXT NOT NULL, lang TEXT NOT NULL, content TEXT NOT NULL,
  fetched_at TEXT NOT NULL, PRIMARY KEY (offering_id, lang)
);
CREATE TABLE cache.catalog_search_items (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE VIRTUAL TABLE cache.offerings_fts USING fts5(
  offering_id UNINDEXED, code, title_ja, title_en, instructors_ja, instructors_en
);
CREATE TABLE cache.lms_courses (
  id TEXT PRIMARY KEY, lms_id INTEGER UNIQUE, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.lms_items (
  id TEXT PRIMARY KEY, cmid INTEGER, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
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
  id TEXT PRIMARY KEY, scope TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.gpa_by_term (
  id TEXT PRIMARY KEY, year INTEGER, term TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.academic_calendar (
  id TEXT PRIMARY KEY, date TEXT, type TEXT, replacement_day INTEGER, label TEXT,
  source_hash TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.departments (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.recommended_courses (
  id TEXT PRIMARY KEY, department_id TEXT, code TEXT, course_group TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.important_dates (
  id TEXT PRIMARY KEY, academic_year INTEGER NOT NULL, start_date TEXT NOT NULL,
  end_date TEXT NOT NULL, category TEXT NOT NULL, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE INDEX cache.important_dates_range ON important_dates(academic_year,start_date,end_date);
CREATE TABLE cache.exams (
  id TEXT PRIMARY KEY, academic_year INTEGER NOT NULL, level TEXT NOT NULL,
  quarter TEXT NOT NULL, course_code TEXT NOT NULL, exam_date TEXT NOT NULL,
  status TEXT NOT NULL, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE INDEX cache.exams_by_course ON exams(academic_year,course_code);
CREATE INDEX cache.exams_by_date ON exams(academic_year,exam_date);
CREATE TABLE cache.class_schedules (
  id TEXT PRIMARY KEY, academic_year INTEGER NOT NULL, course_code TEXT NOT NULL,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE cache.dataset_state (
  dataset TEXT NOT NULL, key TEXT NOT NULL, account TEXT NOT NULL DEFAULT '',
  generation INTEGER NOT NULL DEFAULT 0, fetched_at TEXT, validator TEXT,
  outcome TEXT, message TEXT, state TEXT NOT NULL DEFAULT 'idle', lease_run TEXT,
  failures INTEGER NOT NULL DEFAULT 0, next_eligible TEXT, cursor TEXT,
  last_error TEXT, last_used TEXT,
  PRIMARY KEY (dataset, key)
);
CREATE TABLE cache.sync_runs (
  id INTEGER PRIMARY KEY, run TEXT NOT NULL, dataset TEXT NOT NULL, key TEXT NOT NULL,
  account TEXT NOT NULL DEFAULT '', started_at TEXT NOT NULL, finished_at TEXT NOT NULL,
  outcome TEXT NOT NULL, changed INTEGER NOT NULL DEFAULT 0, requests INTEGER,
  bytes INTEGER, source TEXT, error_kind TEXT
);
CREATE INDEX cache.sync_runs_by_finished ON sync_runs(finished_at);
CREATE INDEX cache.sync_runs_by_key ON sync_runs(dataset, key, finished_at);
"#;

/// Creates the user schema in a new user.db. A file of another version, or
/// one with tables but no version, is refused and left intact.
pub(super) fn migrate_user(connection: &mut Connection) -> Result<bool, DataError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version == USER_SCHEMA_VERSION {
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
    transaction.pragma_update(None, "user_version", USER_SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(true)
}

fn attach_as(connection: &Connection, path: &Path, schema: &str) -> Result<(), DataError> {
    let path = path
        .to_str()
        .ok_or_else(|| DataError::storage("The data folder path is not valid UTF-8"))?;
    connection.execute(&format!("ATTACH DATABASE ?1 AS {schema}"), [path])?;
    Ok(())
}

/// Attaches cache.db as `cache`, the way every connection of a store sees it.
pub(super) fn attach(connection: &Connection, path: &Path) -> Result<(), DataError> {
    attach_as(connection, path, "cache")
}

/// Attaches cache.db to the write connection. A file of another schema
/// version, or one SQLite cannot read, is deleted and created again: its
/// rows are fetched again, and with no freshness rows every key reads as
/// missing. Returns whether the schema was created.
pub(super) fn attach_cache(connection: &mut Connection, path: &Path) -> Result<bool, DataError> {
    let found = attach(connection, path).and_then(|()| {
        connection
            .query_row("PRAGMA cache.user_version", [], |row| row.get::<_, i64>(0))
            .map_err(DataError::from)
    });
    match found {
        Ok(CACHE_SCHEMA_VERSION) => return Ok(false),
        Ok(0) => {}
        Ok(version) => log::info!(
            target: "isct_core::data",
            "cache.db has schema {version}, not {CACHE_SCHEMA_VERSION}; creating it again"
        ),
        Err(error) => log::warn!(
            target: "isct_core::data",
            "cache.db cannot be read ({error}); creating it again"
        ),
    }
    // Only this connection exists while the store opens, so the files can go.
    let _ = connection.execute("DETACH DATABASE cache", []);
    remove_database(path)?;
    attach(connection, path)?;
    connection.execute_batch("PRAGMA cache.journal_mode=WAL;")?;
    let transaction = connection.transaction()?;
    create_cache_schema(&transaction)?;
    transaction.commit()?;
    Ok(true)
}

fn create_cache_schema(connection: &Connection) -> Result<(), DataError> {
    connection.execute_batch(CACHE_SCHEMA)?;
    connection.execute_batch(&format!(
        "PRAGMA cache.user_version={CACHE_SCHEMA_VERSION};"
    ))?;
    Ok(())
}

/// Drops every table of cache.db and creates the schema again, inside the
/// caller's transaction: a reset leaves either the old cache or an empty
/// one. Virtual tables go first; they drop their shadow tables themselves.
pub(super) fn recreate_cache(transaction: &Transaction<'_>) -> Result<(), DataError> {
    let mut statement = transaction.prepare(
        "SELECT name FROM cache.sqlite_master
         WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY sql LIKE 'CREATE VIRTUAL TABLE%' DESC, name",
    )?;
    let tables: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    drop(statement);
    for table in tables {
        transaction.execute_batch(&format!("DROP TABLE IF EXISTS cache.\"{table}\";"))?;
    }
    create_cache_schema(transaction)
}

/// Both schemas in one in-memory connection, for code that writes through a
/// transaction and is tested without a store.
pub fn open_in_memory() -> Result<Connection, DataError> {
    let mut connection = Connection::open_in_memory()?;
    migrate_user(&mut connection)?;
    connection.execute("ATTACH DATABASE ':memory:' AS cache", [])?;
    create_cache_schema(&connection)?;
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
                log::warn!(target: "isct_core::data", "{} was not renamed: {error}", from.display());
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
    log::info!(target: "isct_core::data", "Moved the user tables of school.db into user.db");
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
    for table in USER_TABLES {
        let columns = shared_columns(&transaction, table)?;
        transaction.execute(
            &format!("INSERT INTO main.{table} ({columns}) SELECT {columns} FROM legacy.{table}"),
            [],
        )?;
    }
    transaction.pragma_update(None, "user_version", USER_SCHEMA_VERSION)?;
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
