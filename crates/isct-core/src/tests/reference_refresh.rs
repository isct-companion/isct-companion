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
    let path = std::env::temp_dir().join(format!("isct-reference-{name}-{}.db", Uuid::new_v4()));
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
        .refresh_all(true)
        .await
        .unwrap();

    assert_eq!(transport.probes.load(Ordering::Relaxed), 1);
    let calls = transport.calls();
    assert_eq!(calls.len(), 3);
    assert!(!calls.iter().any(|call| call.0 == "important-dates"));

    service(&store, transport.clone())
        .refresh_all(true)
        .await
        .unwrap();
    assert_eq!(transport.probes.load(Ordering::Relaxed), 2);
    assert_eq!(transport.calls().len(), 6);
}

#[tokio::test]
async fn automatic_and_manual_runs_use_the_selected_term_year() {
    let store = store("selected-year");
    // A 2026 calendar makes currentTerm a 2026 quarter.
    service(&store, Arc::new(FakeTransport::default()))
        .apply_fetch(calendar_fetch(2026))
        .unwrap();
    store
        .put_workspace_setting("selectedTerm", json!("2031-1Q"), "test")
        .unwrap();
    let workspace = store.load_workspace().unwrap();
    assert!(workspace["currentTerm"]
        .as_str()
        .unwrap()
        .starts_with("2026"));
    disable(&store, &["departments", "important-dates", "exams"]);
    put_subscription(&store, "calendar", true, json!({}));
    let transport = Arc::new(FakeTransport::default());
    for force in [false, true] {
        service(&store, transport.clone())
            .refresh_all(force)
            .await
            .unwrap();
    }
    let years = transport
        .calls()
        .into_iter()
        .map(|(_, year, _)| year)
        .collect::<Vec<_>>();
    assert_eq!(years, [2031, 2031]);
    assert_eq!(
        subscription_views(&store.load_workspace().unwrap())["calendar"]["year"],
        2031
    );
}

#[tokio::test]
async fn partially_published_calendar_is_cached_until_it_changes() {
    let store = store("partial-calendar");
    let mut partial = calendar_fetch(2026);
    if let Some(ReferencePayload::Calendar(calendar)) = partial.value.as_mut() {
        calendar
            .quarters
            .retain(|quarter, _| quarter == "1Q" || quarter == "2Q");
    }
    let transport = Arc::new(FakeTransport::default());
    transport.respond("calendar", [FakeResponse::Ok(partial)]);
    let service = service(&store, transport.clone());
    service
        .refresh_one(
            request("calendar", 2026, false),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert!(!calendar_cache_present(
        &store.load_workspace().unwrap(),
        2026
    ));
    let workspace = store.load_workspace().unwrap();
    assert!(cache_present(&store, &workspace, &request("calendar", 2026, false)).unwrap());

    // The hourly automatic check neither probes nor fetches while fresh.
    let probes_before = transport.probes.load(Ordering::Relaxed);
    service
        .refresh_one(
            request("calendar", 2026, false),
            PullPriority::Automatic,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(transport.probes.load(Ordering::Relaxed), probes_before);

    // A forced check with an unchanged index reuses the cache.
    service
        .refresh_one(
            request("calendar", 2026, true),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(transport.calls().len(), 1);
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
        .refresh_all(true)
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
    assert!(matches!(
        ReferenceError::from(DataError::Conflict("stale".into())),
        ReferenceError::Conflict(message) if message == "stale"
    ));
    assert!(matches!(
        ReferenceError::from(DataError::Compatibility("old".into())),
        ReferenceError::Compatibility(message) if message == "old"
    ));
    assert!(matches!(
        ReferenceError::from(DataError::InvalidInput("bad".into())),
        ReferenceError::InvalidInput(message) if message == "bad"
    ));
}
