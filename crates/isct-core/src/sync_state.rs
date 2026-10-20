//! Freshness per (dataset, key): the one record of whether a piece of school
//! or reference data is current.
//!
//! A key is the atomic unit of a sync. Its rows and its `dataset_state` row
//! commit in one transaction ([`commit`]), so nothing is marked fresh unless
//! all of it landed. A failed or cancelled read writes no rows; [`fail`] only
//! pushes that key's next attempt back. Every read also leaves one
//! `sync_runs` row ([`record_run`]) for diagnostics.
use crate::data::DataError;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

/// A kind of data with one freshness policy. Names are stored.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord, Serialize)]
pub enum Dataset {
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
    /// One シラバス detail page. Key: offering id.
    CatalogDetail,
    /// One Isct-Data file. Key: `{kind}/{year}`.
    Reference,
}

impl Dataset {
    pub const ALL: [Dataset; 9] = [
        Self::LmsEnrolment,
        Self::LmsDeadlines,
        Self::LmsCourse,
        Self::LmsAssign,
        Self::LmsHistory,
        Self::KyomuPage,
        Self::CatalogOfficial,
        Self::CatalogDetail,
        Self::Reference,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::LmsEnrolment => "lms.enrolment",
            Self::LmsDeadlines => "lms.deadlines",
            Self::LmsCourse => "lms.course",
            Self::LmsAssign => "lms.assign",
            Self::LmsHistory => "lms.history",
            Self::KyomuPage => "kyomu.page",
            Self::CatalogOfficial => "catalog.official",
            Self::CatalogDetail => "catalog.detail",
            Self::Reference => "reference",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|dataset| dataset.name() == name)
    }
}

/// How old a key's content is allowed to get, by the term it belongs to.
/// Only LMS course data distinguishes terms; other datasets use `Current`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Term {
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
pub fn ttl(dataset: Dataset, term: Term) -> Option<Duration> {
    let seconds = match (dataset, term) {
        (_, Term::Frozen) => return None,
        (Dataset::LmsEnrolment, _) => 12 * HOUR,
        (Dataset::LmsDeadlines, _) => 2 * HOUR,
        (Dataset::LmsCourse, Term::Current) => DAY,
        (Dataset::LmsAssign, Term::Current) => 6 * HOUR,
        (Dataset::LmsCourse | Dataset::LmsAssign, Term::Past) => 30 * DAY,
        (Dataset::LmsHistory, _) => DAY,
        (Dataset::KyomuPage | Dataset::CatalogOfficial, _) => 7 * DAY,
        (Dataset::CatalogDetail, _) => 14 * DAY,
        (Dataset::Reference, _) => DAY,
    };
    Some(Duration::seconds(seconds))
}

/// One freshness key.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub dataset: Dataset,
    pub key: String,
}

impl Key {
    pub fn new(dataset: Dataset, key: impl Into<String>) -> Self {
        Self {
            dataset,
            key: key.into(),
        }
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}/{}", self.dataset.name(), self.key)
    }
}

/// The account a key's content belongs to: `science:<username>`,
/// `tokyotech:<username>`, or `""` for public data. A key stored for another
/// account counts as missing.
pub fn account(profile: crate::school_auth::SchoolAuthProfile, username: &str) -> String {
    let prefix = match profile {
        crate::school_auth::SchoolAuthProfile::Science => "science",
        crate::school_auth::SchoolAuthProfile::TokyoTech => "tokyotech",
    };
    format!("{prefix}:{}", username.trim())
}

/// What the last complete read of a key found.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
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
pub struct KeyState {
    pub dataset: String,
    pub key: String,
    pub account: String,
    pub generation: i64,
    pub fetched_at: Option<DateTime<Utc>>,
    pub validator: Option<String>,
    pub outcome: Option<Outcome>,
    pub message: Option<String>,
    pub failures: u32,
    pub next_eligible: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_used: Option<DateTime<Utc>>,
}

/// Whether a key should be read now. Ordered by urgency: missing content
/// comes before stale content.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum Due {
    /// Never fetched, invalidated (gc, reset) or stored for another account.
    /// Beats TTL, freezing and validators.
    Missing,
    Stale,
    Fresh,
    /// Failed recently; not before this time.
    BackingOff(DateTime<Utc>),
}

impl Due {
    pub fn is_due(self) -> bool {
        matches!(self, Self::Missing | Self::Stale)
    }
}

/// Automatic due check. Manual refresh ignores TTL and backoff and does not
/// call this.
pub fn due(
    state: Option<&KeyState>,
    ttl: Option<Duration>,
    account: &str,
    now: DateTime<Utc>,
) -> Due {
    let Some(state) = state else {
        return Due::Missing;
    };
    if let Some(until) = state.next_eligible.filter(|until| *until > now) {
        return Due::BackingOff(until);
    }
    let Some(fetched) = state.fetched_at.filter(|_| state.account == account) else {
        return Due::Missing;
    };
    match ttl {
        Some(ttl) if now >= fetched + ttl => Due::Stale,
        _ => Due::Fresh,
    }
}

/// Retry delay after `failures` consecutive failures: 5 min, ×3 each time,
/// at most 6 h.
pub fn backoff(failures: u32) -> Duration {
    let minutes = 5i64.saturating_mul(3i64.saturating_pow(failures.saturating_sub(1)));
    Duration::minutes(minutes.min(6 * 60))
}

/// A complete read of one key.
pub struct Commit<'a> {
    pub account: &'a str,
    /// ETag, content hash or index hash, when the source offers one.
    pub validator: Option<&'a str>,
    pub outcome: Outcome,
    /// Short reason shown for `Unavailable`; never response text.
    pub message: Option<&'a str>,
}

/// Marks `key` fresh. Call inside the transaction that wrote the key's rows.
pub fn commit(
    connection: &Connection,
    key: &Key,
    commit: Commit<'_>,
    now: DateTime<Utc>,
) -> Result<(), DataError> {
    connection
        .execute(
            "INSERT INTO dataset_state(dataset,key,account,fetched_at,validator,outcome,message,state,failures)
             VALUES(?1,?2,?3,?4,?5,?6,?7,'idle',0)
             ON CONFLICT(dataset,key) DO UPDATE SET account=excluded.account,
               fetched_at=excluded.fetched_at, validator=excluded.validator,
               outcome=excluded.outcome, message=excluded.message, state='idle',
               lease_run=NULL, failures=0, next_eligible=NULL, last_error=NULL",
            params![
                key.dataset.name(),
                key.key,
                commit.account,
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
/// Returns false, and changes nothing, when the key has no stored content
/// for `account`; the caller must then read the content in full.
pub fn renew(
    connection: &Connection,
    key: &Key,
    account: &str,
    now: DateTime<Utc>,
) -> Result<bool, DataError> {
    let renewed = connection
        .execute(
            "UPDATE dataset_state SET fetched_at=?4, state='idle', lease_run=NULL,
               failures=0, next_eligible=NULL, last_error=NULL
             WHERE dataset=?1 AND key=?2 AND account=?3 AND fetched_at IS NOT NULL",
            params![key.dataset.name(), key.key, account, now.to_rfc3339()],
        )
        .map_err(DataError::from)?;
    Ok(renewed > 0)
}

/// Records a failed read and returns when the key may be tried again.
/// Stored rows and `fetched_at` stay. `error_kind` is a short code
/// (`timeout`, `http-503`, `parse`), never response text.
pub fn fail(
    connection: &Connection,
    key: &Key,
    account: &str,
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
            "INSERT INTO dataset_state(dataset,key,account,state,failures,next_eligible,last_error)
             VALUES(?1,?2,?3,'failed',?4,?5,?6)
             ON CONFLICT(dataset,key) DO UPDATE SET state='failed', lease_run=NULL,
               failures=excluded.failures, next_eligible=excluded.next_eligible,
               last_error=excluded.last_error",
            params![
                key.dataset.name(),
                key.key,
                account,
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
pub fn invalidate(connection: &Connection, key: &Key) -> Result<(), DataError> {
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
pub fn touch(connection: &Connection, keys: &[Key], now: DateTime<Utc>) -> Result<(), DataError> {
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

const COLUMNS: &str = "dataset,key,account,generation,fetched_at,validator,outcome,message,
    failures,next_eligible,last_error,last_used";

fn time(value: Option<String>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn key_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<KeyState> {
    Ok(KeyState {
        dataset: row.get(0)?,
        key: row.get(1)?,
        account: row.get(2)?,
        generation: row.get(3)?,
        fetched_at: time(row.get(4)?),
        validator: row.get(5)?,
        outcome: match row.get::<_, Option<String>>(6)?.as_deref() {
            Some("complete") => Some(Outcome::Complete),
            Some("unavailable") => Some(Outcome::Unavailable),
            _ => None,
        },
        message: row.get(7)?,
        failures: row.get(8)?,
        next_eligible: time(row.get(9)?),
        last_error: row.get(10)?,
        last_used: time(row.get(11)?),
    })
}

pub fn get(connection: &Connection, key: &Key) -> Result<Option<KeyState>, DataError> {
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
pub fn list(connection: &Connection, dataset: Option<Dataset>) -> Result<Vec<KeyState>, DataError> {
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
pub enum RunOutcome {
    Complete,
    /// Validator matched; only `fetched_at` moved.
    Unchanged,
    Unavailable,
    Failed,
    Cancelled,
    SignInRequired,
}

impl RunOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Unchanged => "unchanged",
            Self::Unavailable => "unavailable",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::SignInRequired => "sign-in-required",
        }
    }
}

/// One `sync_runs` row.
pub struct RunRecord<'a> {
    /// Correlates the rows of one pull (the pull job id).
    pub run: &'a str,
    pub key: &'a Key,
    pub account: &'a str,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub outcome: RunOutcome,
    pub changed: usize,
    pub requests: Option<u32>,
    pub bytes: Option<u64>,
    /// Canonical source: a REST function name, `host/path` for a page
    /// ([`source_of_url`]), or an Isct-Data file URL. Never a query string.
    pub source: &'a str,
    pub error_kind: Option<&'a str>,
}

pub fn record_run(connection: &Connection, run: &RunRecord<'_>) -> Result<(), DataError> {
    connection
        .execute(
            "INSERT INTO sync_runs(run,dataset,key,account,started_at,finished_at,outcome,changed,requests,bytes,source,error_kind)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                run.run,
                run.key.dataset.name(),
                run.key.key,
                run.account,
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
pub fn source_of_url(url: &url::Url) -> String {
    format!("{}{}", url.host_str().unwrap_or(""), url.path())
}

#[cfg(test)]
#[path = "tests/sync_state.rs"]
mod tests;

/// The candidate version a key is holding back (a public file that shrank
/// suspiciously), stored in `dataset_state.cursor`.
pub fn cursor(connection: &Connection, key: &Key) -> Result<Option<String>, DataError> {
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
pub fn set_cursor(
    connection: &Connection,
    key: &Key,
    account: &str,
    cursor: Option<&str>,
) -> Result<(), DataError> {
    match cursor {
        Some(cursor) => connection.execute(
            "INSERT INTO dataset_state(dataset,key,account,cursor) VALUES(?1,?2,?3,?4)
             ON CONFLICT(dataset,key) DO UPDATE SET cursor=excluded.cursor",
            params![key.dataset.name(), key.key, account, cursor],
        ),
        None => connection.execute(
            "UPDATE dataset_state SET cursor=NULL WHERE dataset=?1 AND key=?2",
            params![key.dataset.name(), key.key],
        ),
    }
    .map_err(DataError::from)?;
    Ok(())
}
