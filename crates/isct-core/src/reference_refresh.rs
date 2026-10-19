//! Reference subscription refresh policy, acquisition, and persistence.

use crate::{
    data::{versioned::content_hash, DataError, DataStore, WorkspaceChangeSource},
    pull_coordinator::{
        PullContext, PullCoordinator, PullError, PullOptions, PullPriority, PullTarget,
    },
    subscriptions::{
        self, AcademicCalendar, ClassSchedules, ImportantDates, PublishedExamFeed, ReferenceFetch,
        SubscriptionClient, SubscriptionError,
    },
};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{json, Value};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

/// The public Isct-Data index every reference subscription starts from.
pub const PUBLIC_INDEX: &str =
    "https://raw.githubusercontent.com/isct-companion/Isct-Data/main/index.json";
/// Every reference dataset kind a subscription can follow.
pub const REFERENCE_KINDS: [&str; 4] = ["calendar", "important-dates", "exams", "class-schedules"];
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
            if value["notModified"].as_bool() == Some(false) {
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
                WorkspaceChangeSource::ReferencePull,
                Some(revision),
            )?;
        }
        result
    }

    fn apply_fetch(&self, fetched: TransportFetch) -> Result<Value, ReferenceError> {
        let write =
            |apply: &dyn Fn(&rusqlite::Transaction<'_>) -> Result<usize, SubscriptionError>| {
                self.store
                    .write(Some(WorkspaceChangeSource::ReferencePull), apply)
            };
        let mut files = None;
        let changed = match fetched.value.as_ref() {
            None => return Ok(fetch_result(0, &fetched, None, true)),
            Some(ReferencePayload::Calendar(value)) => write(&|transaction| {
                subscriptions::apply_reference(transaction, Some(value), None)
            })?,
            Some(ReferencePayload::ImportantDates(value)) => {
                write(&|transaction| subscriptions::apply_important_dates(transaction, value))?
            }
            Some(ReferencePayload::ClassSchedules(value)) => {
                write(&|transaction| subscriptions::apply_class_schedules(transaction, value))?
            }
            Some(ReferencePayload::Exams(feeds)) => {
                let mut changed = 0;
                let mut applied = Vec::new();
                // Each exam feed keeps its own commit.
                for feed in feeds {
                    changed +=
                        write(&|transaction| subscriptions::apply_exam_feed(transaction, feed))?;
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
        "calendar" => &["academicCalendar"],
        "important-dates" => &["importantDates"],
        "exams" => &["exams"],
        "class-schedules" => &["classSchedules"],
        _ => {
            return Err(ReferenceError::InvalidInput(
                "unsupported reference cache kind".into(),
            ))
        }
    };
    let year_field = "academicYear";
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
    // A calendar applied before markers existed (or restored from a backup)
    // still counts when it is complete; a partially published calendar
    // counts through its local marker, so it is not refetched every hour.
    if request.kind == "calendar" && calendar_cache_present(workspace, request.year.into()) {
        return Ok(true);
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
#[path = "tests/reference_refresh.rs"]
mod tests;
