use super::*;
use crate::data::DataStore;
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
    let path = std::env::temp_dir().join(format!("isct-reference-{name}-{}", Uuid::new_v4()));
    DataStore::initialize_dir(path).unwrap().0
}

fn reader(transport: Arc<dyn ReferenceTransport>) -> ReferenceReader {
    ReferenceReader::new(
        transport,
        Arc::new(|| "2026-10-13T03:15:00Z".parse().unwrap()),
    )
}

/// Reads one leased key and records its ending as the runner does.
async fn read(
    store: &DataStore,
    reader: &ReferenceReader,
    probes: &IndexProbes,
    kind: &str,
    year: i32,
) -> KeyEnd {
    let lease = Lease::detached(store, reference_key(kind, year), "").with_locator(INDEX);
    let end = reader.read(&lease, probes).await;
    lease.ending(&end).write();
    end
}

/// What the planner sees for a key under its rules.
fn due(store: &DataStore, kind: &str, year: i32) -> Due {
    store
        .write(None, |transaction| {
            reference_due(
                transaction,
                &reference_key(kind, year),
                kind,
                year,
                INDEX,
                "2026-10-13T03:20:00Z".parse().unwrap(),
            )
        })
        .unwrap()
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

#[tokio::test]
async fn followed_files_share_one_index_probe_and_skip_disabled_kinds() {
    let store = store("shared-probe");
    store
        .put_workspace_setting("selectedTerm", json!("2026-3Q"))
        .unwrap();
    put_subscription(&store, "calendar", true, json!({}));
    put_subscription(&store, "departments", true, json!({}));
    put_subscription(&store, "important-dates", false, json!({}));
    put_subscription(&store, "exams", true, json!({}));
    put_subscription(&store, "class-schedules", true, json!({}));
    let followed = followed(&store.load_workspace().unwrap());
    assert_eq!(
        followed
            .iter()
            .map(|(kind, year, url)| (kind.as_str(), *year, url.as_str()))
            .collect::<Vec<_>>(),
        [
            ("calendar", 2026, INDEX),
            ("exams", 2026, INDEX),
            ("class-schedules", 2026, INDEX)
        ]
    );
    let transport = Arc::new(FakeTransport::default());
    let reader = reader(transport.clone());
    let probes = IndexProbes::default();
    for (kind, year, _) in &followed {
        assert!(matches!(
            read(&store, &reader, &probes, kind, *year).await,
            KeyEnd::Done { .. }
        ));
    }
    assert_eq!(transport.probes.load(Ordering::Relaxed), 1);
    assert_eq!(transport.calls().len(), 3);

    // A later evaluation probes again; files stored from an unchanged index
    // are renewed, empty ones included, without fetching them.
    let probes = IndexProbes::default();
    for (kind, year, _) in &followed {
        read(&store, &reader, &probes, kind, *year).await;
    }
    assert_eq!(transport.probes.load(Ordering::Relaxed), 2);
    assert_eq!(transport.calls().len(), 3);
}

#[tokio::test]
async fn references_follow_the_selected_term_year() {
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
    let followed = followed(&workspace);
    assert!(followed.iter().all(|(_, year, _)| *year == 2031));
    let transport = Arc::new(FakeTransport::default());
    read(
        &store,
        &reader(transport.clone()),
        &IndexProbes::default(),
        "calendar",
        2031,
    )
    .await;
    assert_eq!(transport.calls()[0].1, 2031);
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
    let reader = reader(transport.clone());
    assert_eq!(due(&store, "calendar", 2026), Due::Missing);
    read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await;
    assert_eq!(rows(&store, "calendar", 2026), 4);
    let fresh = state(&store, "calendar", 2026).unwrap();
    assert_eq!(
        fresh.validator.as_deref(),
        Some("index-v1:remote-v1:4@https://example.test/index.json")
    );

    // The planner does not read a fresh file on its own.
    assert_eq!(due(&store, "calendar", 2026), Due::Fresh);

    // A refresh with an unchanged index only renews.
    assert!(matches!(
        read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await,
        KeyEnd::Done { changed: 0 }
    ));
    assert_eq!(transport.calls().len(), 1);

    // A new index sends the file hash; an unchanged file is not downloaded
    // and the new index hash becomes the validator.
    transport.publish_index("index-v2");
    read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await;
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
        // Missing content beats the validator: the planner reads it.
        assert!(due(&store, "calendar", requested_year).is_due(), "{name}");
        transport.respond(
            "calendar",
            [FakeResponse::Ok(calendar_fetch(requested_year))],
        );
        read(
            &store,
            &reader(transport.clone()),
            &IndexProbes::default(),
            "calendar",
            requested_year,
        )
        .await;
        assert_eq!(transport.calls()[0].2, None, "{name}");
        assert_eq!(rows(&store, "calendar", requested_year), 8, "{name}");
    }
}

#[tokio::test]
async fn suspicious_shrink_is_held_once_and_published_when_the_same_version_returns() {
    let store = store("shrink");
    let transport = Arc::new(FakeTransport::default());
    let reader = reader(transport.clone());
    transport.respond("calendar", [FakeResponse::Ok(calendar_fetch(2026))]);
    read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await;
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
    assert!(matches!(
        read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await,
        KeyEnd::Refused { kind } if kind == SUSPICIOUS_SHRINK
    ));
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
    read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await;
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
    let reader = reader(transport.clone());
    assert!(matches!(
        read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await,
        KeyEnd::Failed { kind, .. } if kind == "network"
    ));
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

    // The planner waits out the backoff; a refresh press reads anyway.
    assert!(matches!(due(&store, "calendar", 2026), Due::BackingOff(_)));
    read(&store, &reader, &IndexProbes::default(), "calendar", 2026).await;
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
    let transport = Arc::new(EditingTransport {
        store: store.clone(),
    });
    read(
        &store,
        &reader(transport),
        &IndexProbes::default(),
        "calendar",
        2026,
    )
    .await;
    let workspace = store.load_workspace().unwrap();
    let row = subscription_row(&workspace, "calendar").unwrap();
    assert_eq!(row["url"], "https://user.example/new-index.json");
    assert!(row["fetchedAt"].is_null());
}

/// Cancels the lease while the file downloads, as a reset or shutdown does.
struct CancellingTransport {
    cancel: tokio_util::sync::CancellationToken,
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
            self.cancel.cancel();
            Ok(calendar_fetch(year))
        })
    }
}

#[tokio::test]
async fn cancellation_before_applying_writes_no_data_or_metadata() {
    let store = store("cancel");
    let lease = Lease::detached(&store, reference_key("calendar", 2026), "").with_locator(INDEX);
    let reader = reader(Arc::new(CancellingTransport {
        cancel: lease.cancel.clone(),
    }));
    assert!(matches!(
        reader.read(&lease, &IndexProbes::default()).await,
        KeyEnd::Cancelled { .. }
    ));
    let workspace = store.load_workspace().unwrap();
    assert!(workspace["academicCalendar"].as_array().unwrap().is_empty());
    assert!(subscription_row(&workspace, "calendar").is_none());
    assert!(state(&store, "calendar", 2026).is_none_or(|state| state.fetched_at.is_none()));
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
fn data_errors_keep_their_reference_category() {
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
    assert!(matches!(
        ReferenceError::from(DataError::Storage("disk".into())),
        ReferenceError::Storage(message) if message == "disk"
    ));
}
