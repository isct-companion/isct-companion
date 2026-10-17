use super::*;
use crate::{
    events::CoreEvent,
    ports::{EventError, EventSink, SecretError, SecretVault},
    pull_coordinator::PullContext,
    school_auth::SchoolAuthService,
    school_service::{IngestOutcome, IngestState, SchoolExecution, SchoolExecutorError},
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;

#[derive(Default)]
struct EmptyVault;

impl SecretVault for EmptyVault {
    fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
        Ok(())
    }

    fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }

    fn delete(&self, _account: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

struct NoopEvents;

impl EventSink for NoopEvents {
    fn emit(&self, _event: CoreEvent) -> Result<(), EventError> {
        Ok(())
    }
}

struct SequencedExecutor {
    calls: AtomicUsize,
    results: Mutex<VecDeque<Result<Vec<IngestOutcome>, SchoolExecutorError>>>,
}

impl SchoolExecutor for SequencedExecutor {
    fn execute(
        &self,
        _request: SchoolExecution,
        _context: PullContext,
    ) -> crate::school_service::SchoolExecutorFuture<'_> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.results.lock().unwrap().pop_front().unwrap()
        })
    }
}

fn store(name: &str) -> DataStore {
    let path =
        std::env::temp_dir().join(format!("isct-school-refresh-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

#[test]
fn freshness_checks_all_six_kyomu_pages_and_exact_ttl() {
    let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
    let mut runs = json!({});
    for page in [
        "gpa",
        "registrations",
        "declared",
        "profile",
        "grades",
        "requirements",
    ] {
        runs[format!("kyomu:{page}")] = json!(now.to_rfc3339());
    }
    assert!(!due(SchoolAuthProfile::TokyoTech, &runs, now));
    assert!(due(
        SchoolAuthProfile::TokyoTech,
        &runs,
        now + chrono::Duration::seconds(KYOMU_STALE_SECONDS as i64)
    ));
    runs.as_object_mut().unwrap().remove("kyomu:grades");
    assert!(due(SchoolAuthProfile::TokyoTech, &runs, now));
}

#[test]
fn foreground_rechecks_each_known_profile_once() {
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    for profile in [SchoolAuthProfile::TokyoTech, SchoolAuthProfile::Science] {
        refresh
            .event(RefreshEvent {
                kind: RefreshKind::AccountActive,
                profile: Some(profile),
                course_id: None,
                network: None,
            })
            .unwrap();
    }
    let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
    let (profiles, _) = refresh
        .take_ready(&json!({}), now, Instant::now())
        .unwrap()
        .unwrap();
    refresh.0.lock().unwrap().running = false;
    assert_eq!(
        profiles,
        [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
    );
    assert!(refresh
        .take_ready(&json!({}), now, Instant::now())
        .unwrap()
        .is_none());
    refresh.event(RefreshEvent::lifecycle(true)).unwrap();
    assert_eq!(
        refresh
            .take_ready(&json!({}), now, Instant::now())
            .unwrap()
            .unwrap()
            .0,
        [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
    );
    // A running batch defers newly due work instead of interleaving it.
    refresh.event(RefreshEvent::lifecycle(true)).unwrap();
    assert!(refresh
        .take_ready(&json!({}), now, Instant::now())
        .unwrap()
        .is_none());
    drop(RunningBatch(refresh.clone()));
    assert!(refresh
        .take_ready(&json!({}), now, Instant::now())
        .unwrap()
        .is_some());
}

// Protect the admission boundary even when a page never succeeds. Data TTL
// and attempt cooldown must remain separate, including a dropped executor.
#[test]
fn full_pull_lifetime_defers_stale_intents_until_cooldown_expires() {
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    refresh.enqueue_scheduled(SchoolSite::Kyomu).unwrap();
    let now = Utc::now();
    let lease = refresh.begin_full_pull(SchoolSite::Kyomu).unwrap();
    assert!(refresh
        .take_ready(&json!({}), now, Instant::now() + FULL_PULL_COOLDOWN * 2)
        .unwrap()
        .is_none());
    drop(lease);
    let finished = refresh.0.lock().unwrap().full_pulls[&SchoolAuthProfile::TokyoTech]
        .finished
        .unwrap();
    for _ in 0..3 {
        refresh.enqueue_scheduled(SchoolSite::Kyomu).unwrap();
        refresh.event(RefreshEvent::lifecycle(true)).unwrap();
    }
    assert!(refresh
        .take_ready(
            &json!({}),
            now,
            finished + FULL_PULL_COOLDOWN - Duration::from_nanos(1)
        )
        .unwrap()
        .is_none());
    assert!(due(SchoolAuthProfile::TokyoTech, &json!({}), now));
    assert_eq!(
        refresh
            .take_ready(&json!({}), now, finished + FULL_PULL_COOLDOWN)
            .unwrap()
            .unwrap()
            .0,
        [SchoolAuthProfile::TokyoTech]
    );
    drop(RunningBatch(refresh.clone()));
    assert!(refresh
        .take_ready(&json!({}), now, finished + FULL_PULL_COOLDOWN)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn manual_partial_pull_cools_automatic_retries_but_can_be_forced_again() {
    let coordinator = PullCoordinator::default();
    coordinator
        .set_conditions(PullConditions {
            network: PullNetwork::Wifi,
            foreground: true,
            school_visible: false,
            allow_cellular: false,
        })
        .await;
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    let service = SchoolService::new(
        coordinator.clone(),
        Arc::new(SchoolAuthService::new(EmptyVault)),
        Arc::new(NoopEvents),
        refresh.clone(),
    );
    let partial = vec![IngestOutcome {
        site: SchoolSite::Kyomu,
        kind: "registrations".into(),
        state: IngestState::Error,
        changed: 0,
        message: "Page unavailable".into(),
    }];
    let executor = Arc::new(SequencedExecutor {
        calls: AtomicUsize::new(0),
        results: Mutex::new(VecDeque::from([Ok(partial.clone()), Ok(partial)])),
    });
    for expected in 1..=2 {
        let outcomes = service
            .pull(
                executor.clone(),
                SchoolSite::Kyomu,
                None,
                PullOptions::manual(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(outcomes[0].state, IngestState::Error);
        assert_eq!(executor.calls.load(Ordering::Relaxed), expected);
        let snapshots = coordinator.snapshots().await;
        assert_eq!(snapshots.last().unwrap().phase.as_deref(), Some("partial"));
        refresh.enqueue_scheduled(SchoolSite::Kyomu).unwrap();
        refresh.event(RefreshEvent::lifecycle(true)).unwrap();
        assert!(refresh
            .take_ready(&json!({}), Utc::now(), Instant::now())
            .unwrap()
            .is_none());
    }
    assert_eq!(
        refresh.0.lock().unwrap().full_pulls[&SchoolAuthProfile::TokyoTech].running,
        0
    );
}

#[test]
fn detail_debounce_and_failure_retry_are_preserved() {
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    for _ in 0..3 {
        refresh
            .event(RefreshEvent {
                kind: RefreshKind::LectureOpened,
                profile: None,
                course_id: Some(42),
                network: None,
            })
            .unwrap();
    }
    let mut queue = refresh.0.lock().unwrap();
    assert_eq!(queue.details.len(), 1);
    assert!(drain_ready_details(&mut queue, Instant::now()).is_empty());
    assert_eq!(
        drain_ready_details(&mut queue, Instant::now() + Duration::from_secs(3)),
        [42]
    );
    drop(queue);
    refresh.retry_detail(42).unwrap();
    refresh
        .event(RefreshEvent {
            kind: RefreshKind::LectureOpened,
            profile: None,
            course_id: Some(42),
            network: None,
        })
        .unwrap();
    assert_eq!(refresh.0.lock().unwrap().details.len(), 1);
}

#[test]
fn conditions_preserve_shell_facts() {
    let refresh = SchoolRefresh::new(PullNetwork::Cellular);
    refresh
        .event(RefreshEvent {
            kind: RefreshKind::Background,
            profile: None,
            course_id: None,
            network: Some("offline".into()),
        })
        .unwrap();
    assert!(!refresh.foreground());
    assert_eq!(
        refresh.conditions(true, true).unwrap(),
        PullConditions {
            network: PullNetwork::Offline,
            foreground: false,
            school_visible: true,
            allow_cellular: true,
        }
    );
}

#[tokio::test]
async fn tick_retries_a_failed_detail_but_keeps_a_successful_one_seen() {
    let store = store("detail-retry");
    let coordinator = PullCoordinator::default();
    coordinator
        .set_conditions(PullConditions {
            network: PullNetwork::Wifi,
            foreground: true,
            school_visible: false,
            allow_cellular: false,
        })
        .await;
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    let service = SchoolService::new(
        coordinator,
        Arc::new(SchoolAuthService::new(EmptyVault)),
        Arc::new(NoopEvents),
        refresh.clone(),
    );
    let executor = Arc::new(SequencedExecutor {
        calls: AtomicUsize::new(0),
        results: Mutex::new(VecDeque::from([
            Err(SchoolExecutorError::Execution("detail failed".into())),
            Ok(vec![IngestOutcome {
                site: SchoolSite::ScienceLms,
                kind: "course-detail".into(),
                state: IngestState::Ok,
                changed: 1,
                message: "Updated".into(),
            }]),
        ])),
    });
    refresh
        .event(RefreshEvent {
            kind: RefreshKind::LectureOpened,
            profile: None,
            course_id: Some(42),
            network: None,
        })
        .unwrap();
    refresh.0.lock().unwrap().details[0].1 = Instant::now();
    let tasks = RuntimeTasks::default();
    let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
    refresh
        .tick(&inputs, &service, executor.clone(), &tasks)
        .unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(executor.calls.load(Ordering::Relaxed), 1);

    refresh
        .event(RefreshEvent {
            kind: RefreshKind::LectureOpened,
            profile: None,
            course_id: Some(42),
            network: None,
        })
        .unwrap();
    refresh.0.lock().unwrap().details[0].1 = Instant::now();
    let tasks = RuntimeTasks::default();
    let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
    refresh
        .tick(&inputs, &service, executor.clone(), &tasks)
        .unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(executor.calls.load(Ordering::Relaxed), 2);

    refresh
        .event(RefreshEvent {
            kind: RefreshKind::LectureOpened,
            profile: None,
            course_id: Some(42),
            network: None,
        })
        .unwrap();
    assert!(refresh.0.lock().unwrap().details.is_empty());
    let tasks = RuntimeTasks::default();
    let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
    refresh
        .tick(&inputs, &service, executor.clone(), &tasks)
        .unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(executor.calls.load(Ordering::Relaxed), 2);
}

struct RecordingExecutor {
    calls: Mutex<Vec<(SchoolSite, Option<u64>)>>,
    lms_started: tokio::sync::Notify,
    release_lms: tokio::sync::Notify,
}

impl SchoolExecutor for RecordingExecutor {
    fn execute(
        &self,
        request: SchoolExecution,
        _context: PullContext,
    ) -> crate::school_service::SchoolExecutorFuture<'_> {
        Box::pin(async move {
            self.calls
                .lock()
                .unwrap()
                .push((request.site, request.course_id));
            if request.site == SchoolSite::ScienceLms && request.course_id.is_none() {
                self.lms_started.notify_one();
                self.release_lms.notified().await;
            }
            Ok(vec![IngestOutcome {
                site: request.site,
                kind: "lms".into(),
                state: IngestState::Ok,
                changed: 0,
                message: "Updated".into(),
            }])
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn batch_rechecks_later_profiles_after_a_manual_pull_and_keeps_detail_order() {
    let store = store("batch-order");
    let coordinator = PullCoordinator::default();
    coordinator
        .set_conditions(PullConditions {
            network: PullNetwork::Wifi,
            foreground: true,
            school_visible: false,
            allow_cellular: false,
        })
        .await;
    let refresh = SchoolRefresh::new(PullNetwork::Wifi);
    let service = SchoolService::new(
        coordinator.clone(),
        Arc::new(SchoolAuthService::new(EmptyVault)),
        Arc::new(NoopEvents),
        refresh.clone(),
    );
    refresh
        .event(RefreshEvent {
            kind: RefreshKind::LectureOpened,
            profile: None,
            course_id: Some(42),
            network: None,
        })
        .unwrap();
    refresh.0.lock().unwrap().details[0].1 = Instant::now();
    for profile in [SchoolAuthProfile::TokyoTech, SchoolAuthProfile::Science] {
        refresh
            .event(RefreshEvent {
                kind: RefreshKind::AccountActive,
                profile: Some(profile),
                course_id: None,
                network: None,
            })
            .unwrap();
    }
    let executor = Arc::new(RecordingExecutor {
        calls: Mutex::new(Vec::new()),
        lms_started: tokio::sync::Notify::new(),
        release_lms: tokio::sync::Notify::new(),
    });
    let tasks = RuntimeTasks::default();
    let inputs = SchoolRefresh::load_inputs(&store).await.unwrap();
    refresh
        .tick(&inputs, &service, executor.clone(), &tasks)
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), executor.lms_started.notified())
        .await
        .unwrap();
    // Queue manual Kyomu while the batch is still waiting for LMS. Manual
    // priority runs it first; the automatic continuation must join or defer it.
    let manual = {
        let service = service.clone();
        let executor = executor.clone();
        tokio::spawn(async move {
            service
                .pull(
                    executor,
                    SchoolSite::Kyomu,
                    None,
                    PullOptions::manual(),
                    false,
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator
                .snapshots()
                .await
                .iter()
                .any(|row| row.priority == crate::pull_coordinator::PullPriority::Manual)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    executor.release_lms.notify_one();
    manual.await.unwrap().unwrap();
    tasks.close();
    tasks.wait().await;
    assert_eq!(
        *executor.calls.lock().unwrap(),
        [
            (SchoolSite::ScienceLms, None),
            (SchoolSite::Kyomu, None),
            (SchoolSite::ScienceLms, Some(42)),
        ]
    );
    assert!(!refresh.0.lock().unwrap().running);
}
