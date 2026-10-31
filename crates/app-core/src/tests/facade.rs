use super::*;
use crate::workspace::{tests::ReadBack, WorkspaceIntent, WorkspaceMutation};
use crate::{
    cloud_sync::{
        CloudFuture, CloudProvider, CloudSync, CloudSyncError, CloudTransport, CloudVersion,
    },
    events::CoreEvent,
    ports::{EventError, SecretError, SecretVault},
    schedule::{NotificationItem, NotificationSink, NotificationSinkError},
    scheduler::{ExportError, ScheduledExporter},
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use uuid::Uuid;

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("vault unavailable".into()))?
            .insert(account.into(), secret.into());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| SecretError::Access("vault unavailable".into()))?
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .map_err(|_| SecretError::Access("vault unavailable".into()))?
            .remove(account);
        Ok(())
    }
}

struct NoopEvents;
impl EventSink for NoopEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), EventError> {
        Ok(())
    }
}

struct NoopExporter;
impl ScheduledExporter for NoopExporter {
    fn export(&self) -> Result<(), ExportError> {
        Ok(())
    }
}

struct NoopNotifications;
impl NotificationSink for NoopNotifications {
    fn deliver<'a>(
        &'a self,
        _item: NotificationItem,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), NotificationSinkError>> + Send + 'a>,
    > {
        Box::pin(async { Ok(()) })
    }
}

struct NoopCloud;
impl CloudTransport for NoopCloud {
    fn disconnect(&self, _provider: CloudProvider) -> Result<(), CloudSyncError> {
        Ok(())
    }
    fn read_workspace<'a>(
        &'a self,
        _provider: CloudProvider,
    ) -> CloudFuture<'a, Option<serde_json::Value>> {
        Box::pin(async { Ok(None) })
    }
    fn merge_and_push<'a>(
        &'a self,
        _provider: CloudProvider,
        document: &'a serde_json::Value,
        _device: &'a str,
    ) -> CloudFuture<'a, serde_json::Value> {
        Box::pin(async move { Ok(document.clone()) })
    }
    fn versions<'a>(&'a self, _provider: CloudProvider) -> CloudFuture<'a, Vec<CloudVersion>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn version_document<'a>(
        &'a self,
        _provider: CloudProvider,
        _revision: u64,
    ) -> CloudFuture<'a, serde_json::Value> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }
}

struct DropEvents(Arc<AtomicBool>);
impl EventSink for DropEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), EventError> {
        Ok(())
    }
}
impl Drop for DropEvents {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("app-core-{name}-{}", Uuid::new_v4()));
    DataStore::initialize_dir(path).unwrap().0
}

#[tokio::test]
async fn credential_mutations_work_without_a_school_browser() {
    let store = store("auth");
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    let core = Core::new(store, auth.clone());
    let profile = SchoolAuthProfile::TokyoTech;

    // Connecting needs the browser to sign in, but keeps what was entered.
    assert!(matches!(
        core.connect_school(
            profile,
            SchoolCredentials {
                username: "student".into(),
                password: "secret".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        )
        .await,
        Err(CredentialMutationError::Browser(_))
    ));
    assert!(auth.load(profile).unwrap().is_some());
    assert!(core.delete_credentials(profile).unwrap());
    assert!(auth.load(profile).unwrap().is_none());
    core.shutdown().await;
}

#[tokio::test]
async fn wake_reports_every_service_after_runtime_failure() {
    let store = store("wake-report");
    let core = Core::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
    );
    let scheduler = core.services(Arc::new(NoopExporter));
    let cloud = crate::cloud_sync::CloudServices {
        backup: crate::backup::BackupService::new(store),
        transport: Arc::new(NoopCloud),
        events: Arc::new(NoopEvents),
    };
    core.runtime.shutdown().await;

    let report = core
        .wake(WakeServices {
            scheduler,
            cloud: Some(cloud),
            reminders: Some(ReminderWake {
                sink: Arc::new(NoopNotifications),
                now: chrono::Utc::now().fixed_offset(),
            }),
        })
        .await;
    assert!(matches!(
        report.runtime,
        Err(RuntimeError::Task(
            crate::runtime_tasks::RuntimeTaskError::Stopped
        ))
    ));
    assert!(matches!(
        report.cloud,
        Some(Err(CloudWakeError::NotAttached))
    ));
    assert!(matches!(report.reminders, Some(Ok(_))));
    core.shutdown().await;
}

#[tokio::test]
async fn shutdown_releases_the_native_event_port() {
    let dropped = Arc::new(AtomicBool::new(false));
    let core = Core::with_events(
        store("event-release"),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        Arc::new(DropEvents(dropped.clone())),
    );
    // Offline: the started planner holds every key instead of reading.
    core.sync_hint(Hint::Network {
        network: Network::Offline,
    });
    core.start(&tokio::runtime::Handle::current()).unwrap();
    assert!(!dropped.load(Ordering::Acquire));
    core.shutdown().await;
    assert!(dropped.load(Ordering::Acquire));
}

#[derive(Default)]
struct RecordedEvents(Mutex<Vec<(&'static str, serde_json::Value)>>);
impl EventSink for RecordedEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), EventError> {
        self.0
            .lock()
            .map_err(|_| EventError::Delivery("recorder unavailable".into()))?
            .push((event.name(), event.payload()));
        Ok(())
    }
}

type Published = (WorkspaceChangeSource, Option<String>, Vec<WorkspaceChange>);

#[derive(Default)]
struct WorkspaceEvents {
    changes: Mutex<Vec<Published>>,
    /// The `seq` of each published event, in order.
    seqs: Mutex<Vec<u64>>,
    fail: AtomicBool,
}

impl WorkspaceEvents {
    fn published(&self) -> Vec<Published> {
        self.changes.lock().unwrap().clone()
    }

    /// Waits for a held background change; the window is 250 ms.
    fn wait_for(&self, count: usize) -> Vec<Published> {
        for _ in 0..200 {
            if self.published().len() >= count {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.published()
    }
}

impl EventSink for WorkspaceEvents {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        if let CoreEvent::WorkspaceChanged {
            source,
            revision,
            changes,
            seq,
        } = event
        {
            self.seqs.lock().unwrap().push(seq);
            self.changes
                .lock()
                .unwrap()
                .push((source, revision, changes));
        }
        if self.fail.load(Ordering::Relaxed) {
            return Err(EventError::Delivery("event channel closed".into()));
        }
        Ok(())
    }
}

#[tokio::test]
async fn the_relay_folds_school_sessions_into_the_sync_snapshot() {
    use crate::lifecycle::SchoolSite;
    use crate::school_browser::{SchoolSessionEvent, SchoolSessionState};
    use crate::school_service::{IngestOutcome, IngestState};
    use crate::sync_state::{self, Commit, Dataset, Key, Outcome};
    let store = store("sync-sources");
    store
        .write(None, |transaction| {
            sync_state::commit(
                transaction,
                &Key::new(Dataset::LmsDeadlines, "all"),
                Commit {
                    account: "science:student",
                    validator: None,
                    outcome: Outcome::Complete,
                    message: None,
                },
                "2026-10-19T02:46:13Z".parse().unwrap(),
            )
        })
        .unwrap();
    let recorded = Arc::new(RecordedEvents::default());
    let core = Core::with_events(
        store,
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        recorded.clone(),
    );
    core.sync_hint(Hint::Network {
        network: Network::Offline,
    });
    core.start(&tokio::runtime::Handle::current()).unwrap();
    let relay = core.events.clone().unwrap();
    relay
        .emit(CoreEvent::SchoolSession(SchoolSessionEvent {
            profile: SchoolAuthProfile::Science,
            state: SchoolSessionState::Active,
            revision: 1,
        }))
        .unwrap();
    relay
        .emit(CoreEvent::SchoolIngest(IngestOutcome {
            site: SchoolSite::Kyomu,
            kind: "service".into(),
            state: IngestState::Maintenance,
            changed: 0,
            message: "閉局中".into(),
        }))
        .unwrap();
    // The relay passes every event on; the snapshot follows as its own event,
    // and the query answers what the last one said, for a shell that missed it.
    let sent = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let current = serde_json::to_value(core.sync_snapshot().await).unwrap();
            let last = recorded
                .0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(name, _)| *name == "sync-snapshot")
                .map(|(_, payload)| payload.clone());
            if last.as_ref() == Some(&current)
                && current["sources"][1]["maintenance"].is_string()
                && current["sources"][0]["lastUpdated"].is_string()
            {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let names: Vec<_> = recorded
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(&names[..2], ["school-session", "school-ingest"]);
    assert_eq!(sent["sources"][0]["source"], "lms");
    assert_eq!(sent["sources"][0]["session"], "active");
    assert_eq!(sent["sources"][0]["lastUpdated"], "2026-10-19T02:46:13Z");
    assert_eq!(sent["sources"][1]["maintenance"], "閉局中");
    // Offline holds the signed-in profile's due keys without failing them.
    assert_eq!(sent["sources"][0]["held"], json!({"kind": "offline"}));
    assert!(sent["sources"][0]["failure"].is_null());
    core.shutdown().await;
}

fn admission_year(
    core: &Core,
    expected_revision: String,
    year: i32,
) -> Result<ReadBack, WorkspaceError> {
    core.workspace.apply(WorkspaceMutation {
        expected_revision,
        intent: WorkspaceIntent::SetAdmissionYear {
            admission_year: Some(year),
        },
    })
}

/// Shells and agent adapters must not have to publish what they wrote: the
/// store's commit path reports every change, once, with its source.
#[test]
fn core_publishes_each_committed_write_once_whoever_made_it() {
    let store = store("published-writes");
    store
        .put_workspace_setting("devicePreferences", json!({"agentAccess":{"enabled":true}}))
        .unwrap();
    let events = Arc::new(WorkspaceEvents::default());
    let core = Core::with_events(
        store,
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        events.clone(),
    );
    let changes = || events.published();
    let query_seq = || {
        core.schedule_query(vec![crate::schedule::WorkspaceQuery::TeachingPeriods])
            .unwrap()
            .seq
    };
    let before = query_seq();

    let committed =
        admission_year(&core, core.workspace.read_back().unwrap().revision, 2026).unwrap();
    let [(source, revision, names)] = &changes()[..] else {
        panic!("one event per committed intent: {:?}", changes());
    };
    assert_eq!(
        (*source, revision.as_deref()),
        (
            WorkspaceChangeSource::UserIntent,
            Some(committed.revision.as_str())
        )
    );
    assert!(names.contains(&WorkspaceChange::Settings), "{names:?}");
    // The event names the commit's sequence: an answer read before it is
    // older, one read after it is not.
    let heard = events.seqs.lock().unwrap()[0];
    assert!(before < heard, "{before} < {heard}");
    assert_eq!(query_seq(), heard);

    // A rejected intent and one that changes no row publish nothing and
    // leave the sequence where it was.
    assert!(matches!(
        admission_year(&core, "stale".into(), 2025),
        Err(WorkspaceError::Conflict(_))
    ));
    admission_year(&core, committed.revision.clone(), 2026).unwrap();
    assert_eq!(changes().len(), 1);
    assert_eq!(query_seq(), heard);

    // An MCP tool writes outside WorkspaceService and is still published,
    // at once: it is the first background change.
    #[cfg(feature = "agent")]
    {
        core.agent
            .call(
                "weekly_event_create",
                json!({"term":"2026-3Q","title":"Review","days":[1],"startTime":"09:00","endTime":"09:45"}),
            )
            .unwrap();
        let (source, revision, names) = changes().last().cloned().unwrap();
        assert_eq!((source, revision), (WorkspaceChangeSource::Mcp, None));
        // A weekly event is a row of `personal_events`.
        assert!(names.contains(&WorkspaceChange::Events), "{names:?}");
    }

    // A failed delivery neither fails nor undoes the committed write.
    events.fail.store(true, Ordering::Relaxed);
    let after = admission_year(&core, core.workspace.read_back().unwrap().revision, 2027).unwrap();
    assert_eq!(after.workspace["academicProfile"]["admissionYear"], 2027);
    assert_eq!(changes().len(), 3);
}

/// The finder keeps its prepared catalog and personal context between
/// keystrokes. A catalog commit, an emptied cache.db and a workspace change
/// each show at the next query, never the cached state. A catalog commit
/// tells shells which year changed, at most once per window and year, with
/// the version a page read after it names; queries that miss together agree.
#[test]
fn finder_queries_follow_catalog_and_workspace_commits() {
    let store = store("finder-cache");
    let events = Arc::new(RecordedEvents::default());
    let core = Core::with_events(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        events.clone(),
    );
    let offering = |id: &str, year: u16, code: &str| {
        json!({"id":id,"year":year,"quarters":["3Q"],"code":code,
            "title":{"ja":code},"instructors":[],"instructionLanguage":"ja",
            "format":{"kinds":["lecture"]},"media":{},
            "credits":{"lecture":2,"exercise":0,"lab":0},"schedule":[],
            "academicUnit":{},"intensive":false,
            "syllabusUrl":crate::catalog::syllabus_url(id, "ja"),"fetchedAt":{}})
    };
    // As the sync engine commits one department of a year.
    let land = |year: u16, rows: &[serde_json::Value]| {
        store
            .catalog_write(year, |transaction| {
                DataStore::catalog_upsert_offerings_in(transaction, rows)
            })
            .unwrap()
    };
    let query = || {
        core.finder_query(serde_json::from_value(json!({"year":2026,"term":"2026-3Q"})).unwrap())
            .unwrap()
    };
    let ids = |page: &crate::course_query::FinderPage| {
        page.rows
            .iter()
            .map(|row| row.course["id"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>()
    };
    let catalog_events = |count: usize| {
        let heard = || {
            events
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, _)| *name == "catalog-changed")
                .map(|(_, payload)| payload.clone())
                .collect::<Vec<_>>()
        };
        // A held change is delivered when its 250 ms window ends.
        for _ in 0..200 {
            if heard().len() >= count {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        heard()
    };

    land(2026, &[offering("202600001", 2026, "CSC.T101")]);
    let first = query();
    assert_eq!(ids(&first), ["202600001"]);
    // The first change goes out at once and names the version just read.
    assert_eq!(
        catalog_events(1),
        [json!({"year":2026,"version":first.catalog_version})]
    );
    // The rest of the window arrives merged, one event per year; the
    // search form's new choices count as a change of their year.
    land(2026, &[offering("202600002", 2026, "CSC.T201")]);
    store
        .record_kyomu_search_options(2026, &json!([]), chrono::Utc::now())
        .unwrap();
    land(2025, &[offering("202500001", 2025, "CSC.T101")]);
    assert_eq!(catalog_events(0).len(), 1, "held until the window ends");
    let page = query();
    assert_eq!(ids(&page), ["202600001", "202600002"]);
    assert_eq!(page.ineligible, 0);
    // The page names the data it was read from, so a shell can tell it is newer.
    assert!(page.catalog_version > first.catalog_version);
    let heard = catalog_events(3);
    assert_eq!(heard[1]["year"], 2026);
    assert!(heard[1]["version"].as_u64() > Some(first.catalog_version));
    assert_eq!(
        heard[2],
        json!({"year":2025,"version":page.catalog_version})
    );
    // A commit that changed no row of the catalog says nothing, even once
    // the window has passed.
    std::thread::sleep(Duration::from_millis(300));
    store
        .record_kyomu_search_options(2026, &json!([]), chrono::Utc::now())
        .unwrap();
    land(2026, &[offering("202600002", 2026, "CSC.T201")]);
    assert_eq!(catalog_events(0).len(), 3);

    // Cold queries made at once, as typing can now send them, agree.
    land(2026, &[offering("202600003", 2026, "CSC.T102")]);
    let pages: Vec<_> = std::thread::scope(|scope| {
        let running: Vec<_> = (0..4).map(|_| scope.spawn(query)).collect();
        running
            .into_iter()
            .map(|query| query.join().unwrap())
            .collect()
    });
    for concurrent in &pages {
        assert_eq!(ids(concurrent), ["202600001", "202600003", "202600002"]);
        assert_eq!(concurrent.catalog_version, pages[0].catalog_version);
    }

    store
        .put_workspace_setting(
            "academicProfile",
            json!({"name":"Mine","gradePath":[],"years":[],
                "studyEnrollments":[{"stage":"undergraduate","entryKind":"regular",
                "entryGrade":1,"admittedTerm":"2026-1Q"}]}),
        )
        .unwrap();
    let before = page.content_revision;
    let page = query();
    assert_eq!(page.ineligible, 1);
    assert_ne!(page.content_revision, before);

    store.rebuild_cache().unwrap();
    assert_eq!(query().stored, 0);
}

/// A sync commits key after key. The shell hears the first commit at once
/// and the rest of its window merged, each event naming what changed; those
/// names feed every field the commits changed.
#[test]
fn background_commits_reach_shells_merged_with_the_records_they_changed() {
    use crate::sync_state::{self, Commit, Dataset, Key, Outcome};
    use WorkspaceChangeSource::SchoolPull;
    let store = store("background-writes");
    let events = Arc::new(WorkspaceEvents::default());
    // Held so the store's commits reach `events`.
    let _core = Core::with_events(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
        events.clone(),
    );
    // Every workspace field with the sync records, as one view reads them.
    let whole = || {
        let mut whole = serde_json::Value::clone(&store.load_reads().unwrap().0);
        let sync = store
            .view(&["freshness", "pageAvailability"].into_iter().collect())
            .unwrap()
            .0;
        for field in ["freshness", "pageAvailability"] {
            whole[field] = sync[field].clone();
        }
        whole
    };
    let before = whole();
    let renew = |transaction: &rusqlite::Transaction<'_>, at: &str| {
        sync_state::commit(
            transaction,
            &Key::new(Dataset::LmsDeadlines, "all"),
            Commit {
                account: "science:student",
                validator: None,
                outcome: Outcome::Complete,
                message: None,
            },
            at.parse().unwrap(),
        )
    };

    // An unchanged revalidation renews only its key's freshness.
    store
        .write(Some(SchoolPull), |transaction| {
            renew(transaction, "2026-10-19T02:46:13Z")
        })
        .unwrap();
    assert_eq!(
        events.published(),
        [(SchoolPull, None, vec![WorkspaceChange::Freshness])]
    );

    // The next key lands inside the window: held, then merged.
    store
        .write(Some(SchoolPull), |transaction| {
            transaction
                .execute(
                    "INSERT INTO registrations
                       (id, value, hash, updated_at, updated_by, rev, source)
                     VALUES ('kyomu:CSC.T251', ?1, 'h', '2026-10-19T02:47:00Z', 'school', 1, ?2)",
                    rusqlite::params![
                        json!({"id":"kyomu:CSC.T251","offeringId":"","code":"CSC.T251","title":"Test"})
                            .to_string(),
                        json!({"provider":"kyomu"}).to_string(),
                    ],
                )
                .map_err(crate::data::DataError::from)?;
            renew(transaction, "2026-10-19T02:47:00Z")
        })
        .unwrap();
    assert_eq!(events.published().len(), 1, "held until the window ends");
    assert_eq!(
        events.wait_for(2)[1],
        (
            SchoolPull,
            None,
            vec![WorkspaceChange::Registrations, WorkspaceChange::Freshness]
        )
    );

    // Every field the commits changed is fed by a change they named, so a
    // shell that re-queries what reads those names misses none of it.
    let after = whole();
    let heard: Vec<WorkspaceChange> = events
        .published()
        .into_iter()
        .flat_map(|(_, _, names)| names)
        .collect();
    for (field, value) in after.as_object().unwrap() {
        if before.get(field) != Some(value) {
            assert!(
                crate::workspace_changes::feeding(std::slice::from_ref(field))
                    .iter()
                    .any(|change| heard.contains(change)),
                "{field} changed, but no heard change feeds it"
            );
        }
    }
    assert_eq!(after["registrations"][0]["code"], "CSC.T251");
    assert!(after["freshness"]
        .as_object()
        .unwrap()
        .contains_key("lms.deadlines/all"));
}

/// Cloud capture follows local changes without a shell asking for it: a
/// committed write makes the next wake capture at once instead of after the
/// periodic interval.
#[tokio::test]
async fn local_changes_bring_the_next_cloud_capture_forward() {
    let root = std::env::temp_dir().join(format!("app-core-capture-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = store("capture");
    let core = Core::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
    );
    core.runtime
        .attach_cloud(
            CloudSync::new(root.join("cloud.json"), &[CloudProvider::GoogleDrive], &[]).unwrap(),
        )
        .unwrap();
    let services = crate::cloud_sync::CloudServices {
        backup: crate::backup::BackupService::new(store),
        transport: Arc::new(NoopCloud),
        events: Arc::new(NoopEvents),
    };
    let wake = || core.runtime.wake_cloud(services.clone()).unwrap();
    async fn settle(wake: impl Fn() -> bool) {
        // The interval is five seconds, so no capture is due while idle.
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(!wake());
        }
    }
    async fn captures(wake: impl Fn() -> bool) -> bool {
        for _ in 0..100 {
            if wake() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    assert!(wake());
    settle(&wake).await;
    admission_year(&core, core.workspace.read_back().unwrap().revision, 2026).unwrap();
    assert!(captures(&wake).await);

    core.shutdown().await;
    let _ = std::fs::remove_dir_all(root);
}

/// A store filled the way the app fills one: an LMS read with its outline
/// and deadlines, a catalog download, the quarter's calendar, and the
/// person's plans, events, exceptions, a placed course and a session mark.
fn seeded_store(name: &str) -> DataStore {
    use adpt_isct::lms::{validate_course, validate_deadlines, validate_enrolment, LmsDeadlines};
    let store = store(name);
    let enrolment = validate_enrolment(json!({
        "language":"en",
        "courses":[
            {"id":42,"fullname":"演習 / Exercise","shortname":"SYN.T001[2026]","idnumber":"202602440",
             "coursecategory":"3","viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=42","hidden":false},
            {"id":43,"fullname":"講義 / Lecture","shortname":"SYN.T002[2026]","idnumber":"",
             "coursecategory":"3","viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=43","hidden":false}
        ],
        "timetables":[{"year":2026,"quarter":"3","slots":[
            {"courseId":42,"day":1,"period":1},{"courseId":42,"day":1,"period":2},
            {"courseId":43,"day":3,"period":5},{"courseId":43,"day":3,"period":6}],"unscheduled":[]}]
    }))
    .unwrap();
    let outline = validate_course(
        42,
        json!({"section":[{"id":"7","title":"Week 1","number":1}],"cm":[
            {"id":"1001","sectionid":"7","module":"assign","name":"Report","visible":true,"uservisible":true,
             "url":"https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=1001"},
            {"id":"1002","sectionid":"7","module":"resource","name":"Handout","visible":true,"uservisible":true,
             "url":"https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=1002"}]}),
    )
    .unwrap();
    let deadlines = validate_deadlines(LmsDeadlines {
        action_events: vec![json!({"instance":1001,"modulename":"assign","course":{"id":42},
            "timesort":1791900000,"url":"https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=1001"})],
        dated_events: vec![json!({"instance":1001,"modulename":"assign","course":{"id":42},
            "eventtype":"due","timesort":1791900000})],
    })
    .unwrap();
    store
        .write(Some(WorkspaceChangeSource::SchoolPull), |tx| {
            let now = chrono::Utc::now();
            crate::lms_ingest::apply_lms_enrolment(tx, &enrolment, "science:student", now)?;
            crate::lms_ingest::apply_lms_course(tx, &outline, "science:student", now)?;
            crate::lms_ingest::apply_lms_deadlines(tx, &deadlines, "science:student", now)?;
            for (id, date, kind) in [
                ("calendar:2026:quarter-start:3Q", "2026-09-28", "quarter-start"),
                ("calendar:2026:quarter-end:3Q", "2026-12-02", "quarter-end"),
            ] {
                let value = json!({"id":id,"academicYear":2026,"quarter":"3Q","date":date,"type":kind,
                    "label":{"en":kind},"rev":1,"updatedAt":"2026-09-01T00:00:00Z","updatedBy":"reference"});
                tx.execute(
                    "INSERT INTO academic_calendar(id,value,hash,updated_at,updated_by,rev)
                     VALUES (?1,?2,'seed','2026-09-01T00:00:00Z','reference',1)",
                    rusqlite::params![id, value.to_string()],
                )
                .map_err(crate::data::DataError::from)?;
            }
            Ok::<_, crate::lms_ingest::IngestError>(())
        })
        .unwrap();
    store
        .catalog_upsert_offerings(&[json!({
            "id":"202602440", "year":2026, "quarters":["3Q"], "code":"SYN.T001",
            "title":{"en":"Exercise","ja":"演習"}, "instructors":[],
            "credits":{"lecture":1,"exercise":1,"lab":0}, "instructionLanguage":"ja",
            "format":{"kinds":["exercise"]}, "media":{}, "academicUnit":{}, "intensive":false,
            "fetchedAt":{}, "schedule":[{"kind":"weekly","day":1,"periods":[1,2],"room":{"en":"W1"}}],
            "syllabusUrl":"https://syllabus.s.isct.ac.jp/courses/202602440"
        })])
        .unwrap();
    let course = store.load_workspace().unwrap()["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["lmsCourseId"] == 42)
        .unwrap()["id"]
        .clone();
    for (table, value) in [
        (
            "personal_events",
            json!({"kind":"event","termId":"2026-3Q","title":"Gym","room":"",
            "days":[2],"startTime":"07:00","endTime":"08:00"}),
        ),
        (
            "personal_events",
            json!({"kind":"event","termId":"2026-3Q","title":"Club","room":"",
            "days":[3],"startTime":"18:00","endTime":"19:00"}),
        ),
        (
            "calendar_exceptions",
            json!({"date":"2026-10-14","type":"no-class","label":"Trip"}),
        ),
        (
            "schedule_overrides",
            json!({"targetKey":format!("{}-weekly-0-2026-10-05", course.as_str().unwrap()),
            "targetKind":"course","courseId":course,"hidden":false,"mark":"cancelled"}),
        ),
    ] {
        store.put_user_record(table, value).unwrap();
    }
    store
}

/// Queries read only the tables their reads name (`DataStore::view`); each
/// answer must be the one the whole workspace gives.
#[test]
fn every_query_answers_from_its_view_as_from_the_whole_workspace() {
    let store = seeded_store("views");
    let core = Core::new(
        store.clone(),
        Arc::new(SchoolAuthService::new(MemoryVault::default())),
    );
    let snapshot = core.workspace.read_back().unwrap();
    let course = snapshot.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course["lmsCourseId"] == 42)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(snapshot.workspace["materials"]
        .as_array()
        .is_some_and(|rows| !rows.is_empty()));
    let now = "2026-10-05T09:00:00+09:00";
    let queries = [
        json!({"kind":"teachingPeriods"}),
        json!({"kind":"timetableAxis"}),
        json!({"kind":"workspaceOverview","term":"2026-3Q"}),
        json!({"kind":"subscriptionViews"}),
        json!({"kind":"planningWeek","term":"2026-3Q"}),
        json!({"kind":"academicYears","date":"2026-10-05","selectedYear":2026}),
        json!({"kind":"taskView","now":now}),
        json!({"kind":"today","now":now}),
        json!({"kind":"today","now":"2026-10-14T12:00:00+09:00"}),
        json!({"kind":"assignmentBoard","now":now}),
        json!({"kind":"courseDetail","courseId":course}),
        json!({"kind":"courseContent","courseId":course,"now":now}),
        json!({"kind":"assignmentDetail","assignmentId":"lms-assignment:1001"}),
        json!({"kind":"lectureSearch","term":"2026-3Q","query":"handout"}),
        json!({"kind":"scheduleView","term":"2026-3Q","from":"2026-09-28","to":"2026-10-25"}),
        json!({"kind":"dayAgenda","term":"2026-3Q","date":"2026-10-05"}),
        json!({"kind":"calendarState","date":"2026-10-14"}),
        json!({"kind":"quarterRange","term":"2026-3Q"}),
        json!({"kind":"scheduledCourses","term":"2026-3Q"}),
        json!({"kind":"academicYear","year":2026}),
        json!({"kind":"timetableYear","year":2026}),
        json!({"kind":"quarterIcs","term":"2026-3Q","generatedAt":now}),
        json!({"kind":"declaredCredits","term":"2026-3Q"}),
        json!({"kind":"intensiveDays","from":"2026-10-01","to":"2026-10-31","periods":[1,2]}),
        json!({"kind":"glance","now":now}),
        json!({"kind":"notificationPlan","now":now,"horizon":"2026-10-12T09:00:00+09:00"}),
        json!({"kind":"classGroups","today":"2026-10-05"}),
    ];
    for raw in queries {
        let request: crate::schedule::WorkspaceQuery = serde_json::from_value(raw).unwrap();
        let whole = serde_json::to_value(
            crate::schedule::query(&snapshot.workspace, &snapshot.school, request.clone()).unwrap(),
        )
        .unwrap();
        let viewed = core.schedule_query(vec![request.clone()]).unwrap();
        assert_eq!(
            serde_json::to_value(&viewed.values[0]).unwrap(),
            whole,
            "{request:?}"
        );
    }
    let _ = std::fs::remove_dir_all(store.directory());
}
