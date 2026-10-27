//! Reference files: what the person follows, when a file is due, and the
//! executor that reads one leased `reference/{kind}/{year}` key.

use crate::{
    data::{DataError, WorkspaceChangeSource},
    subscriptions::{
        self, AcademicCalendar, ClassSchedules, ImportantDates, PublishedExamFeed, ReferenceFetch,
        SubscriptionClient, SubscriptionError,
    },
    sync_engine::{Gated, KeyEnd, Lease},
    sync_state::{self, Commit, Dataset, Due, Key, KeyState, Outcome, RunOutcome, Term},
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, Transaction};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

/// The public Isct-Data index every reference subscription starts from.
pub const PUBLIC_INDEX: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
/// Every reference dataset kind a subscription can follow.
pub const REFERENCE_KINDS: [&str; 4] = ["calendar", "important-dates", "exams", "class-schedules"];
/// `error_kind` of a public file held back because it shrank suspiciously.
pub(crate) const SUSPICIOUS_SHRINK: &str = "suspicious-shrink";
type ReferenceTransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SubscriptionError>> + Send + 'a>>;

#[derive(Clone, Debug, thiserror::Error)]
pub enum ReferenceError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Storage(String),
    /// A concurrent local edit won (stale revision).
    #[error("{0}")]
    Conflict(String),
    /// Stored data has an unsupported shape or version.
    #[error("{0}")]
    Compatibility(String),
    #[error("Pull cancelled.")]
    Cancelled,
    /// A reset moved the sync generation; the read may not write.
    #[error("The sync was reset.")]
    Superseded,
}

impl From<DataError> for ReferenceError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::InvalidInput(message) => Self::InvalidInput(message),
            DataError::Conflict(message) => Self::Conflict(message),
            DataError::Compatibility(message) => Self::Compatibility(message),
            DataError::Storage(message) => Self::Storage(message),
        }
    }
}

impl From<SubscriptionError> for ReferenceError {
    fn from(error: SubscriptionError) -> Self {
        match error {
            SubscriptionError::Validation(message) => Self::InvalidInput(message),
            SubscriptionError::Network(message) => Self::Network(message),
            SubscriptionError::Storage(message) => Self::Storage(message),
        }
    }
}

#[derive(Clone, Debug)]
enum ReferencePayload {
    Calendar(AcademicCalendar),
    ImportantDates(ImportantDates),
    Exams(Vec<PublishedExamFeed>),
    ClassSchedules(ClassSchedules),
}

#[derive(Clone, Debug)]
struct TransportFetch {
    value: Option<ReferencePayload>,
    data_url: String,
    remote_hash: String,
    source_ts: String,
}

impl<T> From<ReferenceFetch<T>> for TransportFetch
where
    ReferencePayload: From<T>,
{
    fn from(value: ReferenceFetch<T>) -> Self {
        Self {
            value: value.value.map(ReferencePayload::from),
            data_url: value.data_url,
            remote_hash: value.remote_hash,
            source_ts: value.source_ts,
        }
    }
}

impl From<AcademicCalendar> for ReferencePayload {
    fn from(value: AcademicCalendar) -> Self {
        Self::Calendar(value)
    }
}
impl From<ImportantDates> for ReferencePayload {
    fn from(value: ImportantDates) -> Self {
        Self::ImportantDates(value)
    }
}
impl From<ClassSchedules> for ReferencePayload {
    fn from(value: ClassSchedules) -> Self {
        Self::ClassSchedules(value)
    }
}
impl From<Vec<PublishedExamFeed>> for ReferencePayload {
    fn from(value: Vec<PublishedExamFeed>) -> Self {
        Self::Exams(value)
    }
}

impl ReferencePayload {
    /// Rows this version publishes.
    fn rows(&self) -> usize {
        match self {
            Self::Calendar(calendar) => calendar.quarters.len() * 2 + calendar.days.len(),
            Self::ImportantDates(dates) => dates.entries.len(),
            Self::Exams(feeds) => feeds.iter().map(|feed| feed.feed.entries.len()).sum(),
            Self::ClassSchedules(schedules) => schedules.entries.len(),
        }
    }

    /// Writes the file's rows; every exam feed of the file shares the one
    /// transaction.
    fn apply(
        &self,
        transaction: &Transaction<'_>,
    ) -> Result<(usize, Option<Vec<Value>>), SubscriptionError> {
        Ok(match self {
            Self::Calendar(value) => (
                subscriptions::apply_reference(transaction, Some(value), None)?,
                None,
            ),
            Self::ImportantDates(value) => (
                subscriptions::apply_important_dates(transaction, value)?,
                None,
            ),
            Self::ClassSchedules(value) => (
                subscriptions::apply_class_schedules(transaction, value)?,
                None,
            ),
            Self::Exams(feeds) => {
                let mut changed = 0;
                let mut files = Vec::new();
                for feed in feeds {
                    changed += subscriptions::apply_exam_feed(transaction, feed)?;
                    files.push(json!({
                        "level":feed.level,
                        "quarter":feed.quarter.to_ascii_uppercase(),
                        "dataUrl":feed.url
                    }));
                }
                (changed, Some(files))
            }
        })
    }
}

trait ReferenceTransport: Send + Sync {
    fn probe<'a>(&'a self, index_url: &'a str) -> ReferenceTransportFuture<'a, String>;
    fn fetch<'a>(
        &'a self,
        kind: &'a str,
        index_url: &'a str,
        year: i32,
        expected_index_hash: &'a str,
        known_hash: Option<&'a str>,
    ) -> ReferenceTransportFuture<'a, TransportFetch>;
}

struct HttpReferenceTransport;

impl ReferenceTransport for HttpReferenceTransport {
    fn probe<'a>(&'a self, index_url: &'a str) -> ReferenceTransportFuture<'a, String> {
        Box::pin(async move {
            SubscriptionClient::new()?
                .probe_index_source(index_url)
                .await
        })
    }

    fn fetch<'a>(
        &'a self,
        kind: &'a str,
        index_url: &'a str,
        year: i32,
        expected_index_hash: &'a str,
        known_hash: Option<&'a str>,
    ) -> ReferenceTransportFuture<'a, TransportFetch> {
        Box::pin(async move {
            let client = SubscriptionClient::new()?;
            match kind {
                "calendar" => Ok(client
                    .fetch_calendar(index_url, year, Some(expected_index_hash), known_hash)
                    .await?
                    .into()),
                "important-dates" => Ok(client
                    .fetch_important_dates(index_url, year, Some(expected_index_hash), known_hash)
                    .await?
                    .into()),
                "exams" => Ok(client
                    .fetch_exams(index_url, year, Some(expected_index_hash), known_hash)
                    .await?
                    .into()),
                "class-schedules" => Ok(client
                    .fetch_class_schedules(index_url, year, Some(expected_index_hash), known_hash)
                    .await?
                    .into()),
                _ => Err(SubscriptionError::Validation(
                    "unsupported subscription type".into(),
                )),
            }
        })
    }
}

/// Index hashes probed recently, so one evaluation's reference keys probe a
/// shared Isct-Data index once.
#[derive(Default)]
pub(crate) struct IndexProbes(Mutex<HashMap<String, Probe>>);

/// When an index was probed, and the hash (or error) it answered.
type Probe = (Instant, Result<String, ReferenceError>);

/// How long a probed index hash is reused.
const PROBE_REUSE: Duration = Duration::from_secs(60);

impl IndexProbes {
    async fn hash(
        &self,
        transport: &dyn ReferenceTransport,
        index_url: &str,
    ) -> Result<String, ReferenceError> {
        let mut cache = self.0.lock().await;
        if let Some((at, result)) = cache.get(index_url) {
            if at.elapsed() < PROBE_REUSE {
                return result.clone();
            }
        }
        let result = transport
            .probe(index_url)
            .await
            .map_err(ReferenceError::from);
        cache.insert(index_url.into(), (Instant::now(), result.clone()));
        result
    }
}

/// The reference files the person follows: every enabled kind for the
/// academic year being viewed, from its subscription's index.
pub(crate) fn followed(workspace: &Value) -> Vec<(String, i32, String)> {
    let Some(year) = subscription_year(workspace) else {
        return Vec::new();
    };
    REFERENCE_KINDS
        .iter()
        .filter_map(|kind| {
            let row = subscription_row(workspace, kind);
            if row.is_some_and(|row| row["enabled"].as_bool() == Some(false)) {
                return None;
            }
            let index_url = row
                .and_then(|row| row["url"].as_str())
                .and_then(normalized_index)
                .unwrap_or_else(|| PUBLIC_INDEX.to_string());
            Some(((*kind).to_string(), year, index_url))
        })
        .collect()
}

/// An HTTPS index URL without credentials or fragment.
fn normalized_index(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw.trim()).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url.to_string())
}

/// Whether a reference key is read now: `sync_state::due`, and a fresh key
/// whose validator came from another index, whose rows are gone, or that
/// holds back a shrunk version is read again.
pub(crate) fn reference_due(
    connection: &Connection,
    key: &Key,
    kind: &str,
    year: i32,
    index_url: &str,
    now: DateTime<Utc>,
) -> Result<Due, DataError> {
    let stored = StoredFile::read(connection, key, kind, year)?;
    let ttl = sync_state::ttl(Dataset::Reference, Term::Current);
    Ok(match sync_state::due(stored.state.as_ref(), ttl, "", now) {
        Due::Fresh if stored.held.is_some() || stored.validator(index_url).is_none() => Due::Stale,
        due => due,
    })
}

/// Reads one leased reference key over Isct-Data.
pub(crate) async fn read_reference(lease: &Lease, probes: &IndexProbes) -> KeyEnd {
    ReferenceReader::default().read(lease, probes).await
}

/// The reference executor: its transport and clock, replaceable in tests.
#[derive(Clone)]
pub(crate) struct ReferenceReader {
    transport: Arc<dyn ReferenceTransport>,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl Default for ReferenceReader {
    fn default() -> Self {
        Self {
            transport: Arc::new(HttpReferenceTransport),
            clock: Arc::new(Utc::now),
        }
    }
}

impl ReferenceReader {
    #[cfg(test)]
    fn new(
        transport: Arc<dyn ReferenceTransport>,
        clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    ) -> Self {
        Self { transport, clock }
    }

    pub(crate) async fn read(&self, lease: &Lease, probes: &IndexProbes) -> KeyEnd {
        let index_url = lease
            .locator
            .clone()
            .unwrap_or_else(|| PUBLIC_INDEX.to_string());
        let Some((kind, year)) = lease
            .key
            .key
            .rsplit_once('/')
            .and_then(|(kind, year)| Some((kind.to_string(), year.parse::<i32>().ok()?)))
            .filter(|(kind, _)| REFERENCE_KINDS.contains(&kind.as_str()))
        else {
            return KeyEnd::failed("unsupported", index_url);
        };
        match self.read_kind(lease, &kind, year, &index_url, probes).await {
            Ok(Read::Current(value)) => KeyEnd::Done {
                changed: value["changed"]
                    .as_u64()
                    .and_then(|changed| usize::try_from(changed).ok())
                    .unwrap_or(0),
            },
            Ok(Read::Held) => KeyEnd::Refused {
                kind: SUSPICIOUS_SHRINK.into(),
            },
            Err(ReferenceError::Cancelled) => KeyEnd::Cancelled { source: index_url },
            Err(ReferenceError::Superseded) => KeyEnd::Stale,
            Err(error) => KeyEnd::failed(error_kind(&error), index_url),
        }
    }

    /// Reads the file, then records its metadata on the subscription row the
    /// Settings view shows. A concurrent user edit of that row wins.
    async fn read_kind(
        &self,
        lease: &Lease,
        kind: &str,
        year: i32,
        index_url: &str,
        probes: &IndexProbes,
    ) -> Result<Read, ReferenceError> {
        let writer = lease.writer();
        let store = writer.store().clone();
        let workspace = store.load_workspace()?;
        let existing = subscription_row(&workspace, kind);
        let stored = store.write(None, |transaction| {
            StoredFile::read(transaction, &lease.key, kind, year)
        })?;
        let mut row = existing.cloned().unwrap_or_else(|| {
            json!({
                "id":format!("subscription:public:{kind}"),"kind":kind,"enabled":true
            })
        });
        let revision = row["rev"].as_u64().unwrap_or(0);
        row["url"] = json!(index_url);
        // Validators live in `dataset_state`.
        if let Some(object) = row.as_object_mut() {
            object.remove("indexHash");
            object.remove("remoteHash");
        }
        let result = self
            .read_file(lease, kind, year, index_url, probes, &stored)
            .await;
        if lease.is_cancelled() {
            return Err(ReferenceError::Cancelled);
        }
        let result = match result {
            // A held version changes nothing the Settings view shows.
            Ok(Read::Held) => return Ok(Read::Held),
            Err(ReferenceError::Superseded) => return Err(ReferenceError::Superseded),
            Ok(Read::Current(value)) => {
                row["year"] = json!(year);
                for key in ["dataUrl", "sourceTs"] {
                    if let Some(value) = value.get(key) {
                        row[key] = value.clone();
                    }
                }
                row["fetchedAt"] = json!((self.clock)().to_rfc3339());
                row["lastError"] = Value::Null;
                Ok(Read::Current(value))
            }
            Err(error) => {
                row["lastError"] = json!(error.to_string());
                Err(error)
            }
        };
        let Some(_gate) = writer.enter() else {
            return Err(ReferenceError::Superseded);
        };
        let latest = store
            .user_rows("subscriptions")?
            .into_iter()
            .find(|current| current["id"] == row["id"]);
        if latest
            .as_ref()
            .is_none_or(|current| current["rev"].as_u64().unwrap_or(0) == revision)
        {
            store.put_user_record_checked(
                "subscriptions",
                row,
                WorkspaceChangeSource::ReferencePull,
                Some(revision),
            )?;
        }
        if result.is_ok() {
            store.record_reference_poll()?;
        }
        result
    }

    /// Reads one Isct-Data file. A matching validator only renews freshness
    /// when the file's rows are stored; a fetched file replaces its rows and
    /// is marked fresh in one transaction, unless it shrank suspiciously.
    async fn read_file(
        &self,
        lease: &Lease,
        kind: &str,
        year: i32,
        index_url: &str,
        probes: &IndexProbes,
        stored: &StoredFile,
    ) -> Result<Read, ReferenceError> {
        let key = &lease.key;
        let writer = lease.writer();
        let write =
            |source: Option<WorkspaceChangeSource>,
             work: &mut dyn FnMut(&Transaction<'_>) -> Result<Read, ReferenceError>| {
                writer
                    .write(source, |transaction| work(transaction))
                    .map_err(|gated| match gated {
                        Gated::Stale => ReferenceError::Superseded,
                        Gated::Failed(error) => error,
                    })
            };
        let record = |connection: &Transaction<'_>,
                      outcome: RunOutcome,
                      changed: usize,
                      source: &str,
                      error_kind: Option<&str>| {
            sync_state::record_run(
                connection,
                &lease.run_record(outcome, changed, source, error_kind, (self.clock)()),
            )
        };
        let index_hash = probes.hash(self.transport.as_ref(), index_url).await?;
        if lease.is_cancelled() {
            return Err(ReferenceError::Cancelled);
        }
        let mut validator = stored.validator(index_url);
        if validator
            .as_ref()
            .is_some_and(|validator| validator.index_hash == index_hash)
        {
            let now = (self.clock)();
            let renewed = write(
                None,
                &mut |transaction: &Transaction<'_>| -> Result<Read, ReferenceError> {
                    if !sync_state::renew(transaction, key, "", now)? {
                        return Ok(Read::Held);
                    }
                    // The published version is the stored one again.
                    sync_state::set_cursor(transaction, key, "", None)?;
                    record(transaction, RunOutcome::Unchanged, 0, index_url, None)?;
                    Ok(Read::Current(not_modified()))
                },
            )?;
            if let Read::Current(value) = renewed {
                return Ok(Read::Current(value));
            }
            // Nothing is stored after all: read the file in full.
            validator = None;
        }
        let known_hash = validator
            .as_ref()
            .map(|validator| validator.file_hash.as_str());
        let fetched = self
            .transport
            .fetch(kind, index_url, year, &index_hash, known_hash)
            .await?;
        if lease.is_cancelled() {
            return Err(ReferenceError::Cancelled);
        }
        let now = (self.clock)();
        let mut next = Validator {
            index_url: index_url.to_string(),
            index_hash,
            file_hash: fetched.remote_hash.clone(),
            rows: validator.as_ref().map_or(0, |validator| validator.rows),
        };
        let Some(payload) = fetched.value.as_ref() else {
            // Only the index changed; the stored file is still current.
            if known_hash.is_none() {
                return Err(ReferenceError::InvalidInput(
                    "The reference source returned no data.".into(),
                ));
            }
            let encoded = next.encode();
            let committed = write(
                None,
                &mut |transaction: &Transaction<'_>| -> Result<Read, ReferenceError> {
                    if next.rows > 0 && stored_rows(transaction, kind, year)? == 0 {
                        return Ok(Read::Held);
                    }
                    sync_state::set_cursor(transaction, key, "", None)?;
                    sync_state::commit(
                        transaction,
                        key,
                        Commit {
                            account: "",
                            validator: Some(&encoded),
                            outcome: Outcome::Complete,
                            message: None,
                        },
                        now,
                    )?;
                    record(
                        transaction,
                        RunOutcome::Unchanged,
                        0,
                        &fetched.data_url,
                        None,
                    )?;
                    Ok(Read::Current(Value::Null))
                },
            )?;
            if matches!(committed, Read::Held) {
                return Err(ReferenceError::Storage(
                    "Stored reference rows disappeared during the refresh.".into(),
                ));
            }
            return Ok(Read::Current(fetch_result(0, &fetched, None, true)));
        };
        next.rows = payload.rows();
        let encoded = next.encode();
        write(
            Some(WorkspaceChangeSource::ReferencePull),
            &mut |transaction: &Transaction<'_>| -> Result<Read, ReferenceError> {
                let pending = sync_state::cursor(transaction, key)?;
                if suspicious_shrink(stored_rows(transaction, kind, year)?, next.rows)
                    && pending.as_deref() != Some(fetched.remote_hash.as_str())
                {
                    // Publish only when a later read returns this same version.
                    sync_state::set_cursor(transaction, key, "", Some(&fetched.remote_hash))?;
                    sync_state::fail(transaction, key, "", SUSPICIOUS_SHRINK, now)?;
                    record(
                        transaction,
                        RunOutcome::Failed,
                        0,
                        &fetched.data_url,
                        Some(SUSPICIOUS_SHRINK),
                    )?;
                    return Ok(Read::Held);
                }
                let (changed, files) = payload.apply(transaction)?;
                sync_state::set_cursor(transaction, key, "", None)?;
                sync_state::commit(
                    transaction,
                    key,
                    Commit {
                        account: "",
                        validator: Some(&encoded),
                        outcome: Outcome::Complete,
                        message: None,
                    },
                    now,
                )?;
                record(
                    transaction,
                    RunOutcome::Complete,
                    changed,
                    &fetched.data_url,
                    None,
                )?;
                Ok(Read::Current(fetch_result(changed, &fetched, files, false)))
            },
        )
    }
}

/// How a read of one reference file ended, short of an error.
enum Read {
    /// The stored file is current: renewed, or replaced by the fetched one.
    Current(Value),
    /// The fetched version shrank suspiciously and was not published.
    Held,
}

/// What a stored reference file was checked against, kept as its
/// `dataset_state.validator`: the index it came from, that index's hash,
/// the file's own hash and how many rows the file had.
#[derive(Clone, Debug, PartialEq)]
struct Validator {
    index_url: String,
    index_hash: String,
    file_hash: String,
    rows: usize,
}

impl Validator {
    fn encode(&self) -> String {
        format!(
            "{}:{}:{}@{}",
            self.index_hash, self.file_hash, self.rows, self.index_url
        )
    }

    fn parse(raw: &str) -> Option<Self> {
        let (hashes, index_url) = raw.split_once('@')?;
        let mut parts = hashes.splitn(3, ':');
        Some(Self {
            index_hash: parts.next()?.into(),
            file_hash: parts.next()?.into(),
            rows: parts.next()?.parse().ok()?,
            index_url: index_url.into(),
        })
    }
}

/// What is stored for one reference key when a read starts.
struct StoredFile {
    state: Option<KeyState>,
    /// The file hash of a version held back as a suspicious shrink.
    held: Option<String>,
    /// Live rows the file published.
    rows: usize,
}

impl StoredFile {
    fn read(connection: &Connection, key: &Key, kind: &str, year: i32) -> Result<Self, DataError> {
        Ok(Self {
            state: sync_state::get(connection, key)?,
            held: sync_state::cursor(connection, key)?,
            rows: stored_rows(connection, kind, year)?,
        })
    }

    /// The validator, when it was taken from this index and the file's rows
    /// are still stored: missing content beats validators.
    fn validator(&self, index_url: &str) -> Option<Validator> {
        self.state
            .as_ref()?
            .validator
            .as_deref()
            .and_then(Validator::parse)
            .filter(|validator| {
                validator.index_url == index_url && (validator.rows == 0 || self.rows > 0)
            })
    }
}

pub(crate) fn reference_key(kind: &str, year: i32) -> Key {
    Key::new(Dataset::Reference, format!("{kind}/{year}"))
}

/// Live rows one reference file published.
fn stored_rows(connection: &Connection, kind: &str, year: i32) -> Result<usize, DataError> {
    let (table, prefixes): (&str, &[&str]) = match kind {
        "calendar" => ("academic_calendar", &["calendar"]),
        "important-dates" => ("important_dates", &["important-date"]),
        "exams" => ("exams", &["exam", "exam-week-class"]),
        "class-schedules" => ("class_schedules", &["class-schedule"]),
        _ => {
            return Err(DataError::InvalidInput(
                "unsupported subscription type".into(),
            ))
        }
    };
    let mut rows = 0;
    for prefix in prefixes {
        let count: i64 = connection
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM {table}
                     WHERE id LIKE ?1 AND missing_since IS NULL AND deleted_at IS NULL"
                ),
                [format!("{prefix}:{year}:%")],
                |row| row.get(0),
            )
            .map_err(DataError::from)?;
        rows += usize::try_from(count).unwrap_or(0);
    }
    Ok(rows)
}

/// Public data only: a version that loses 30% or more of the stored rows,
/// or all of them, is held until a later read returns the same version.
pub(crate) fn suspicious_shrink(stored: usize, candidate: usize) -> bool {
    stored > 0 && candidate.saturating_mul(10) <= stored.saturating_mul(7)
}

/// The short `error_kind` code of a failed read; never the message.
fn error_kind(error: &ReferenceError) -> &'static str {
    match error {
        ReferenceError::Network(_) => "network",
        ReferenceError::InvalidInput(_) | ReferenceError::Compatibility(_) => "parse",
        ReferenceError::Storage(_) | ReferenceError::Conflict(_) => "storage",
        ReferenceError::Cancelled => "cancelled",
        ReferenceError::Superseded => "superseded",
    }
}

/// References follow the term the person is viewing for every kind of run.
pub(crate) fn subscription_year(workspace: &Value) -> Option<i32> {
    workspace["selectedTerm"]
        .as_str()
        .and_then(|term| term.get(..4))
        .and_then(|year| year.parse::<i32>().ok())
        .filter(|year| *year > 0)
}

/// The settings view uses the same year selection as every reference refresh.
pub fn subscription_views(workspace: &Value) -> Value {
    let mut views = serde_json::Map::new();
    for kind in REFERENCE_KINDS {
        let row = subscription_row(workspace, kind);
        let year = subscription_year(workspace);
        let enabled = row.and_then(|row| row["enabled"].as_bool()).unwrap_or(true);
        let state = if !enabled {
            json!({"state":"off"})
        } else if year.is_none() {
            json!({"state":"no-year"})
        } else if let Some(error) = row
            .and_then(|row| row["lastError"].as_str())
            .filter(|s| !s.is_empty())
        {
            json!({"state":"error", "message":error})
        } else if let Some(fetched) = row
            .filter(|row| row["year"].as_i64() == year.map(i64::from))
            .and_then(|row| row["fetchedAt"].as_str())
            .filter(|raw| timestamp(Some(raw)).is_some())
        {
            json!({"state":"current", "fetchedAt":fetched})
        } else {
            json!({"state":"pending"})
        };
        let url = row
            .and_then(|row| row["url"].as_str())
            .filter(|url| url.starts_with("https://"))
            .unwrap_or(PUBLIC_INDEX);
        views.insert(kind.into(), json!({"year":year,"enabled":enabled,"state":state,"url":url,"dataUrl":row.and_then(|row|row.get("dataUrl"))}));
    }
    Value::Object(views)
}

fn subscription_row<'a>(workspace: &'a Value, kind: &str) -> Option<&'a Value> {
    workspace["subscriptions"].as_array().and_then(|rows| {
        rows.iter().find(|row| {
            row["kind"].as_str() == Some(kind) && row.get("deletedAt").is_none_or(Value::is_null)
        })
    })
}

fn timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn not_modified() -> Value {
    json!({"changed":0,"skipped":true,"notModified":true})
}

fn fetch_result(
    changed: usize,
    fetched: &TransportFetch,
    files: Option<Vec<Value>>,
    not_modified: bool,
) -> Value {
    let mut value = json!({
        "changed":changed,
        "dataUrl":fetched.data_url,
        "remoteHash":fetched.remote_hash,
        "sourceTs":fetched.source_ts,
        "notModified":not_modified
    });
    if let Some(files) = files {
        value["files"] = json!(files);
    }
    value
}

#[cfg(test)]
#[path = "tests/reference_refresh.rs"]
mod tests;
