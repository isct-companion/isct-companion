use super::DataError;
use rusqlite::Connection;

pub const SCHEMA_VERSION: i64 = 7;

const CREATE_SCHEMA: &str = r#"
CREATE TABLE timetable_courses (
  id TEXT PRIMARY KEY, term TEXT, offering_id TEXT, names TEXT, title TEXT,
  instructor TEXT, code TEXT, instruction_language TEXT, credits REAL,
  credit_split TEXT, meetings TEXT, color TEXT, url TEXT, source_value TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
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
CREATE TABLE memberships (
  id TEXT PRIMARY KEY, term TEXT, course_id TEXT, status TEXT, evidence TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
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
CREATE TABLE assignments (
  id TEXT PRIMARY KEY, course_id TEXT, title TEXT, details TEXT, due_at TEXT,
  source_url TEXT, lms_cmid INTEGER, lms_course_id INTEGER, submission TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
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
CREATE TABLE offerings (
  id TEXT PRIMARY KEY, year INTEGER, quarters TEXT, code TEXT, class_code TEXT,
  unit_code TEXT, title TEXT, instructors TEXT, instruction_language TEXT,
  format TEXT, media TEXT, class_label TEXT, credits TEXT, schedule TEXT,
  academic_unit TEXT, intensive INTEGER, syllabus_url TEXT, source_updated TEXT,
  fetched_at TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE offering_content (
  offering_id TEXT NOT NULL, lang TEXT NOT NULL, content TEXT NOT NULL,
  fetched_at TEXT NOT NULL, PRIMARY KEY (offering_id, lang)
);
CREATE TABLE catalog_search_items (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE VIRTUAL TABLE offerings_fts USING fts5(
  offering_id UNINDEXED, code, title_ja, title_en, instructors_ja, instructors_en
);
CREATE TABLE lms_courses (
  id TEXT PRIMARY KEY, lms_id INTEGER UNIQUE, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE lms_items (
  id TEXT PRIMARY KEY, cmid INTEGER, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
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
CREATE TABLE import_runs (
  id TEXT PRIMARY KEY, provider TEXT NOT NULL, scope TEXT NOT NULL,
  state TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT, message TEXT
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
CREATE TABLE gpa_summary (
  id TEXT PRIMARY KEY, scope TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE gpa_by_term (
  id TEXT PRIMARY KEY, year INTEGER, term TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE subscriptions (
  id TEXT PRIMARY KEY, url TEXT, kind TEXT, enabled INTEGER, subscription_hash TEXT,
  fetched_at TEXT, last_error TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT
);
CREATE TABLE academic_calendar (
  id TEXT PRIMARY KEY, date TEXT, type TEXT, replacement_day INTEGER, label TEXT,
  source_hash TEXT, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE departments (
  id TEXT PRIMARY KEY, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE recommended_courses (
  id TEXT PRIMARY KEY, department_id TEXT, code TEXT, course_group TEXT,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
CREATE TABLE important_dates (
  id TEXT PRIMARY KEY, academic_year INTEGER NOT NULL, start_date TEXT NOT NULL,
  end_date TEXT NOT NULL, category TEXT NOT NULL, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE INDEX important_dates_range ON important_dates(academic_year,start_date,end_date);
CREATE TABLE exams (
  id TEXT PRIMARY KEY, academic_year INTEGER NOT NULL, level TEXT NOT NULL,
  quarter TEXT NOT NULL, course_code TEXT NOT NULL, exam_date TEXT NOT NULL,
  status TEXT NOT NULL, value TEXT NOT NULL, hash TEXT NOT NULL,
  updated_at TEXT NOT NULL, updated_by TEXT NOT NULL, rev INTEGER NOT NULL,
  source TEXT, deleted_at TEXT, last_seen_at TEXT, missing_since TEXT
);
CREATE INDEX exams_by_course ON exams(academic_year,course_code);
CREATE INDEX exams_by_date ON exams(academic_year,exam_date);
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

const CREATE_CLASS_SCHEDULES: &str = r#"CREATE TABLE class_schedules (
  id TEXT PRIMARY KEY, academic_year INTEGER NOT NULL, course_code TEXT NOT NULL,
  value TEXT NOT NULL, hash TEXT NOT NULL, updated_at TEXT NOT NULL,
  updated_by TEXT NOT NULL, rev INTEGER NOT NULL, source TEXT, deleted_at TEXT,
  last_seen_at TEXT, missing_since TEXT
);
"#;

pub fn migrate(connection: &mut Connection) -> Result<bool, DataError> {
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
        .map_err(DataError::from)?;
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(DataError::from)?;
    if version == SCHEMA_VERSION {
        return Ok(false);
    }
    // Never rebuild a populated database, including on downgrade or an unknown schema.
    let populated: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' AND type='table')",
        [], |row| row.get(0),
    ).map_err(DataError::from)?;
    if version != 6 && (version != 0 || populated) {
        return Err(DataError::compatibility(format!(
            "Database schema {version} is not supported. Your database was left intact."
        )));
    }
    let transaction = connection.transaction().map_err(DataError::from)?;
    if version == 0 {
        transaction
            .execute_batch(CREATE_SCHEMA)
            .map_err(DataError::from)?;
    }
    // Schema 7 only adds this cache table; all user and school rows stay in place.
    transaction
        .execute_batch(CREATE_CLASS_SCHEDULES)
        .map_err(DataError::from)?;
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(DataError::from)?;
    transaction.commit().map_err(DataError::from)?;
    Ok(true)
}

#[cfg(test)]
#[path = "../tests/data__migrations.rs"]
mod tests;
