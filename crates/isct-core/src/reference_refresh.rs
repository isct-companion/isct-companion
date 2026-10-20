//! Reference subscription refresh policy, acquisition, and persistence.

use crate::{
    data::{DataError, DataStore, WorkspaceChangeSource},
    pull_coordinator::{
        PullContext, PullCoordinator, PullError, PullOptions, PullPriority, PullTarget,
    },
    subscriptions::{
        self, AcademicCalendar, ClassSchedules, ImportantDates, PublishedExamFeed, ReferenceFetch,
        SubscriptionClient, SubscriptionError,
    },
    sync_state::{self, Commit, Dataset, Due, Key, KeyState, Outcome, RunOutcome, RunRecord, Term},
};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, Transaction};
use serde_json::{json, Value};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

/// The public Isct-Data index every reference subscription starts from.
pub const PUBLIC_INDEX: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
/// Every reference dataset kind a subscription can follow.
pub const REFERENCE_KINDS: [&str; 4] = ["calendar", "important-dates", "exams", "class-schedules"];
/// `error_kind` of a public file held back because it shrank suspiciously.
const SUSPICIOUS_SHRINK: &str = "suspicious-shrink";
type ReferenceTransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SubscriptionError>> + Send + 'a>>;
type Probes = Arc<Mutex<HashMap<String, Result<String, ReferenceError>>>>;

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
    #[error("{0}")]
    Coordination(#[source] PullError),
    #[error("{0} reference source(s) failed")]
    PartialFailure(usize),
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

impl From<PullError> for ReferenceError {
    fn from(error: PullError) -> Self {
        match error {
            PullError::Cancelled => Self::Cancelled,
            PullError::InvalidTarget(message) | PullError::InvalidInput(message) => {
                Self::InvalidInput(message)
            }
            PullError::Network(message) => Self::Network(message),
            PullError::Storage(message) => Self::Storage(message),
            other => Self::Coordination(other),
        }
    }
}

impl From<ReferenceError> for PullError {
    fn from(error: ReferenceError) -> Self {
        match error {
            ReferenceError::InvalidInput(message) => Self::InvalidInput(message),
            ReferenceError::Network(message) => Self::Network(message),
            // The coordinator has no conflict/compatibility category; both
            // are local storage outcomes of the pull.
            ReferenceError::Storage(message)
            | ReferenceError::Conflict(message)
            | ReferenceError::Compatibility(message) => Self::Storage(message),
            ReferenceError::Cancelled => Self::Cancelled,
            ReferenceError::Coordination(error) => error,
            ReferenceError::PartialFailure(count) => {
                Self::Execution(format!("{count} reference source(s) failed"))
            }
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

#[derive(Clone)]
pub struct ReferenceService {
    store: DataStore,
    coordinator: PullCoordinator,
    transport: Arc<dyn ReferenceTransport>,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl ReferenceService {
    pub fn new(store: DataStore, coordinator: PullCoordinator) -> Self {
        Self {
            store,
            coordinator,
            transport: Arc::new(HttpReferenceTransport),
            clock: Arc::new(Utc::now),
        }
    }

    #[cfg(test)]
    fn with_transport(mut self, transport: Arc<dyn ReferenceTransport>) -> Self {
        self.transport = transport;
        self
    }

    #[cfg(test)]
    fn with_clock(mut self, clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    /// Automatic and manual runs use the same year rule as the Settings view.
    pub async fn refresh_all(&self, force: bool) -> Result<String, ReferenceError> {
        let workspace = self.store.load_workspace()?;
        let probes: Probes = Arc::new(Mutex::new(HashMap::new()));
        // Correlates this refresh's `sync_runs` rows.
        let run = uuid::Uuid::new_v4().to_string();
        let mut changed = 0u64;
        let mut failures = 0usize;
        for kind in REFERENCE_KINDS {
            let row = subscription_row(&workspace, kind);
            if row.is_some_and(|row| row["enabled"].as_bool() == Some(false)) {
                continue;
            }
            let Some(year) = subscription_year(&workspace) else {
                continue;
            };
            let index_url = row
                .and_then(|row| row["url"].as_str())
                .filter(|url| url.starts_with("https://"))
                .unwrap_or(PUBLIC_INDEX)
                .to_string();
            let request = ReferenceRequest {
                kind: kind.into(),
                index_url,
                year,
                force,
                run: run.clone(),
            };
            match self
                .refresh_one(
                    request,
                    if force {
                        PullPriority::Manual
                    } else {
                        PullPriority::Automatic
                    },
                    probes.clone(),
                )
                .await
            {
                Ok(result) => changed += result["changed"].as_u64().unwrap_or(0),
                Err(_) => failures += 1,
            }
        }
        self.store.record_reference_poll()?;
        if failures > 0 {
            Err(ReferenceError::PartialFailure(failures))
        } else if changed > 0 {
            Ok(format!("Updated {changed} reference dataset(s)"))
        } else {
            Ok("Reference data is up to date".into())
        }
    }

    async fn refresh_one(
        &self,
        mut request: ReferenceRequest,
        priority: PullPriority,
        probes: Probes,
    ) -> Result<Value, ReferenceError> {
        if let PullTarget::Reference {
            kind, index_url, ..
        } = (PullTarget::Reference {
            kind: request.kind.clone(),
            index_url: request.index_url.clone(),
            year: request.year,
        })
        .normalized()?
        {
            request.kind = kind;
            request.index_url = index_url;
        }
        let service = self.clone();
        let target = PullTarget::Reference {
            kind: request.kind.clone(),
            index_url: request.index_url.clone(),
            year: request.year,
        };
        let mut options = if priority == PullPriority::Manual {
            PullOptions::manual()
        } else {
            PullOptions::automatic()
        };
        options.priority = priority;
        self.coordinator
            .run(target, options, move |context| {
                let service = service.clone();
                let request = request.clone();
                let probes = probes.clone();
                async move {
                    service
                        .run_reference(&request, &context, &probes)
                        .await
                        .map_err(PullError::from)
                }
            })
            .await
            .map_err(ReferenceError::from)
    }

    async fn run_reference(
        &self,
        request: &ReferenceRequest,
        context: &PullContext,
        probes: &Probes,
    ) -> Result<Value, ReferenceError> {
        if !REFERENCE_KINDS.contains(&request.kind.as_str()) {
            return Err(ReferenceError::InvalidInput(
                "unsupported subscription type".into(),
            ));
        }
        let workspace = self.store.load_workspace()?;
        let existing = subscription_row(&workspace, &request.kind);
        let priority = context.priority();
        if priority == PullPriority::Automatic
            && existing.is_some_and(|row| row["enabled"].as_bool() == Some(false))
        {
            return Ok(not_modified());
        }
        let key = reference_key(&request.kind, request.year);
        let started = (self.clock)();
        let stored = self.store.write(None, |transaction| {
            StoredFile::read(transaction, &key, &request.kind, request.year)
        })?;
        if !request.force
            && priority != PullPriority::Manual
            && !stored.due(&request.index_url, started)
        {
            return Ok(not_modified());
        }
        let mut row = existing.cloned().unwrap_or_else(|| {
            json!({
                "id":format!("subscription:public:{}",request.kind),"kind":request.kind,"enabled":true
            })
        });
        let revision = row["rev"].as_u64().unwrap_or(0);
        row["url"] = json!(request.index_url);
        // Validators live in `dataset_state`.
        if let Some(object) = row.as_object_mut() {
            object.remove("indexHash");
            object.remove("remoteHash");
        }
        let run = Run {
            id: &request.run,
            key: &key,
            started,
        };
        let result = self
            .read_file(request, context, probes, &run, &stored)
            .await;
        if let Err(error) = &result {
            // Nothing of the file was written; push its next attempt back.
            let kind = error_kind(error);
            let outcome = if matches!(error, ReferenceError::Cancelled) {
                RunOutcome::Cancelled
            } else {
                RunOutcome::Failed
            };
            let finished = (self.clock)();
            self.store.write(None, |transaction| {
                sync_state::fail(transaction, &key, "", kind, finished)?;
                run.record(
                    transaction,
                    finished,
                    outcome,
                    0,
                    &request.index_url,
                    Some(kind),
                )
            })?;
        }
        if !context.applying().await {
            return Err(ReferenceError::Cancelled);
        }
        let result = match result {
            // A held version changes nothing the Settings view shows.
            Ok(Read::Held) => return Ok(held()),
            Ok(Read::Current(value)) => {
                row["year"] = json!(request.year);
                for key in ["dataUrl", "sourceTs"] {
                    if let Some(value) = value.get(key) {
                        row[key] = value.clone();
                    }
                }
                row["fetchedAt"] = json!((self.clock)().to_rfc3339());
                row["lastError"] = Value::Null;
                Ok(value)
            }
            Err(error) => {
                row["lastError"] = json!(error.to_string());
                Err(error)
            }
        };
        let latest = self
            .store
            .user_rows("subscriptions")?
            .into_iter()
            .find(|current| current["id"] == row["id"]);
        if latest
            .as_ref()
            .is_none_or(|current| current["rev"].as_u64().unwrap_or(0) == revision)
        {
            self.store.put_user_record_checked(
                "subscriptions",
                row,
                WorkspaceChangeSource::ReferencePull,
                Some(revision),
            )?;
        }
        result
    }

    /// Reads one Isct-Data file. A matching validator only renews freshness
    /// when the file's rows are stored; a fetched file replaces its rows and
    /// is marked fresh in one transaction, unless it shrank suspiciously.
    async fn read_file(
        &self,
        request: &ReferenceRequest,
        context: &PullContext,
        probes: &Probes,
        run: &Run<'_>,
        stored: &StoredFile,
    ) -> Result<Read, ReferenceError> {
        let key = run.key;
        if !context.phase("index").await {
            return Err(ReferenceError::Cancelled);
        }
        let index_hash = {
            let mut cache = probes.lock().await;
            if let Some(result) = cache.get(&request.index_url) {
                result.clone()?
            } else {
                let result = self
                    .transport
                    .probe(&request.index_url)
                    .await
                    .map_err(ReferenceError::from);
                cache.insert(request.index_url.clone(), result.clone());
                result?
            }
        };
        let mut validator = stored.validator(&request.index_url);
        if validator
            .as_ref()
            .is_some_and(|validator| validator.index_hash == index_hash)
        {
            if !context.applying().await {
                return Err(ReferenceError::Cancelled);
            }
            let now = (self.clock)();
            let renewed = self.store.write(None, |transaction| {
                if !sync_state::renew(transaction, key, "", now)? {
                    return Ok::<_, DataError>(false);
                }
                // The published version is the stored one again.
                sync_state::set_cursor(transaction, key, "", None)?;
                run.record(
                    transaction,
                    now,
                    RunOutcome::Unchanged,
                    0,
                    &request.index_url,
                    None,
                )?;
                Ok(true)
            })?;
            if renewed {
                return Ok(Read::Current(not_modified()));
            }
            // Nothing is stored after all: read the file in full.
            if !context.applied().await {
                return Err(ReferenceError::Cancelled);
            }
            validator = None;
        }
        let known_hash = validator
            .as_ref()
            .map(|validator| validator.file_hash.as_str());
        if !context.phase("fetching").await {
            return Err(ReferenceError::Cancelled);
        }
        let fetched = self
            .transport
            .fetch(
                &request.kind,
                &request.index_url,
                request.year,
                &index_hash,
                known_hash,
            )
            .await?;
        if context.is_cancelled() || !context.applying().await {
            return Err(ReferenceError::Cancelled);
        }
        let now = (self.clock)();
        let mut next = Validator {
            index_url: request.index_url.clone(),
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
            let committed = self.store.write(None, |transaction| {
                if next.rows > 0 && stored_rows(transaction, &request.kind, request.year)? == 0 {
                    return Ok::<_, DataError>(false);
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
                run.record(
                    transaction,
                    now,
                    RunOutcome::Unchanged,
                    0,
                    &fetched.data_url,
                    None,
                )?;
                Ok(true)
            })?;
            if !committed {
                return Err(ReferenceError::Storage(
                    "Stored reference rows disappeared during the refresh.".into(),
                ));
            }
            return Ok(Read::Current(fetch_result(0, &fetched, None, true)));
        };
        next.rows = payload.rows();
        let encoded = next.encode();
        self.store.write(
            Some(WorkspaceChangeSource::ReferencePull),
            |transaction| -> Result<Read, ReferenceError> {
                let pending = sync_state::cursor(transaction, key)?;
                if suspicious_shrink(
                    stored_rows(transaction, &request.kind, request.year)?,
                    next.rows,
                ) && pending.as_deref() != Some(fetched.remote_hash.as_str())
                {
                    // Publish only when a later read returns this same version.
                    sync_state::set_cursor(transaction, key, "", Some(&fetched.remote_hash))?;
                    sync_state::fail(transaction, key, "", SUSPICIOUS_SHRINK, now)?;
                    run.record(
                        transaction,
                        now,
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
                run.record(
                    transaction,
                    now,
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

#[derive(Clone)]
struct ReferenceRequest {
    kind: String,
    index_url: String,
    year: i32,
    force: bool,
    /// Correlates the `sync_runs` rows of one refresh.
    run: String,
}

/// How a read of one reference file ended, short of an error.
enum Read {
    /// The stored file is current: renewed, or replaced by the fetched one.
    Current(Value),
    /// The fetched version shrank suspiciously and was not published.
    Held,
}

/// One read of a reference key, for its `sync_runs` row.
struct Run<'a> {
    id: &'a str,
    key: &'a Key,
    started: DateTime<Utc>,
}

impl Run<'_> {
    fn record(
        &self,
        connection: &Connection,
        finished: DateTime<Utc>,
        outcome: RunOutcome,
        changed: usize,
        source: &str,
        error_kind: Option<&str>,
    ) -> Result<(), DataError> {
        sync_state::record_run(
            connection,
            &RunRecord {
                run: self.id,
                key: self.key,
                account: "",
                started_at: self.started,
                finished_at: finished,
                outcome,
                changed,
                requests: None,
                bytes: None,
                source,
                error_kind,
            },
        )
    }
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

    /// Whether an automatic run reads the file now.
    fn due(&self, index_url: &str, now: DateTime<Utc>) -> bool {
        let ttl = sync_state::ttl(Dataset::Reference, Term::Current);
        match sync_state::due(self.state.as_ref(), ttl, "", now) {
            Due::BackingOff(_) => false,
            // A held version is read again once its backoff has passed.
            Due::Fresh => self.held.is_some() || self.validator(index_url).is_none(),
            Due::Missing | Due::Stale => true,
        }
    }
}

fn reference_key(kind: &str, year: i32) -> Key {
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
fn suspicious_shrink(stored: usize, candidate: usize) -> bool {
    stored > 0 && candidate.saturating_mul(10) <= stored.saturating_mul(7)
}

/// The short `error_kind` code of a failed read; never the message.
fn error_kind(error: &ReferenceError) -> &'static str {
    match error {
        ReferenceError::Network(_) => "network",
        ReferenceError::InvalidInput(_) | ReferenceError::Compatibility(_) => "parse",
        ReferenceError::Storage(_) | ReferenceError::Conflict(_) => "storage",
        ReferenceError::Cancelled => "cancelled",
        ReferenceError::Coordination(_) | ReferenceError::PartialFailure(_) => "coordination",
    }
}

/// References follow the term the person is viewing for every kind of run.
fn subscription_year(workspace: &Value) -> Option<i32> {
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

/// The built-in reference task checks once per process launch before its
/// hourly recurrence becomes due.
pub fn reference_due_at_launch(task: &Value, started_at: DateTime<Utc>) -> bool {
    task.get("enabled").and_then(Value::as_bool) == Some(true)
        && task.get("id").and_then(Value::as_str) == Some("builtin:reference-subscriptions")
        && timestamp(task.get("lastRunAt").and_then(Value::as_str))
            .is_none_or(|last| last < started_at)
}

fn not_modified() -> Value {
    json!({"changed":0,"skipped":true,"notModified":true})
}

fn held() -> Value {
    json!({"changed":0,"skipped":true,"notModified":true,"held":true})
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
