//! Freshness per (dataset, key): the one record of whether a piece of school
//! or reference data is current, kept like a keyspace (`d/azfvr2`).
//!
//! A key is the atomic unit of a sync. Its rows and its `dataset_state` row
//! commit in one transaction ([`commit`]), so nothing is marked fresh unless
//! all of it landed. A failed or cancelled read writes no rows; [`fail`] only
//! pushes that key's next attempt back.
//!
//! Besides when it was read, each key holds when its content last changed
//! (`changed_at`) and a sorted set of times it falls due because something
//! else changed (`due_marks`, [`mark_due`]). Every read ends in
//! [`record_run`], in the transaction that wrote its rows: a read that
//! landed clears the marks it answers (set before it started), and one that
//! changed rows stamps `changed_at` and marks what depends on the key
//! ([`dependents`]). Ingest only says whether its key changed; the planner
//! only reads these facts. `sync_runs` is diagnostics.
use crate::data::DataError;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// A kind of data with one freshness policy. Names are stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize)]
pub(crate) enum Dataset {
    /// The enrolled course list and the timetable grids. Key: `all`.
    LmsEnrolment,
    /// Open work and upcoming events. Key: `all`.
    LmsDeadlines,
    /// One course: its structure and module details. Key: LMS course id.
    LmsCourse,
    /// One course's assignment and quiz states. Key: LMS course id.
    LmsAssign,
    /// One past month of calendar deadlines. Key: `YYYY-MM`.
    LmsHistory,
    /// One approved 教務Web page. Key: page kind (`gpa`, `grades`, …).
    KyomuPage,
    /// 教務Web course search for one department. Key: `{year}/{prefix}`.
    CatalogOfficial,
    /// One Isct-Data file. Key: `{kind}/{year}`.
    Reference,
    /// The in-app announcements file Isct-Data serves outside its index.
    /// Key: its format (`v1`).
    Announcements,
}

impl Dataset {
    pub(crate) const ALL: [Dataset; 9] = [
        Self::LmsEnrolment,
        Self::LmsDeadlines,
        Self::LmsCourse,
        Self::LmsAssign,
        Self::LmsHistory,
        Self::KyomuPage,
        Self::CatalogOfficial,
        Self::Reference,
        Self::Announcements,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::LmsEnrolment => "lms.enrolment",
            Self::LmsDeadlines => "lms.deadlines",
            Self::LmsCourse => "lms.course",
            Self::LmsAssign => "lms.assign",
            Self::LmsHistory => "lms.history",
            Self::KyomuPage => "kyomu.page",
            Self::CatalogOfficial => "catalog.official",
            Self::Reference => "reference",
            Self::Announcements => "announcements",
        }
    }

    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|dataset| dataset.name() == name)
    }
}

/// How old a key's content is allowed to get, by the term it belongs to.
/// Only LMS course data distinguishes terms; other datasets use `Current`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Term {
    Current,
    /// An earlier term of the current academic year.
    Past,
    /// An earlier academic year, or a month that is over: read once, then
    /// renewed only by hand or when its content is missing.
    Frozen,
}

const HOUR: i64 = 60 * 60;
const DAY: i64 = 24 * HOUR;

/// The base TTL of a key, or `None` when a fetched key is never renewed
/// automatically.
/// Each source's own rule, by `d/0mzmh8` and the decisions under it: the
/// 学籍 page almost never changes, every other 教務 page a week.
pub(crate) fn ttl(key: &Key, term: Term) -> Option<Duration> {
    let seconds = match (key.dataset, term) {
        (_, Term::Frozen) => return None,
        (Dataset::LmsEnrolment, _) => 12 * HOUR,
        (Dataset::LmsDeadlines, _) => 2 * HOUR,
        (Dataset::LmsCourse, Term::Current) => DAY,
        (Dataset::LmsAssign, Term::Current) => 6 * HOUR,
        (Dataset::LmsCourse | Dataset::LmsAssign, Term::Past) => 30 * DAY,
        (Dataset::LmsHistory, _) => DAY,
        (Dataset::KyomuPage, _) if key.key == "profile" => 30 * DAY,
        (Dataset::KyomuPage | Dataset::CatalogOfficial, _) => 7 * DAY,
        (Dataset::Reference, _) => DAY,
        // Checked while the app is in the foreground only
        // (`plan::planner::automatic_keys`).
        (Dataset::Announcements, _) => 55 * 60,
    };
    Some(Duration::seconds(seconds))
}

/// One freshness key.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub(crate) struct Key {
    pub dataset: Dataset,
    pub key: String,
}

impl Key {
    pub(crate) fn new(dataset: Dataset, key: impl Into<String>) -> Self {
        Self {
            dataset,
            key: key.into(),
        }
    }

    pub(crate) fn lms_enrolment() -> Self {
        Self::new(Dataset::LmsEnrolment, "all")
    }

    pub(crate) fn lms_deadlines() -> Self {
        Self::new(Dataset::LmsDeadlines, "all")
    }

    pub(crate) fn lms_assignments(course_id: u64) -> Self {
        Self::new(Dataset::LmsAssign, course_id.to_string())
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}/{}", self.dataset.name(), self.key)
    }
}

/// What the last complete read of a key found.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Outcome {
    Complete,
    /// The source said this content is not offered to the account (a closed
    /// 教務 window). Stored rows stay; the read still counts as current.
    Unavailable,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One `dataset_state` row.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeyState {
    pub dataset: String,
    pub key: String,
    pub(crate) fetched_at: Option<DateTime<Utc>>,
    pub(crate) validator: Option<String>,
    pub outcome: Option<Outcome>,
    pub message: Option<String>,
    pub failures: u32,
    pub next_eligible: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    /// When a complete read last changed the key's rows.
    pub changed_at: Option<DateTime<Utc>>,
    /// When the key falls due because something it depends on changed,
    /// earliest first.
    pub due_marks: Vec<DateTime<Utc>>,
    pub last_used: Option<DateTime<Utc>>,
}

/// Whether a key should be read now. Ordered by urgency: missing content
/// comes before stale content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub(crate) enum Due {
    /// Never fetched, or invalidated (gc, reset). Beats TTL, freezing and
    /// validators.
    Missing,
    Stale,
    Fresh,
    /// Failed recently; not before this time.
    BackingOff(DateTime<Utc>),
}

impl Due {
    #[cfg(test)]
    pub(crate) fn is_due(self) -> bool {
        matches!(self, Self::Missing | Self::Stale)
    }
}

/// Automatic due check. Manual refresh ignores TTL and backoff and does not
/// call this.
pub(crate) fn due(state: Option<&KeyState>, ttl: Option<Duration>, now: DateTime<Utc>) -> Due {
    let Some(state) = state else {
        return Due::Missing;
    };
    if let Some(until) = state.next_eligible.filter(|until| *until > now) {
        return Due::BackingOff(until);
    }
    let Some(fetched) = state.fetched_at else {
        return Due::Missing;
    };
    if expires_at(fetched, ttl).is_some_and(|at| now >= at) {
        Due::Stale
    } else {
        Due::Fresh
    }
}

pub(crate) fn expires_at(fetched: DateTime<Utc>, ttl: Option<Duration>) -> Option<DateTime<Utc>> {
    ttl.map(|ttl| fetched + ttl)
}

/// Retry delay after `failures` consecutive failures: 5 min, ×3 each time,
/// at most 6 h.
pub(crate) fn backoff(failures: u32) -> Duration {
    let minutes = 5i64.saturating_mul(3i64.saturating_pow(failures.saturating_sub(1)));
    Duration::minutes(minutes.min(6 * 60))
}

/// A complete read of one key.
pub(crate) struct Commit<'a> {
    /// ETag, content hash or index hash, when the source offers one.
    pub(crate) validator: Option<&'a str>,
    pub outcome: Outcome,
    /// Short reason shown for `Unavailable`; never response text.
    pub message: Option<&'a str>,
}

/// Marks `key` fresh. Call inside the transaction that wrote the key's rows.
pub(crate) fn commit(
    connection: &Connection,
    key: &Key,
    commit: Commit<'_>,
    now: DateTime<Utc>,
) -> Result<(), DataError> {
    connection
        .execute(
            "INSERT INTO dataset_state(dataset,key,fetched_at,validator,outcome,message,failures)
             VALUES(?1,?2,?3,?4,?5,?6,0)
             ON CONFLICT(dataset,key) DO UPDATE SET
               fetched_at=excluded.fetched_at, validator=excluded.validator,
               outcome=excluded.outcome, message=excluded.message,
               failures=0, next_eligible=NULL, last_error=NULL",
            params![
                key.dataset.name(),
                key.key,
                now.to_rfc3339(),
                commit.validator,
                commit.outcome.as_str(),
                commit.message,
            ],
        )
        .map_err(DataError::from)?;
    Ok(())
}

/// The source reported no change (304, same hash): renew `fetched_at` only.
/// Returns false, and changes nothing, when the key has no stored content;
/// the caller must then read the content in full.
pub(crate) fn renew(
    connection: &Connection,
    key: &Key,
    now: DateTime<Utc>,
) -> Result<bool, DataError> {
    let renewed = connection
        .execute(
            "UPDATE dataset_state SET fetched_at=?3, failures=0, next_eligible=NULL, last_error=NULL
             WHERE dataset=?1 AND key=?2 AND fetched_at IS NOT NULL",
            params![key.dataset.name(), key.key, now.to_rfc3339()],
        )
        .map_err(DataError::from)?;
    Ok(renewed > 0)
}

/// Records a failed read and returns when the key may be tried again.
/// Stored rows and `fetched_at` stay. `error_kind` is a short code
/// (`timeout`, `http-503`, `parse`), never response text.
pub(crate) fn fail(
    connection: &Connection,
    key: &Key,
    error_kind: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, DataError> {
    let failures: u32 = connection
        .query_row(
            "SELECT failures FROM dataset_state WHERE dataset=?1 AND key=?2",
            params![key.dataset.name(), key.key],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?
        .unwrap_or(0);
    let failures = failures.saturating_add(1);
    let next = now + backoff(failures);
    connection
        .execute(
            "INSERT INTO dataset_state(dataset,key,failures,next_eligible,last_error)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(dataset,key) DO UPDATE SET
               failures=excluded.failures, next_eligible=excluded.next_eligible,
               last_error=excluded.last_error",
            params![
                key.dataset.name(),
                key.key,
                failures,
                next.to_rfc3339(),
                error_kind,
            ],
        )
        .map_err(DataError::from)?;
    Ok(next)
}

/// Forgets that `key` was fetched, so it reads as missing: call in the
/// transaction that deletes its content (gc), so no validator or freeze can
/// keep the gap.
pub(crate) fn invalidate(connection: &Connection, key: &Key) -> Result<(), DataError> {
    connection
        .execute(
            "UPDATE dataset_state SET fetched_at=NULL, validator=NULL, outcome=NULL
             WHERE dataset=?1 AND key=?2",
            params![key.dataset.name(), key.key],
        )
        .map_err(DataError::from)?;
    Ok(())
}

/// Notes that the interface read these keys, for gc.
pub(crate) fn touch(
    connection: &Connection,
    keys: &[Key],
    now: DateTime<Utc>,
) -> Result<(), DataError> {
    let mut statement = connection
        .prepare_cached("UPDATE dataset_state SET last_used=?3 WHERE dataset=?1 AND key=?2")
        .map_err(DataError::from)?;
    let now = now.to_rfc3339();
    for key in keys {
        statement
            .execute(params![key.dataset.name(), key.key, now])
            .map_err(DataError::from)?;
    }
    Ok(())
}

const COLUMNS: &str = "dataset,key,fetched_at,validator,outcome,message,
    failures,next_eligible,last_error,last_used,changed_at,due_marks";

fn time(value: Option<String>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn key_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<KeyState> {
    Ok(KeyState {
        dataset: row.get(0)?,
        key: row.get(1)?,
        fetched_at: time(row.get(2)?),
        validator: row.get(3)?,
        outcome: match row.get::<_, Option<String>>(4)?.as_deref() {
            Some("complete") => Some(Outcome::Complete),
            Some("unavailable") => Some(Outcome::Unavailable),
            _ => None,
        },
        message: row.get(5)?,
        failures: row.get(6)?,
        next_eligible: time(row.get(7)?),
        last_error: row.get(8)?,
        last_used: time(row.get(9)?),
        changed_at: time(row.get(10)?),
        due_marks: marks(row.get::<_, Option<String>>(11)?.as_deref()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}

/// The stored set of due marks, earliest first. A value `store_marks` did
/// not write fails the read.
fn marks(raw: Option<&str>) -> Result<Vec<DateTime<Utc>>, DataError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let corrupt = |error: &dyn std::fmt::Display| {
        DataError::Storage(format!("corrupt due_marks {raw:?}: {error}"))
    };
    let mut marks = serde_json::from_str::<Vec<String>>(raw)
        .map_err(|error| corrupt(&error))?
        .iter()
        .map(|at| {
            DateTime::parse_from_rfc3339(at)
                .map(|at| at.with_timezone(&Utc))
                .map_err(|error| corrupt(&error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    marks.sort();
    marks.dedup();
    Ok(marks)
}

fn store_marks(
    connection: &Connection,
    key: &Key,
    marks: &[DateTime<Utc>],
) -> Result<(), DataError> {
    let raw = (!marks.is_empty()).then(|| {
        serde_json::Value::from(marks.iter().map(|at| at.to_rfc3339()).collect::<Vec<_>>())
            .to_string()
    });
    connection
        .execute(
            "INSERT INTO dataset_state(dataset,key,due_marks) VALUES(?1,?2,?3)
             ON CONFLICT(dataset,key) DO UPDATE SET due_marks=excluded.due_marks",
            params![key.dataset.name(), key.key, raw],
        )
        .map_err(DataError::from)?;
    Ok(())
}

fn stored_marks(connection: &Connection, key: &Key) -> Result<Vec<DateTime<Utc>>, DataError> {
    let raw: Option<Option<String>> = connection
        .query_row(
            "SELECT due_marks FROM dataset_state WHERE dataset=?1 AND key=?2",
            params![key.dataset.name(), key.key],
            |row| row.get(0),
        )
        .optional()
        .map_err(DataError::from)?;
    marks(raw.flatten().as_deref())
}

/// Makes `key` due at `at`, beside any marks it already has: like `ZADD`,
/// the set keeps every distinct time until a read answers it.
pub(crate) fn mark_due(connection: &Connection, key: &Key, at: DateTime<Utc>) -> Result<(), DataError> {
    let mut marks = stored_marks(connection, key)?;
    if !marks.contains(&at) {
        marks.push(at);
        marks.sort();
    }
    store_marks(connection, key, &marks)
}

/// Which keys fall due when `key`'s content changes, and after how long:
/// the one table of these dependencies (`d/azfvr2`, beside the read-order
/// table `plan::planner::prerequisite`). The school can compute the GPA and
/// requirements after the grades land, so they are read at once and a day
/// later (`d/aa17ta`). A deadline that moved marks its own course's
/// assignments, by row, in the deadline ingest.
fn dependents(key: &Key) -> Vec<(Key, Duration)> {
    match (key.dataset, key.key.as_str()) {
        (Dataset::KyomuPage, "grades") => ["gpa", "requirements"]
            .into_iter()
            .flat_map(|page| {
                [Duration::zero(), Duration::days(1)]
                    .map(|after| (Key::new(Dataset::KyomuPage, page), after))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The keyspace side of a read that landed: the marks set before it
/// started are answered; a read that changed rows stamps `changed_at` and
/// marks the key's dependents.
fn settle_read(connection: &Connection, run: &RunRecord<'_>) -> Result<(), DataError> {
    let left: Vec<DateTime<Utc>> = stored_marks(connection, run.key)?
        .into_iter()
        .filter(|at| *at > run.started_at)
        .collect();
    store_marks(connection, run.key, &left)?;
    if run.changed > 0 {
        connection
            .execute(
                "UPDATE dataset_state SET changed_at=?3 WHERE dataset=?1 AND key=?2",
                params![
                    run.key.dataset.name(),
                    run.key.key,
                    run.finished_at.to_rfc3339()
                ],
            )
            .map_err(DataError::from)?;
        for (dependent, after) in dependents(run.key) {
            mark_due(connection, &dependent, run.finished_at + after)?;
        }
    }
    Ok(())
}

pub(crate) fn get(connection: &Connection, key: &Key) -> Result<Option<KeyState>, DataError> {
    connection
        .query_row(
            &format!("SELECT {COLUMNS} FROM dataset_state WHERE dataset=?1 AND key=?2"),
            params![key.dataset.name(), key.key],
            key_state,
        )
        .optional()
        .map_err(DataError::from)
}

/// Every key of `dataset`, or of all datasets.
pub(crate) fn list(
    connection: &Connection,
    dataset: Option<Dataset>,
) -> Result<Vec<KeyState>, DataError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {COLUMNS} FROM dataset_state WHERE ?1 IS NULL OR dataset=?1 ORDER BY dataset,key"
        ))
        .map_err(DataError::from)?;
    let rows = statement
        .query_map([dataset.map(Dataset::name)], key_state)
        .map_err(DataError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DataError::from)?;
    Ok(rows)
}

/// How one read of a key ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RunOutcome {
    Complete,
    /// Validator matched; only `fetched_at` moved.
    Unchanged,
    Unavailable,
    Failed,
    Cancelled,
    SignInRequired,
    /// Nothing landed because the data it lands on is not stored yet; the
    /// key waits for it, without a failure.
    Waiting,
}

impl RunOutcome {
    /// A read that reached the school and stored its answer.
    fn lands(self) -> bool {
        matches!(self, Self::Complete | Self::Unchanged | Self::Unavailable)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Unchanged => "unchanged",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::SignInRequired => "sign-in-required",
            Self::Waiting => "waiting",
        }
    }
}

/// One `sync_runs` row.
pub(crate) struct RunRecord<'a> {
    /// Correlates the rows of one pull (the pull job id).
    pub run: &'a str,
    pub key: &'a Key,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) finished_at: DateTime<Utc>,
    pub outcome: RunOutcome,
    pub changed: usize,
    pub requests: Option<u32>,
    pub bytes: Option<u64>,
    /// Canonical source: a REST function name, `host/path` for a page
    /// ([`source_of_url`]), or an Isct-Data file URL. Never a query string.
    pub source: &'a str,
    pub(crate) error_kind: Option<&'a str>,
}

/// Ends one read: its diagnostics row, and for a read that landed
/// ([`RunOutcome::lands`]) its keyspace side (`settle_read`). Call in the
/// transaction that wrote the read's rows.
pub(crate) fn record_run(connection: &Connection, run: &RunRecord<'_>) -> Result<(), DataError> {
    if run.outcome.lands() {
        settle_read(connection, run)?;
    }
    connection
        .execute(
            "INSERT INTO sync_runs(run,dataset,key,started_at,finished_at,outcome,changed,requests,bytes,source,error_kind)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                run.run,
                run.key.dataset.name(),
                run.key.key,
                run.started_at.to_rfc3339(),
                run.finished_at.to_rfc3339(),
                run.outcome.as_str(),
                run.changed as i64,
                run.requests,
                run.bytes.map(|bytes| bytes as i64),
                run.source,
                run.error_kind,
            ],
        )
        .map_err(DataError::from)?;
    Ok(())
}

/// `host/path` of a URL: no scheme, query, fragment or credentials, so
/// `sesskey`, `wstoken`, `token` and `passport` never reach the database.
pub(crate) fn source_of_url(url: &url::Url) -> String {
    format!("{}{}", url.host_str().unwrap_or(""), url.path())
}

#[cfg(test)]
#[path = "tests/sync_state.rs"]
mod tests;

/// The candidate version a key is holding back (a public file that shrank
/// suspiciously), stored in `dataset_state.cursor`.
pub(crate) fn cursor(connection: &Connection, key: &Key) -> Result<Option<String>, DataError> {
    connection
        .query_row(
            "SELECT cursor FROM dataset_state WHERE dataset=?1 AND key=?2",
            params![key.dataset.name(), key.key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(DataError::from)
}

/// Sets `cursor` (creating the key's row, which reads as missing until a
/// commit) or clears it. Freshness, failures and the validator stay.
pub(crate) fn set_cursor(
    connection: &Connection,
    key: &Key,
    cursor: Option<&str>,
) -> Result<(), DataError> {
    match cursor {
        Some(cursor) => connection.execute(
            "INSERT INTO dataset_state(dataset,key,cursor) VALUES(?1,?2,?3)
             ON CONFLICT(dataset,key) DO UPDATE SET cursor=excluded.cursor",
            params![key.dataset.name(), key.key, cursor],
        ),
        None => connection.execute(
            "UPDATE dataset_state SET cursor=NULL WHERE dataset=?1 AND key=?2",
            params![key.dataset.name(), key.key],
        ),
    }
    .map_err(DataError::from)?;
    Ok(())
}
