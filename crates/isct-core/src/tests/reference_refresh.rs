use super::*;
use crate::subscriptions::DateRange;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
};
use uuid::Uuid;

const INDEX: &str = "https://example.test/index.json";

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
    index_hash: StdMutex<String>,
    calls: StdMutex<Vec<FetchCall>>,
    responses: StdMutex<HashMap<String, VecDeque<FakeResponse>>>,
}

impl Default for FakeTransport {
    fn default() -> Self {
        Self {
            probes: AtomicUsize::new(0),
            index_hash: StdMutex::new("index-v1".into()),
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

    /// Publishes a new index; its files keep their hashes.
    fn publish_index(&self, hash: &str) {
        *self.index_hash.lock().unwrap() = hash.into();
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
            Ok(self.index_hash.lock().unwrap().clone())
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
                // Like the real source: the same file hash sends no body.
                None if known_hash == Some("remote-v1") => Ok(not_modified_fetch()),
                None => Ok(empty_fetch(kind, year)),
            }
        })
    }
}

fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("isct-reference-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

fn service(store: &DataStore, transport: Arc<dyn ReferenceTransport>) -> ReferenceService {
    ReferenceService::new(store.clone(), PullCoordinator::default())
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

/// A published file without rows.
fn empty_fetch(kind: &str, year: i32) -> TransportFetch {
    let value = match kind {
        "calendar" => calendar(year, &[]),
        "important-dates" => ReferencePayload::ImportantDates(ImportantDates {
            schema_version: 1,
            academic_year: year,
            timezone: "Asia/Tokyo".into(),
            source: Vec::new(),
            entries: Vec::new(),
        }),
        "exams" => ReferencePayload::Exams(Vec::new()),
        _ => ReferencePayload::ClassSchedules(ClassSchedules {
            schema_version: 1,
            academic_year: year,
            timezone: "Asia/Tokyo".into(),
            source: Vec::new(),
            entries: Vec::new(),
        }),
    };
    TransportFetch {
        value: Some(value),
        ..not_modified_fetch()
    }
}

fn calendar(year: i32, quarters: &[&str]) -> ReferencePayload {
    ReferencePayload::Calendar(AcademicCalendar {
        schema_version: 1,
        academic_year: year,
        source: Vec::new(),
        quarters: quarters
            .iter()
            .map(|quarter| {
                (
                    quarter.to_string(),
                    DateRange {
                        start: format!("{year}-04-01"),
                        end: format!("{year}-05-01"),
                    },
                )
            })
            .collect(),
        days: Vec::new(),
        teaching_ranges: BTreeMap::new(),
        exam_ranges: BTreeMap::new(),
    })
}

/// All four quarters: eight rows.
fn calendar_fetch(year: i32) -> TransportFetch {
    TransportFetch {
        value: Some(calendar(year, &["1Q", "2Q", "3Q", "4Q"])),
        ..not_modified_fetch()
    }
}

/// Writes a file's rows without any freshness record.
fn seed(store: &DataStore, fetch: TransportFetch) {
    store
        .write(
            Some(WorkspaceChangeSource::ReferencePull),
            |transaction| -> Result<_, SubscriptionError> {
                fetch.value.as_ref().unwrap().apply(transaction)
            },
        )
        .unwrap();
}

fn state(store: &DataStore, kind: &str, year: i32) -> Option<KeyState> {
    store
        .write(None, |transaction| {
            sync_state::get(transaction, &reference_key(kind, year))
        })
        .unwrap()
}

fn rows(store: &DataStore, kind: &str, year: i32) -> usize {
    store
        .write(None, |transaction| stored_rows(transaction, kind, year))
        .unwrap()
}

/// `(key, outcome, error_kind)` of every `sync_runs` row, oldest first.
fn runs(store: &DataStore) -> Vec<(String, String, Option<String>)> {
    store
        .write(None, |transaction| {
            let mut statement = transaction
                .prepare("SELECT key, outcome, error_kind FROM sync_runs ORDER BY id")
                .map_err(DataError::from)?;
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .map_err(DataError::from)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(DataError::from)?;
            Ok::<_, DataError>(rows)
        })
        .unwrap()
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
    store.put_user_record("subscriptions", row).unwrap()
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
        run: "test-run".into(),
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
        )
        .unwrap();
    put_subscription(&store, "calendar", true, json!({}));
    put_subscription(&store, "departments", true, json!({}));
    put_subscription(&store, "important-dates", false, json!({}));
    put_subscription(&store, "exams", true, json!({}));
    put_subscription(&store, "class-schedules", true, json!({}));
    let transport = Arc::new(FakeTransport::default());
    service(&store, transport.clone())
        .refresh_all(true)
        .await
        .unwrap();

    assert_eq!(transport.probes.load(Ordering::Relaxed), 1);
    let calls = transport.calls();
    assert_eq!(calls.len(), 3);
    assert!(!calls.iter().any(|call| call.0 == "departments"));
    assert!(!calls.iter().any(|call| call.0 == "important-dates"));

    // Every refresh probes again; files stored from an unchanged index are
    // renewed, empty ones included, without fetching them.
    service(&store, transport.clone())
        .refresh_all(true)
        .await
        .unwrap();
    assert_eq!(transport.probes.load(Ordering::Relaxed), 2);
    assert_eq!(transport.calls().len(), 3);
}

#[tokio::test]
async fn automatic_and_manual_runs_use_the_selected_term_year() {
    let store = store("selected-year");
    // A 2026 calendar makes currentTerm a 2026 quarter.
    seed(&store, calendar_fetch(2026));
    store
        .put_workspace_setting("selectedTerm", json!("2031-1Q"))
        .unwrap();
    let workspace = store.load_workspace().unwrap();
    assert!(workspace["currentTerm"]
        .as_str()
        .unwrap()
        .starts_with("2026"));
    disable(
        &store,
        &["departments", "important-dates", "exams", "class-schedules"],
    );
    put_subscription(&store, "calendar", true, json!({}));
    let transport = Arc::new(FakeTransport::default());
    for force in [false, true] {
        service(&store, transport.clone())
            .refresh_all(force)
            .await
            .unwrap();
        transport.publish_index("index-v2");
    }
    let years = transport
        .calls()
        .into_iter()
        .map(|(_, year, _)| year)
        .collect::<Vec<_>>();
    assert_eq!(years, [2031, 2031]);
    assert!(state(&store, "calendar", 2031)
        .unwrap()
        .fetched_at
        .is_some());
    assert_eq!(
        subscription_views(&store.load_workspace().unwrap())["calendar"]["year"],
        2031
    );
}

#[tokio::test]
async fn stored_file_is_renewed_without_a_fetch_while_its_index_is_unchanged() {
    let store = store("renew");
    let mut partial = calendar_fetch(2026);
    partial.value = Some(calendar(2026, &["1Q", "2Q"]));
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
    assert_eq!(rows(&store, "calendar", 2026), 4);
    let fresh = state(&store, "calendar", 2026).unwrap();
    assert_eq!(
        fresh.validator.as_deref(),
        Some("index-v1:remote-v1:4@https://example.test/index.json")
    );

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

    // A forced check with an unchanged index only renews.
    let result = service
        .refresh_one(
            request("calendar", 2026, true),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(result["notModified"], true);
    assert_eq!(transport.calls().len(), 1);

    // A new index sends the file hash; an unchanged file is not downloaded
    // and the new index hash becomes the validator.
    transport.publish_index("index-v2");
    service
        .refresh_one(
            request("calendar", 2026, true),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(transport.calls()[1].2.as_deref(), Some("remote-v1"));
    assert_eq!(rows(&store, "calendar", 2026), 4);
    assert!(state(&store, "calendar", 2026)
        .unwrap()
        .validator
        .unwrap()
        .starts_with("index-v2:remote-v1:4@"));
    let outcomes: Vec<_> = runs(&store).into_iter().map(|run| run.1).collect();
    assert_eq!(outcomes, ["complete", "unchanged", "unchanged"]);
}

#[tokio::test]
async fn validator_without_stored_rows_reads_the_file_in_full() {
    // Rows gone (gc, reset) under a fresh, matching validator; rows of
    // another year.
    for (name, stored_year, requested_year) in
        [("missing-rows", 2026, 2026), ("changed-year", 2025, 2026)]
    {
        let store = store(name);
        let transport = Arc::new(FakeTransport::default());
        let service = service(&store, transport.clone());
        if stored_year != requested_year {
            seed(&store, calendar_fetch(stored_year));
        }
        store
            .write(None, |transaction| {
                sync_state::commit(
                    transaction,
                    &reference_key("calendar", stored_year),
                    Commit {
                        account: "",
                        validator: Some(&format!("index-v1:remote-v1:8@{INDEX}")),
                        outcome: Outcome::Complete,
                        message: None,
                    },
                    "2026-10-13T00:00:00Z".parse().unwrap(),
                )
            })
            .unwrap();
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
        assert_eq!(rows(&store, "calendar", requested_year), 8, "{name}");
    }
}

#[tokio::test]
async fn suspicious_shrink_is_held_once_and_published_when_the_same_version_returns() {
    let store = store("shrink");
    let transport = Arc::new(FakeTransport::default());
    let service = service(&store, transport.clone());
    transport.respond("calendar", [FakeResponse::Ok(calendar_fetch(2026))]);
    service
        .refresh_one(
            request("calendar", 2026, true),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(rows(&store, "calendar", 2026), 8);

    // A new version with one quarter left: 2 of 8 rows.
    let shrunk = || TransportFetch {
        value: Some(calendar(2026, &["1Q"])),
        remote_hash: "remote-v2".into(),
        ..not_modified_fetch()
    };
    transport.publish_index("index-v2");
    transport.respond(
        "calendar",
        [FakeResponse::Ok(shrunk()), FakeResponse::Ok(shrunk())],
    );
    let held = service
        .refresh_one(
            request("calendar", 2026, true),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(held["held"], true);
    assert_eq!(rows(&store, "calendar", 2026), 8);
    let state_after_hold = state(&store, "calendar", 2026).unwrap();
    assert_eq!(
        state_after_hold.last_error.as_deref(),
        Some(SUSPICIOUS_SHRINK)
    );
    assert!(state_after_hold.next_eligible.is_some());
    assert!(state_after_hold
        .validator
        .unwrap()
        .starts_with("index-v1:remote-v1:8@"));
    assert_eq!(
        runs(&store).last().unwrap().2.as_deref(),
        Some(SUSPICIOUS_SHRINK)
    );

    // The same version read again is accepted.
    service
        .refresh_one(
            request("calendar", 2026, true),
            PullPriority::Manual,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(rows(&store, "calendar", 2026), 2);
    let published = state(&store, "calendar", 2026).unwrap();
    assert_eq!((published.failures, published.last_error), (0, None));
    assert!(published
        .validator
        .unwrap()
        .starts_with("index-v2:remote-v2:2@"));
    assert!(store
        .write(None, |transaction| sync_state::cursor(
            transaction,
            &reference_key("calendar", 2026)
        ))
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn failed_read_keeps_rows_and_backs_off_until_a_retry_succeeds() {
    let store = store("retry");
    seed(&store, calendar_fetch(2026));
    put_subscription(&store, "calendar", true, json!({"year":2026}));
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
    let service = service(&store, transport.clone());
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
    assert_eq!(rows(&store, "calendar", 2026), 8);
    let backing_off = state(&store, "calendar", 2026).unwrap();
    assert_eq!(
        (backing_off.failures, backing_off.last_error.as_deref()),
        (1, Some("network"))
    );
    assert!(backing_off.fetched_at.is_none());
    assert_eq!(
        runs(&store),
        [(
            "calendar/2026".to_string(),
            "failed".to_string(),
            Some("network".to_string())
        )]
    );

    // An automatic run waits out the backoff.
    service
        .refresh_one(
            request("calendar", 2026, false),
            PullPriority::Automatic,
            probes(),
        )
        .await
        .unwrap();
    assert_eq!(transport.calls().len(), 1);

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
    assert!(retried.get("remoteHash").is_none());
    let recovered = state(&store, "calendar", 2026).unwrap();
    assert_eq!((recovered.failures, recovered.last_error), (0, None));
    assert!(recovered.fetched_at.is_some());
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
        year: i32,
        _expected_index_hash: &'a str,
        _known_hash: Option<&'a str>,
    ) -> ReferenceTransportFuture<'a, TransportFetch> {
        Box::pin(async move {
            let mut row = self.store.user_rows("subscriptions")?.remove(0);
            let revision = row["rev"].as_u64().unwrap();
            row["url"] = json!("https://user.example/new-index.json");
            self.store.put_user_record_checked(
                "subscriptions",
                row,
                crate::data::WorkspaceChangeSource::UserIntent,
                Some(revision),
            )?;
            Ok(calendar_fetch(year))
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
    let service = ReferenceService::new(store.clone(), coordinator.clone())
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
    assert!(state(&store, "calendar", 2026).is_none_or(|state| state.fetched_at.is_none()));
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
    assert_eq!(rows(&store, "calendar", 2026), 8);
    assert!(state(&store, "calendar", 2026)
        .unwrap()
        .fetched_at
        .is_some());
    assert!(subscription_row(&workspace, "calendar").unwrap()["lastError"].is_null());
    assert_eq!(
        subscription_row(&workspace, "important-dates").unwrap()["lastError"],
        "offline"
    );
}

#[test]
fn subscription_views_follow_the_selected_year_and_reject_bad_fetch_dates() {
    let mut workspace = json!({"selectedTerm":"2026-3Q","currentTerm":"2025-4Q","academicProfile":{"admissionYear":2024},"subscriptions":[{"kind":"calendar","year":2026,"fetchedAt":"not-a-date"}]});
    let views = subscription_views(&workspace);
    assert!(views.get("departments").is_none());
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
