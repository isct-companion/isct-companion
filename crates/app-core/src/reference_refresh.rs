//! Reference files: what the person follows, when a file is due, and the
//! executor that reads one leased reference key: `{kind}/{year}` for a file
//! chosen by a year, `campus-map/v1` for the campus map.

use crate::{
    data::{DataError, WorkspaceChangeSource},
    plan::{Gated, KeyEnd, Lease},
    subscriptions::{
        self, AcademicCalendar, ClassSchedules, Departments, ImportantDates, PublishedExamFeed,
        ReferenceFetch, SubscriptionClient, SubscriptionError,
    },
    sync_state::{self, Commit, Dataset, Due, Key, KeyState, Outcome, RunOutcome, Term},
};
#[cfg(feature = "campus-map")]
use crate::campus_map::CampusMapFile;
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
pub const REFERENCE_KINDS: [&str; 5] = [
    "calendar",
    "departments",
    "important-dates",
    "exams",
    "class-schedules",
];
/// How long a cohort's study guide (`departments`) stays fresh. It is
/// published once per admission year and almost never corrected.
const DEPARTMENTS_TTL: chrono::Duration = chrono::Duration::days(365);
/// `error_kind` of a public file held back because it shrank suspiciously.
pub(crate) const SUSPICIOUS_SHRINK: &str = "suspicious-shrink";
/// The campus map's kind. It is no subscription: Core always follows it
/// from the public index, and it is not among [`REFERENCE_KINDS`].
#[cfg(feature = "campus-map")]
pub(crate) const CAMPUS_MAP_KIND: &str = "campus-map";

/// What a reference file is chosen by: the second part of its key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileScope {
    /// The academic year of the term being viewed.
    AcademicYear(i32),
    /// The person's admission year: the study guide.
    AdmissionYear(i32),
    /// The whole campus, one file per format version (`v1`).
    #[cfg(feature = "campus-map")]
    Campus,
}

impl FileScope {
    /// The key segment: the year, or the format version.
    fn segment(self) -> String {
        match self {
            Self::AcademicYear(year) | Self::AdmissionYear(year) => year.to_string(),
            #[cfg(feature = "campus-map")]
            Self::Campus => format!("v{}", crate::campus_map::FORMAT_VERSION),
        }
    }

    fn year(self) -> Option<i32> {
        match self {
            Self::AcademicYear(year) | Self::AdmissionYear(year) => Some(year),
            #[cfg(feature = "campus-map")]
            Self::Campus => None,
        }
    }
}

/// The scope of a kind chosen by a year: the study guide by the admission
/// year, everything else by the academic year.
pub(crate) fn year_scope(kind: &str, year: i32) -> FileScope {
    if kind == "departments" {
        FileScope::AdmissionYear(year)
    } else {
        FileScope::AcademicYear(year)
    }
}

/// A reference key's kind and scope; `None` for a key no reader knows.
fn parse_key(key: &str) -> Option<(String, FileScope)> {
    let (kind, segment) = key.rsplit_once('/')?;
    #[cfg(feature = "campus-map")]
    if kind == CAMPUS_MAP_KIND {
        return (segment == FileScope::Campus.segment())
            .then(|| (kind.to_string(), FileScope::Campus));
    }
    let year = segment.parse::<i32>().ok()?;
    REFERENCE_KINDS
        .contains(&kind)
        .then(|| (kind.to_string(), year_scope(kind, year)))
}

/// Reference keys read ahead of other background work and on cellular too:
/// the campus map, which the map buttons wait for.
pub(crate) fn raised(key: &Key) -> bool {
    #[cfg(feature = "campus-map")]
    {
        *key == reference_key(CAMPUS_MAP_KIND, FileScope::Campus)
    }
    #[cfg(not(feature = "campus-map"))]
    {
        let _ = key;
        false
    }
}
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
            DataError::Open(error) => Self::Storage(error.to_string()),
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
    Departments(Departments),
    ImportantDates(ImportantDates),
    Exams(Vec<PublishedExamFeed>),
    ClassSchedules(ClassSchedules),
    #[cfg(feature = "campus-map")]
    CampusMap(CampusMapFile),
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
impl From<Departments> for ReferencePayload {
    fn from(value: Departments) -> Self {
        Self::Departments(value)
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
#[cfg(feature = "campus-map")]
impl From<CampusMapFile> for ReferencePayload {
    fn from(value: CampusMapFile) -> Self {
        Self::CampusMap(value)
    }
}

impl ReferencePayload {
    /// Rows this version publishes.
    fn rows(&self) -> usize {
        match self {
            Self::Calendar(calendar) => calendar.quarters.len() * 2 + calendar.days.len(),
            Self::Departments(guide) => guide
                .departments
                .iter()
                .map(|department| 1 + department.recommended.len())
                .sum(),
            Self::ImportantDates(dates) => dates.entries.len(),
            Self::Exams(feeds) => feeds.iter().map(|feed| feed.feed.entries.len()).sum(),
            Self::ClassSchedules(schedules) => schedules.entries.len(),
            #[cfg(feature = "campus-map")]
            Self::CampusMap(file) => file.units,
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
            Self::Departments(value) => (
                subscriptions::apply_reference(transaction, None, Some(value))?,
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
            #[cfg(feature = "campus-map")]
            Self::CampusMap(file) => (
                crate::data::put_campus_map(transaction, &file.hash, &file.gz, file.units)?,
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
        scope: FileScope,
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
        scope: FileScope,
        expected_index_hash: &'a str,
        known_hash: Option<&'a str>,
    ) -> ReferenceTransportFuture<'a, TransportFetch> {
        Box::pin(async move {
            let client = SubscriptionClient::new()?;
            #[cfg(feature = "campus-map")]
            if scope == FileScope::Campus {
                return Ok(client
                    .fetch_campus_map(index_url, Some(expected_index_hash), known_hash)
                    .await?
                    .into());
            }
            let year = scope.year().ok_or_else(|| {
                SubscriptionError::Validation("this file is not chosen by a year".into())
            })?;
            match kind {
                "calendar" => Ok(client
                    .fetch_calendar(index_url, year, Some(expected_index_hash), known_hash)
                    .await?
                    .into()),
                "departments" => Ok(client
                    .fetch_departments(index_url, year, Some(expected_index_hash), known_hash)
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

/// The reference files the person follows: every enabled kind for its
/// year (`subscription_year`), from its subscription's index; and the campus
/// map, always, from the public index.
pub(crate) fn followed(workspace: &Value) -> Vec<(String, FileScope, String)> {
    let followed = REFERENCE_KINDS.iter().filter_map(|kind| {
        let year = subscription_year(workspace, kind)?;
        let row = subscription_row(workspace, kind);
        if row.is_some_and(|row| row["enabled"].as_bool() == Some(false)) {
            return None;
        }
        let index_url = row
            .and_then(|row| row["url"].as_str())
            .and_then(normalized_index)
            .unwrap_or_else(|| PUBLIC_INDEX.to_string());
        Some(((*kind).to_string(), year_scope(kind, year), index_url))
    });
    #[cfg(feature = "campus-map")]
    let followed = followed.chain(std::iter::once((
        CAMPUS_MAP_KIND.to_string(),
        FileScope::Campus,
        PUBLIC_INDEX.to_string(),
    )));
    followed.collect()
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
/// Whether a reference key is due, and when a fresh one goes stale.
pub(crate) fn reference_due(
    connection: &Connection,
    key: &Key,
    kind: &str,
    scope: FileScope,
    index_url: &str,
    now: DateTime<Utc>,
) -> Result<(Due, Option<DateTime<Utc>>), DataError> {
    let stored = StoredFile::read(connection, key, kind, scope)?;
    let ttl = if kind == "departments" {
        Some(DEPARTMENTS_TTL)
    } else {
        sync_state::ttl(Dataset::Reference, Term::Current)
    };
    let due = match sync_state::due(stored.state.as_ref(), ttl, now) {
        Due::Fresh if stored.held.is_some() || stored.validator(index_url).is_none() => Due::Stale,
        due => due,
    };
    let stale_at = (due == Due::Fresh)
        .then(|| {
            stored
                .state
                .as_ref()
                .and_then(|state| state.fetched_at)
                .zip(ttl)
                .map(|(fetched, ttl)| fetched + ttl)
        })
        .flatten();
    Ok((due, stale_at))
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
        let Some((kind, scope)) = parse_key(&lease.key.key) else {
            return KeyEnd::failed("unsupported", index_url);
        };
        let read = match scope {
            FileScope::AcademicYear(year) | FileScope::AdmissionYear(year) => {
                self.read_kind(lease, &kind, scope, year, &index_url, probes)
                    .await
            }
            #[cfg(feature = "campus-map")]
            FileScope::Campus => self.read_public(lease, &kind, scope, &index_url, probes).await,
        };
        match read {
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
        scope: FileScope,
        year: i32,
        index_url: &str,
        probes: &IndexProbes,
    ) -> Result<Read, ReferenceError> {
        let writer = lease.writer();
        let store = writer.store().clone();
        let workspace = store.load_fields(&["subscriptions"])?;
        let existing = subscription_row(&workspace, kind);
        let stored = store.write(None, |transaction| {
            StoredFile::read(transaction, &lease.key, kind, scope)
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
            .read_file(lease, kind, scope, index_url, probes, &stored)
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

    /// Reads a file no subscription row describes (the campus map): only
    /// the file and its freshness.
    #[cfg(feature = "campus-map")]
    async fn read_public(
        &self,
        lease: &Lease,
        kind: &str,
        scope: FileScope,
        index_url: &str,
        probes: &IndexProbes,
    ) -> Result<Read, ReferenceError> {
        let stored = lease.writer().store().write(None, |transaction| {
            StoredFile::read(transaction, &lease.key, kind, scope)
        })?;
        let result = self
            .read_file(lease, kind, scope, index_url, probes, &stored)
            .await;
        if lease.is_cancelled() {
            return Err(ReferenceError::Cancelled);
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
        scope: FileScope,
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
                    if !sync_state::renew(transaction, key, now)? {
                        return Ok(Read::Held);
                    }
                    // The published version is the stored one again.
                    sync_state::set_cursor(transaction, key, None)?;
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
            .fetch(kind, index_url, scope, &index_hash, known_hash)
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
                    if next.rows > 0 && stored_rows(transaction, kind, scope)? == 0 {
                        return Ok(Read::Held);
                    }
                    sync_state::set_cursor(transaction, key, None)?;
                    sync_state::commit(
                        transaction,
                        key,
                        Commit {
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
                if suspicious_shrink(stored_rows(transaction, kind, scope)?, next.rows)
                    && pending.as_deref() != Some(fetched.remote_hash.as_str())
                {
                    // Publish only when a later read returns this same version.
                    sync_state::set_cursor(transaction, key, Some(&fetched.remote_hash))?;
                    sync_state::fail(transaction, key, SUSPICIOUS_SHRINK, now)?;
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
                sync_state::set_cursor(transaction, key, None)?;
                sync_state::commit(
                    transaction,
                    key,
                    Commit {
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
    fn read(
        connection: &Connection,
        key: &Key,
        kind: &str,
        scope: FileScope,
    ) -> Result<Self, DataError> {
        Ok(Self {
            state: sync_state::get(connection, key)?,
            held: sync_state::cursor(connection, key)?,
            rows: stored_rows(connection, kind, scope)?,
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

pub(crate) fn reference_key(kind: &str, scope: FileScope) -> Key {
    Key::new(
        Dataset::Reference,
        format!("{kind}/{}", scope.segment()),
    )
}

/// Live rows one reference file published; the campus map counts its units.
fn stored_rows(connection: &Connection, kind: &str, scope: FileScope) -> Result<usize, DataError> {
    let year = match scope {
        FileScope::AcademicYear(year) | FileScope::AdmissionYear(year) => year,
        #[cfg(feature = "campus-map")]
        FileScope::Campus => return crate::data::campus_map_units(connection),
    };
    let sources: &[(&str, &str)] = match kind {
        "calendar" => &[("academic_calendar", "calendar")],
        "departments" => &[("departments", "cohort"), ("recommended_courses", "cohort")],
        "important-dates" => &[("important_dates", "important-date")],
        "exams" => &[("exams", "exam"), ("exams", "exam-week-class")],
        "class-schedules" => &[("class_schedules", "class-schedule")],
        _ => {
            return Err(DataError::InvalidInput(
                "unsupported subscription type".into(),
            ))
        }
    };
    let mut rows = 0;
    for (table, prefix) in sources {
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

/// The year a kind's file is chosen by: the admission year for the study
/// guide (`departments`), never falling back to another cohort's
/// (`constraint/subscription-cohort-axes`); the academic year of the term
/// being viewed for everything else.
pub(crate) fn subscription_year(workspace: &Value, kind: &str) -> Option<i32> {
    if kind == "departments" {
        return workspace
            .pointer("/academicProfile/admissionYear")
            .and_then(Value::as_i64)
            .and_then(|year| i32::try_from(year).ok())
            .filter(|year| *year > 0);
    }
    academic_year(workspace)
}

/// The academic year of the term being viewed.
pub(crate) fn academic_year(workspace: &Value) -> Option<i32> {
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
        let year = subscription_year(workspace, kind);
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
