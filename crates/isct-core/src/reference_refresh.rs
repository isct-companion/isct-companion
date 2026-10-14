//! Reference subscription refresh policy, acquisition, and persistence.

use crate::{
    data::{versioned::content_hash, DataError, DataStore},
    events::{CoreEvent, WorkspaceChangeSource},
    ports::EventSink,
    pull_coordinator::{
        PullContext, PullCoordinator, PullError, PullOptions, PullPriority, PullTarget,
    },
    subscriptions::{
        self, AcademicCalendar, Departments, ImportantDates, PublishedExamFeed, ReferenceFetch,
        SubscriptionClient, SubscriptionError,
    },
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

const PUBLIC_INDEX: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
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
    #[error("Pull cancelled.")]
    Cancelled,
    #[error("{0}")]
    Coordination(#[source] PullError),
    #[error("{0} reference source(s) failed")]
    PartialFailure(usize),
}

impl From<DataError> for ReferenceError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
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
            ReferenceError::Storage(message) => Self::Storage(message),
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
    Departments(Departments),
    ImportantDates(ImportantDates),
    Exams(Vec<PublishedExamFeed>),
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
impl From<Vec<PublishedExamFeed>> for ReferencePayload {
    fn from(value: Vec<PublishedExamFeed>) -> Self {
        Self::Exams(value)
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
    events: Arc<dyn EventSink>,
    transport: Arc<dyn ReferenceTransport>,
    clock: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl ReferenceService {
    pub fn new(store: DataStore, coordinator: PullCoordinator, events: Arc<dyn EventSink>) -> Self {
        Self {
            store,
            coordinator,
            events,
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

    pub async fn refresh_all(&self, force: bool, selected: bool) -> Result<String, ReferenceError> {
        let workspace = self.store.load_workspace()?;
        let probes: Probes = Arc::new(Mutex::new(HashMap::new()));
        let mut changed = 0u64;
        let mut failures = 0usize;
        for kind in ["calendar", "departments", "important-dates", "exams"] {
            let row = subscription_row(&workspace, kind);
            if row.is_some_and(|row| row["enabled"].as_bool() == Some(false)) {
                continue;
            }
            let Some(year) = subscription_year(&workspace, kind, selected) else {
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
        let _ = self.events.emit(CoreEvent::WorkspaceChanged {
            source: WorkspaceChangeSource::ReferencePoll,
        });
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
        if !["calendar", "departments", "important-dates", "exams"].contains(&request.kind.as_str())
        {
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
        let same_source =
            existing.is_some_and(|row| same_index(row["url"].as_str(), &request.index_url));
        let has_cache = cache_present(&self.store, &workspace, request)?;
        if !request.force
            && priority != PullPriority::Manual
            && same_source
            && has_cache
            && existing
                .is_some_and(|row| reference_is_fresh(row, request.year.into(), (self.clock)()))
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
        let result = async {
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
            let same_year = same_source && row["year"].as_i64() == Some(request.year.into());
            let cached = same_year && has_cache && row["indexHash"].as_str() == Some(&index_hash);
            let value = if cached {
                not_modified()
            } else {
                let known_hash = (same_year && has_cache)
                    .then(|| row["remoteHash"].as_str())
                    .flatten();
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
                self.apply_fetch(fetched)?
            };
            if !context.applying().await {
                return Err(ReferenceError::Cancelled);
            }
            if request.kind != "calendar" && value["notModified"].as_bool() == Some(false) {
                let materialized = self.store.load_workspace()?;
                self.store.record_reference_cache(
                    &request.kind,
                    request.year,
                    json!({
                        "indexUrl":request.index_url,
                        "fingerprint":cache_fingerprint(&materialized, &request.kind, request.year)?
                    }),
                )?;
            }
            row["year"] = json!(request.year);
            row["indexHash"] = json!(index_hash);
            for key in ["dataUrl", "remoteHash", "sourceTs"] {
                if let Some(value) = value.get(key) {
                    row[key] = value.clone();
                }
            }
            row["fetchedAt"] = json!((self.clock)().to_rfc3339());
            row["lastError"] = Value::Null;
            Ok(value)
        }
        .await;
        if !context.applying().await {
            return Err(ReferenceError::Cancelled);
        }
        if let Err(error) = &result {
            row["lastError"] = json!(error.to_string());
        }
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
                "pull-coordinator",
                Some(revision),
            )?;
        }
        let _ = self.events.emit(CoreEvent::WorkspaceChanged {
            source: WorkspaceChangeSource::ReferencePull,
        });
        result
    }

    fn apply_fetch(&self, fetched: TransportFetch) -> Result<Value, ReferenceError> {
        let mut connection = rusqlite::Connection::open(self.store.path())
            .map_err(|error| ReferenceError::Storage(error.to_string()))?;
        let mut files = None;
        let changed = match fetched.value.as_ref() {
            None => return Ok(fetch_result(0, &fetched, None, true)),
            Some(ReferencePayload::Calendar(value)) => {
                subscriptions::apply_reference(&mut connection, Some(value), None)?
            }
            Some(ReferencePayload::Departments(value)) => {
                subscriptions::apply_reference(&mut connection, None, Some(value))?
            }
            Some(ReferencePayload::ImportantDates(value)) => {
                subscriptions::apply_important_dates(&mut connection, value)?
            }
            Some(ReferencePayload::Exams(feeds)) => {
                let mut changed = 0;
                let mut applied = Vec::new();
                for feed in feeds {
                    changed += subscriptions::apply_exam_feed(&mut connection, feed)?;
                    applied.push(json!({
                        "level":feed.level,
                        "quarter":feed.quarter.to_ascii_uppercase(),
                        "dataUrl":feed.url
                    }));
                }
                files = Some(applied);
                changed
            }
        };
        Ok(fetch_result(changed, &fetched, files, false))
    }
}

#[derive(Clone)]
struct ReferenceRequest {
    kind: String,
    index_url: String,
    year: i32,
    force: bool,
}

fn subscription_year(workspace: &Value, kind: &str, selected: bool) -> Option<i32> {
    let year = if kind == "departments" {
        workspace
            .pointer("/academicProfile/admissionYear")
            .and_then(Value::as_i64)
    } else {
        workspace[if selected {
            "selectedTerm"
        } else {
            "currentTerm"
        }]
        .as_str()
        .and_then(|term| term.get(..4))
        .and_then(|year| year.parse().ok())
    };
    year.and_then(|year| i32::try_from(year).ok())
        .filter(|year| *year > 0)
}

/// The settings view uses the same year selection as manual reference refresh.
pub fn subscription_views(workspace: &Value) -> Value {
    let mut views = serde_json::Map::new();
    for kind in ["calendar", "departments", "important-dates", "exams"] {
        let row = subscription_row(workspace, kind);
        let year = subscription_year(workspace, kind, true);
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

pub(crate) fn calendar_cache_present(workspace: &Value, year: i64) -> bool {
    let boundaries = workspace
        .get("academicCalendar")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let kind = row.get("type").and_then(Value::as_str)?;
            let quarter = row.get("quarter").and_then(Value::as_str)?;
            let raw_date = row.get("date").and_then(Value::as_str)?;
            let valid_date = NaiveDate::parse_from_str(raw_date, "%Y-%m-%d")
                .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == raw_date);
            (matches!(kind, "quarter-start" | "quarter-end")
                && valid_date
                && row.get("academicYear").and_then(Value::as_i64) == Some(year)
                && row.get("deletedAt").is_none_or(Value::is_null)
                && row.get("missingSince").is_none_or(Value::is_null))
            .then(|| format!("{kind}:{quarter}"))
        })
        .collect::<std::collections::HashSet<_>>();
    ["1Q", "2Q", "3Q", "4Q"].into_iter().all(|quarter| {
        boundaries.contains(&format!("quarter-start:{quarter}"))
            && boundaries.contains(&format!("quarter-end:{quarter}"))
    })
}

pub(crate) fn reference_is_fresh(row: &Value, year: i64, now: DateTime<Utc>) -> bool {
    row.get("year").and_then(Value::as_i64) == Some(year)
        && row.get("lastError").is_none_or(Value::is_null)
        && timestamp(row.get("fetchedAt").and_then(Value::as_str))
            .is_some_and(|last| now - last < Duration::hours(24))
}

/// The built-in reference task checks once per process launch before its
/// hourly recurrence becomes due.
pub fn reference_due_at_launch(task: &Value, started_at: DateTime<Utc>) -> bool {
    task.get("enabled").and_then(Value::as_bool) == Some(true)
        && task.get("id").and_then(Value::as_str) == Some("builtin:reference-subscriptions")
        && timestamp(task.get("lastRunAt").and_then(Value::as_str))
            .is_none_or(|last| last < started_at)
}

fn cache_fingerprint(workspace: &Value, kind: &str, year: i32) -> Result<String, ReferenceError> {
    let fields: &[&str] = match kind {
        "departments" => &["departments", "recommendedCourses"],
        "important-dates" => &["importantDates"],
        "exams" => &["exams"],
        _ => {
            return Err(ReferenceError::InvalidInput(
                "unsupported reference cache kind".into(),
            ))
        }
    };
    let year_field = if kind == "departments" {
        "cohortYear"
    } else {
        "academicYear"
    };
    let mut rows: Vec<_> = fields
        .iter()
        .flat_map(|field| {
            workspace[*field]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|row| {
                    row[year_field].as_i64() == Some(year.into())
                        && row.get("deletedAt").is_none_or(Value::is_null)
                        && row.get("missingSince").is_none_or(Value::is_null)
                })
                .map(move |row| json!({"collection":field,"record":row}))
        })
        .collect();
    rows.sort_by_key(|row| format!("{}:{}", row["collection"], row["record"]["id"]));
    content_hash(&json!(rows)).map_err(ReferenceError::from)
}

fn cache_present(
    store: &DataStore,
    workspace: &Value,
    request: &ReferenceRequest,
) -> Result<bool, ReferenceError> {
    if request.kind == "calendar" {
        return Ok(calendar_cache_present(workspace, request.year.into()));
    }
    let Some(marker) = store.reference_cache_marker(&request.kind, request.year)? else {
        return Ok(false);
    };
    Ok(marker["indexUrl"].as_str() == Some(&request.index_url)
        && marker["fingerprint"].as_str()
            == Some(&cache_fingerprint(workspace, &request.kind, request.year)?))
}

fn same_index(saved: Option<&str>, requested: &str) -> bool {
    saved
        .and_then(|saved| {
            let mut url = url::Url::parse(saved.trim()).ok()?;
            url.set_fragment(None);
            Some(url.to_string() == requested)
        })
        .unwrap_or(false)
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
mod tests {
    use super::*;
    use crate::{ports::EventError, subscriptions::DateRange};
    use std::{
        collections::{BTreeMap, VecDeque},
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex as StdMutex,
        },
    };
    use uuid::Uuid;

    const INDEX: &str = "https://example.test/index.json";

    #[derive(Default)]
    struct RecordingEvents(StdMutex<Vec<CoreEvent>>);

    impl EventSink for RecordingEvents {
        fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
            self.0.lock().unwrap().push(event);
            Ok(())
        }
    }

    struct FetchCall {
        kind: String,
        year: i32,
        known_hash: Option<String>,
    }

    enum FakeResponse {
        Ok(TransportFetch),
        Err(SubscriptionError),
    }

    struct FakeTransport {
        probes: AtomicUsize,
        calls: StdMutex<Vec<FetchCall>>,
        responses: StdMutex<HashMap<String, VecDeque<FakeResponse>>>,
    }

    impl Default for FakeTransport {
        fn default() -> Self {
            Self {
                probes: AtomicUsize::new(0),
                calls: StdMutex::new(Vec::new()),
                responses: StdMutex::new(HashMap::new()),
            }
        }
    }

    impl FakeTransport {
        fn respond(&self, kind: &str, responses: impl IntoIterator<Item = FakeResponse>) {
            self.responses
                .lock()
                .unwrap()
                .insert(kind.into(), responses.into_iter().collect());
        }

        fn calls(&self) -> Vec<(String, i32, Option<String>)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|call| (call.kind.clone(), call.year, call.known_hash.clone()))
                .collect()
        }
    }

    impl ReferenceTransport for FakeTransport {
        fn probe<'a>(&'a self, _index_url: &'a str) -> ReferenceTransportFuture<'a, String> {
            Box::pin(async move {
                self.probes.fetch_add(1, Ordering::Relaxed);
                Ok("index-v1".into())
            })
        }

        fn fetch<'a>(
            &'a self,
            kind: &'a str,
            _index_url: &'a str,
            year: i32,
            _expected_index_hash: &'a str,
            known_hash: Option<&'a str>,
        ) -> ReferenceTransportFuture<'a, TransportFetch> {
            Box::pin(async move {
                self.calls.lock().unwrap().push(FetchCall {
                    kind: kind.into(),
                    year,
                    known_hash: known_hash.map(str::to_owned),
                });
                match self
                    .responses
                    .lock()
                    .unwrap()
                    .get_mut(kind)
                    .and_then(VecDeque::pop_front)
                {
                    Some(FakeResponse::Ok(value)) => Ok(value),
                    Some(FakeResponse::Err(error)) => Err(error),
                    None => Ok(not_modified_fetch()),
                }
            })
        }
    }

    fn store(name: &str) -> DataStore {
        let path =
            std::env::temp_dir().join(format!("isct-reference-{name}-{}.db", Uuid::new_v4()));
        DataStore::initialize(path).unwrap().0
    }

    fn service(store: &DataStore, transport: Arc<dyn ReferenceTransport>) -> ReferenceService {
        ReferenceService::new(
            store.clone(),
            PullCoordinator::default(),
            Arc::new(RecordingEvents::default()),
        )
        .with_transport(transport)
        .with_clock(Arc::new(|| "2026-10-13T03:15:00Z".parse().unwrap()))
    }

    fn not_modified_fetch() -> TransportFetch {
        TransportFetch {
            value: None,
            data_url: "https://example.test/data.json".into(),
            remote_hash: "remote-v1".into(),
            source_ts: "2026-10-13T00:00:00Z".into(),
        }
    }

    fn calendar_fetch(year: i32) -> TransportFetch {
        let quarters = ["1Q", "2Q", "3Q", "4Q"]
            .into_iter()
            .map(|quarter| {
                (
                    quarter.into(),
                    DateRange {
                        start: format!("{year}-04-01"),
                        end: format!("{year}-05-01"),
                    },
                )
            })
            .collect();
        TransportFetch {
            value: Some(ReferencePayload::Calendar(AcademicCalendar {
                schema_version: 1,
                academic_year: year,
                source: Vec::new(),
                quarters,
                days: Vec::new(),
                teaching_ranges: BTreeMap::new(),
                exam_ranges: BTreeMap::new(),
            })),
            ..not_modified_fetch()
        }
    }

    fn put_subscription(store: &DataStore, kind: &str, enabled: bool, extra: Value) -> Value {
        let mut row = json!({
            "id":format!("subscription:public:{kind}"),
            "kind":kind,
            "enabled":enabled,
            "url":INDEX
        });
        if let (Some(row), Some(extra)) = (row.as_object_mut(), extra.as_object()) {
            row.extend(extra.clone());
        }
        store.put_user_record("subscriptions", row, "test").unwrap()
    }

    fn disable(store: &DataStore, kinds: &[&str]) {
        for kind in kinds {
            put_subscription(store, kind, false, json!({}));
        }
    }

    fn request(kind: &str, year: i32, force: bool) -> ReferenceRequest {
        ReferenceRequest {
            kind: kind.into(),
            index_url: INDEX.into(),
            year,
            force,
        }
    }

    fn probes() -> Probes {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[tokio::test]
    async fn batch_probes_a_shared_index_once_and_skips_disabled_sources() {
        let store = store("shared-probe");
        store
            .put_workspace_setting(
                "academicProfile",
                json!({"name":"Test","gradePath":[],"years":[],"admissionYear":2026}),
                "test",
            )
            .unwrap();
        put_subscription(&store, "calendar", true, json!({}));
        put_subscription(&store, "departments", true, json!({}));
        put_subscription(&store, "important-dates", false, json!({}));
        put_subscription(&store, "exams", true, json!({}));
        let transport = Arc::new(FakeTransport::default());
        service(&store, transport.clone())
            .refresh_all(true, true)
            .await
            .unwrap();

        assert_eq!(transport.probes.load(Ordering::Relaxed), 1);
        let calls = transport.calls();
        assert_eq!(calls.len(), 3);
        assert!(!calls.iter().any(|call| call.0 == "important-dates"));

        service(&store, transport.clone())
            .refresh_all(true, true)
            .await
            .unwrap();
        assert_eq!(transport.probes.load(Ordering::Relaxed), 2);
        assert_eq!(transport.calls().len(), 6);
    }

    #[tokio::test]
    async fn valid_cache_with_the_same_hash_skips_fetch_and_uses_completion_clock() {
        let store = store("same-hash");
        let setup = service(&store, Arc::new(FakeTransport::default()));
        setup.apply_fetch(calendar_fetch(2026)).unwrap();
        put_subscription(
            &store,
            "calendar",
            true,
            json!({
                "year":2026,"indexHash":"index-v1","remoteHash":"remote-v1",
                "fetchedAt":"2026-10-12T00:00:00Z"
            }),
        );
        let times = Arc::new(StdMutex::new(VecDeque::from([
            "2026-10-13T03:15:00Z".parse().unwrap(),
            "2026-10-13T03:16:00Z".parse().unwrap(),
        ])));
        let transport = Arc::new(FakeTransport::default());
        let clock = {
            let times = times.clone();
            Arc::new(move || times.lock().unwrap().pop_front().unwrap())
        };
        let result = ReferenceService::new(
            store.clone(),
            PullCoordinator::default(),
            Arc::new(RecordingEvents::default()),
        )
        .with_transport(transport.clone())
        .with_clock(clock)
        .refresh_one(
            request("calendar", 2026, false),
            PullPriority::Automatic,
            probes(),
        )
        .await
        .unwrap();

        assert_eq!(result["notModified"], true);
        assert!(transport.calls().is_empty());
        let row = subscription_row(&store.load_workspace().unwrap(), "calendar")
            .unwrap()
            .clone();
        assert_eq!(row["fetchedAt"], "2026-10-13T03:16:00+00:00");
    }

    #[tokio::test]
    async fn missing_quarter_and_changed_year_do_not_send_a_known_hash() {
        for (name, cached_year, requested_year, materialize) in [
            ("missing-quarter", 2026, 2026, false),
            ("changed-year", 2025, 2026, true),
        ] {
            let store = store(name);
            let transport = Arc::new(FakeTransport::default());
            let service = service(&store, transport.clone());
            if materialize {
                service.apply_fetch(calendar_fetch(cached_year)).unwrap();
            }
            put_subscription(
                &store,
                "calendar",
                true,
                json!({
                    "year":cached_year,"indexHash":"index-v1","remoteHash":"remote-v1",
                    "fetchedAt":"2026-10-12T00:00:00Z"
                }),
            );
            transport.respond(
                "calendar",
                [FakeResponse::Ok(calendar_fetch(requested_year))],
            );
            service
                .refresh_one(
                    request("calendar", requested_year, false),
                    PullPriority::Automatic,
                    probes(),
                )
                .await
                .unwrap();
            assert_eq!(transport.calls()[0].2, None, "{name}");
        }
    }

    #[tokio::test]
    async fn failed_metadata_is_cleared_by_a_later_retry() {
        let store = store("retry");
        put_subscription(
            &store,
            "calendar",
            true,
            json!({
                "year":2026,"indexHash":"old-index","remoteHash":"old-remote",
                "fetchedAt":"2026-10-13T03:14:00Z"
            }),
        );
        let transport = Arc::new(FakeTransport::default());
        let mut recovered = calendar_fetch(2026);
        recovered.remote_hash = "remote-v2".into();
        transport.respond(
            "calendar",
            [
                FakeResponse::Err(SubscriptionError::Network("offline".into())),
                FakeResponse::Ok(recovered),
            ],
        );
        let service = service(&store, transport);
        assert!(service
            .refresh_one(
                request("calendar", 2026, true),
                PullPriority::Manual,
                probes()
            )
            .await
            .is_err());
        let failed = subscription_row(&store.load_workspace().unwrap(), "calendar")
            .unwrap()
            .clone();
        assert_eq!(failed["lastError"], "offline");
        assert_eq!(failed["indexHash"], "old-index");
        assert_eq!(failed["remoteHash"], "old-remote");
        assert!(!calendar_cache_present(
            &store.load_workspace().unwrap(),
            2026
        ));

        service
            .refresh_one(
                request("calendar", 2026, true),
                PullPriority::Manual,
                probes(),
            )
            .await
            .unwrap();
        let retried = subscription_row(&store.load_workspace().unwrap(), "calendar")
            .unwrap()
            .clone();
        assert!(retried["lastError"].is_null());
        assert_eq!(retried["indexHash"], "index-v1");
        assert_eq!(retried["remoteHash"], "remote-v2");
        assert!(calendar_cache_present(
            &store.load_workspace().unwrap(),
            2026
        ));
    }

    struct EditingTransport {
        store: DataStore,
    }

    impl ReferenceTransport for EditingTransport {
        fn probe<'a>(&'a self, _index_url: &'a str) -> ReferenceTransportFuture<'a, String> {
            Box::pin(async { Ok("index-v1".into()) })
        }

        fn fetch<'a>(
            &'a self,
            _kind: &'a str,
            _index_url: &'a str,
            _year: i32,
            _expected_index_hash: &'a str,
            _known_hash: Option<&'a str>,
        ) -> ReferenceTransportFuture<'a, TransportFetch> {
            Box::pin(async move {
                let mut row = self.store.user_rows("subscriptions")?.remove(0);
                let revision = row["rev"].as_u64().unwrap();
                row["url"] = json!("https://user.example/new-index.json");
                self.store
                    .put_user_record_checked("subscriptions", row, "user", Some(revision))?;
                Ok(not_modified_fetch())
            })
        }
    }

    #[tokio::test]
    async fn user_revision_and_url_change_wins_over_inflight_metadata() {
        let store = store("user-wins");
        put_subscription(&store, "calendar", true, json!({}));
        let service = service(
            &store,
            Arc::new(EditingTransport {
                store: store.clone(),
            }),
        );
        service
            .refresh_one(
                request("calendar", 2026, true),
                PullPriority::Manual,
                probes(),
            )
            .await
            .unwrap();
        let workspace = store.load_workspace().unwrap();
        let row = subscription_row(&workspace, "calendar").unwrap();
        assert_eq!(row["url"], "https://user.example/new-index.json");
        assert!(row["fetchedAt"].is_null());
    }

    struct CancellingTransport {
        coordinator: PullCoordinator,
    }

    impl ReferenceTransport for CancellingTransport {
        fn probe<'a>(&'a self, _index_url: &'a str) -> ReferenceTransportFuture<'a, String> {
            Box::pin(async { Ok("index-v1".into()) })
        }

        fn fetch<'a>(
            &'a self,
            _kind: &'a str,
            _index_url: &'a str,
            year: i32,
            _expected_index_hash: &'a str,
            _known_hash: Option<&'a str>,
        ) -> ReferenceTransportFuture<'a, TransportFetch> {
            Box::pin(async move {
                let id = self.coordinator.snapshots().await[0].id.clone();
                assert!(self.coordinator.cancel(&id).await);
                Ok(calendar_fetch(year))
            })
        }
    }

    #[tokio::test]
    async fn cancellation_before_applying_writes_no_data_or_metadata() {
        let store = store("cancel");
        let coordinator = PullCoordinator::default();
        let service = ReferenceService::new(
            store.clone(),
            coordinator.clone(),
            Arc::new(RecordingEvents::default()),
        )
        .with_transport(Arc::new(CancellingTransport { coordinator }))
        .with_clock(Arc::new(|| "2026-10-13T03:15:00Z".parse().unwrap()));
        assert!(service
            .refresh_one(
                request("calendar", 2026, true),
                PullPriority::Manual,
                probes()
            )
            .await
            .is_err());
        let workspace = store.load_workspace().unwrap();
        assert!(workspace["academicCalendar"].as_array().unwrap().is_empty());
        assert!(subscription_row(&workspace, "calendar").is_none());
    }

    #[tokio::test]
    async fn batch_keeps_successful_data_when_another_source_fails() {
        let store = store("partial");
        disable(&store, &["departments", "exams"]);
        put_subscription(&store, "calendar", true, json!({}));
        put_subscription(&store, "important-dates", true, json!({}));
        let transport = Arc::new(FakeTransport::default());
        transport.respond("calendar", [FakeResponse::Ok(calendar_fetch(2026))]);
        transport.respond(
            "important-dates",
            [FakeResponse::Err(SubscriptionError::Network(
                "offline".into(),
            ))],
        );

        let error = service(&store, transport)
            .refresh_all(true, true)
            .await
            .unwrap_err();
        assert!(matches!(error, ReferenceError::PartialFailure(1)));
        let workspace = store.load_workspace().unwrap();
        assert!(calendar_cache_present(&workspace, 2026));
        assert!(subscription_row(&workspace, "calendar").unwrap()["lastError"].is_null());
        assert_eq!(
            subscription_row(&workspace, "important-dates").unwrap()["lastError"],
            "offline"
        );
    }

    #[test]
    fn freshness_has_a_strict_24_hour_boundary_and_rejects_non_iso_dates() {
        let now = "2026-10-13T03:15:00Z".parse().unwrap();
        assert!(reference_is_fresh(
            &json!({"year":2026,"fetchedAt":"2026-10-12T03:15:01Z"}),
            2026,
            now
        ));
        assert!(!reference_is_fresh(
            &json!({"year":2026,"fetchedAt":"2026-10-12T03:15:00Z"}),
            2026,
            now
        ));
        assert!(!reference_is_fresh(
            &json!({"year":2026,"fetchedAt":"2026-10-13 03:15:00"}),
            2026,
            now
        ));
        assert!(!reference_is_fresh(
            &json!({"year":2025,"fetchedAt":"2026-10-13T03:14:59Z"}),
            2026,
            now
        ));
        assert!(!reference_is_fresh(
            &json!({"year":2026,"fetchedAt":"2026-10-13T03:14:59Z","lastError":"offline"}),
            2026,
            now
        ));
    }

    #[test]
    fn restored_hashes_require_all_valid_quarter_boundaries() {
        let mut workspace = json!({"academicCalendar":[]});
        workspace["academicCalendar"] = Value::Array(
            ["1Q", "2Q", "3Q", "4Q"]
                .into_iter()
                .flat_map(|quarter| {
                    ["quarter-start", "quarter-end"]
                        .into_iter()
                        .map(move |kind| {
                            json!({
                                "type":kind,"quarter":quarter,"academicYear":2026,
                                "date":"2026-04-01"
                            })
                        })
                })
                .collect(),
        );
        assert!(calendar_cache_present(&workspace, 2026));
        workspace["academicCalendar"][0]["missingSince"] = json!("2026-10-13T00:00:00Z");
        assert!(!calendar_cache_present(&workspace, 2026));
        workspace["academicCalendar"][0]["missingSince"] = Value::Null;
        workspace["academicCalendar"][0]["date"] = json!("2026-02-30");
        assert!(!calendar_cache_present(&workspace, 2026));
    }

    #[test]
    fn restored_metadata_never_replaces_local_materialization_evidence() {
        let source = store("restored-metadata");
        for kind in ["departments", "important-dates", "exams"] {
            let request = request(kind, 2026, false);
            let workspace = source.load_workspace().unwrap();
            assert!(!cache_present(&source, &workspace, &request).unwrap());
            source
                .record_reference_cache(
                    kind,
                    2026,
                    json!({
                        "indexUrl":request.index_url,
                        "fingerprint":cache_fingerprint(&workspace, kind, 2026).unwrap()
                    }),
                )
                .unwrap();
            assert!(cache_present(&source, &workspace, &request).unwrap());
            let mut changed = workspace;
            let field = match kind {
                "departments" => "departments",
                "exams" => "exams",
                _ => "importantDates",
            };
            changed[field] = json!([{"id":"new-entry","cohortYear":2026,"academicYear":2026}]);
            assert!(!cache_present(&source, &changed, &request).unwrap());
        }
        let backup: Value = serde_json::from_str(&source.export_data(json!([])).unwrap()).unwrap();
        assert!(backup["tables"]["settings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| !row["key"].as_str().unwrap().starts_with("referenceCache:")));
        let restored = store("restored-import");
        restored.import_data(&backup).unwrap();
        assert!(restored
            .reference_cache_marker("exams", 2026)
            .unwrap()
            .is_none());
    }

    #[test]
    fn equivalent_index_urls_share_cache_identity() {
        assert!(same_index(
            Some(" HTTPS://EXAMPLE.COM:443/index.json#old "),
            "https://example.com/index.json"
        ));
        assert!(!same_index(
            Some("https://example.com/other.json"),
            "https://example.com/index.json"
        ));
        assert!(!same_index(None, "https://example.com/index.json"));
    }

    #[test]
    fn built_in_reference_task_runs_once_per_launch() {
        let started = "2026-10-13T03:15:00Z".parse().unwrap();
        let mut task = json!({
            "id":"builtin:reference-subscriptions",
            "enabled":true,
            "lastRunAt":"2026-10-12T03:00:00Z"
        });
        assert!(reference_due_at_launch(&task, started));
        task["lastRunAt"] = json!(started.to_rfc3339());
        assert!(!reference_due_at_launch(&task, started));
        task["id"] = json!("another-task");
        task["lastRunAt"] = Value::Null;
        assert!(!reference_due_at_launch(&task, started));
        task["id"] = json!("builtin:reference-subscriptions");
        task["enabled"] = json!(false);
        assert!(!reference_due_at_launch(&task, started));
    }

    #[test]
    fn reference_error_categories_survive_the_coordinator_boundary() {
        for (source, expected) in [
            (
                ReferenceError::Network("reference host unavailable".into()),
                PullError::Network("reference host unavailable".into()),
            ),
            (
                ReferenceError::Storage("reference cache unavailable".into()),
                PullError::Storage("reference cache unavailable".into()),
            ),
            (
                ReferenceError::InvalidInput("reference URL invalid".into()),
                PullError::InvalidInput("reference URL invalid".into()),
            ),
        ] {
            let pulled = PullError::from(source);
            assert_eq!(pulled, expected);
            match ReferenceError::from(pulled) {
                ReferenceError::Network(message) => {
                    assert_eq!(message, "reference host unavailable")
                }
                ReferenceError::Storage(message) => {
                    assert_eq!(message, "reference cache unavailable")
                }
                ReferenceError::InvalidInput(message) => {
                    assert_eq!(message, "reference URL invalid")
                }
                other => panic!("unexpected restored reference error: {other}"),
            }
        }
        assert_eq!(
            PullError::from(ReferenceError::Coordination(PullError::RuntimeUnavailable)),
            PullError::RuntimeUnavailable
        );
    }
}

#[cfg(test)]
mod subscription_view_tests {
    use super::*;
    #[test]
    fn follows_admission_and_selected_year_without_accepting_bad_fetch_dates() {
        let mut workspace = json!({"selectedTerm":"2026-3Q","currentTerm":"2025-4Q","academicProfile":{"admissionYear":2024},"subscriptions":[{"kind":"calendar","year":2026,"fetchedAt":"not-a-date"}]});
        let views = subscription_views(&workspace);
        assert_eq!(views["departments"]["year"], 2024);
        assert_eq!(views["calendar"]["year"], 2026);
        assert_eq!(views["calendar"]["state"]["state"], "pending");
        workspace["subscriptions"][0]["fetchedAt"] = json!("2026-10-06T12:00:00Z");
        assert_eq!(
            subscription_views(&workspace)["calendar"]["state"]["state"],
            "current"
        );
        workspace["subscriptions"][0]["year"] = json!(2025);
        assert_eq!(
            subscription_views(&workspace)["calendar"]["state"]["state"],
            "pending"
        );
        workspace["subscriptions"][0]["enabled"] = json!(false);
        assert_eq!(
            subscription_views(&workspace)["calendar"]["state"]["state"],
            "off"
        );
    }
}
