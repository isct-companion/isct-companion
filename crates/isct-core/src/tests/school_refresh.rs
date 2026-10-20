use super::*;
use crate::{
    ports::{SecretError, SecretVault},
    pull_coordinator::PullContext,
    school_auth::SchoolAuthService,
    school_service::{IngestOutcome, IngestState, SchoolExecution, SchoolExecutorError},
};
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

fn fresh(dataset: Dataset, key: &str, fetched_at: DateTime<Utc>) -> KeyState {
    KeyState {
        dataset: dataset.name().into(),
        key: key.into(),
        account: "tokyotech:student".into(),
        generation: 0,
        fetched_at: Some(fetched_at),
        validator: None,
        outcome: Some(sync_state::Outcome::Complete),
        message: None,
        failures: 0,
        next_eligible: None,
        last_error: None,
        last_used: None,
    }
}

fn kyomu_pages(fetched_at: DateTime<Utc>) -> Vec<KeyState> {
    crate::school_parser::kyomu::PAGES
        .iter()
        .map(|(_, page)| fresh(Dataset::KyomuPage, page, fetched_at))
        .collect()
}

#[test]
fn freshness_checks_every_key_of_a_profile_with_its_ttl_and_backoff() {
    let now: DateTime<Utc> = "2026-10-12T00:00:00Z".parse().unwrap();
    let ttl = sync_state::ttl(Dataset::KyomuPage, Term::Current).unwrap();
    let mut pages = kyomu_pages(now);
    assert!(!due(SchoolAuthProfile::TokyoTech, &pages, now));
    assert!(due(SchoolAuthProfile::TokyoTech, &pages, now + ttl));
    // A page backing off after a failure does not make the profile due.
    let grades = pages.iter_mut().find(|state| state.key == "grades").unwrap();
    grades.fetched_at = None;
    grades.next_eligible = Some(now + chrono::Duration::minutes(5));
    assert!(!due(SchoolAuthProfile::TokyoTech, &pages, now));
    // Once the backoff ends the missing page is due again.
    assert!(due(
        SchoolAuthProfile::TokyoTech,
        &pages,
        now + chrono::Duration::minutes(5)
    ));
    pages.retain(|state| state.key != "grades");
    assert!(due(SchoolAuthProfile::TokyoTech, &pages, now));

    // Science needs both its course list and its deadlines; a fresh course
    // key does not stand in for them.
    let mut lms = vec![
        fresh(Dataset::LmsEnrolment, "all", now),
        fresh(Dataset::LmsCourse, "42", now),
    ];
    assert!(due(SchoolAuthProfile::Science, &lms, now));
    lms.push(fresh(Dataset::LmsDeadlines, "all", now));
    assert!(!due(SchoolAuthProfile::Science, &lms, now));
    let deadlines = sync_state::ttl(Dataset::LmsDeadlines, Term::Current).unwrap();
    assert!(due(SchoolAuthProfile::Science, &lms, now + deadlines));
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
        .take_ready(&[], now, Instant::now())
        .unwrap()
        .unwrap();
    refresh.0.lock().unwrap().running = false;
    assert_eq!(
        profiles,
        [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
    );
    assert!(refresh
        .take_ready(&[], now, Instant::now())
        .unwrap()
        .is_none());
    refresh.event(RefreshEvent::lifecycle(true)).unwrap();
    assert_eq!(
        refresh
            .take_ready(&[], now, Instant::now())
            .unwrap()
            .unwrap()
            .0,
        [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
    );
    // A running batch defers newly due work instead of interleaving it.
    refresh.event(RefreshEvent::lifecycle(true)).unwrap();
    assert!(refresh
        .take_ready(&[], now, Instant::now())
        .unwrap()
        .is_none());
    drop(RunningBatch(refresh.clone()));
    assert!(refresh
        .take_ready(&[], now, Instant::now())
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
        .take_ready(&[], now, Instant::now() + FULL_PULL_COOLDOWN * 2)
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
            &[],
            now,
            finished + FULL_PULL_COOLDOWN - Duration::from_nanos(1)
        )
        .unwrap()
        .is_none());
    assert!(due(SchoolAuthProfile::TokyoTech, &[], now));
    assert_eq!(
        refresh
            .take_ready(&[], now, finished + FULL_PULL_COOLDOWN)
            .unwrap()
            .unwrap()
            .0,
        [SchoolAuthProfile::TokyoTech]
    );
    drop(RunningBatch(refresh.clone()));
    assert!(refresh
        .take_ready(&[], now, finished + FULL_PULL_COOLDOWN)
        .unwrap()
        .is_none());

    // Each pull that leaves a page missing triples the wait, up to six hours,
    // so a permanently broken page cannot cost a sign-in every five minutes.
    let retry_after = |misses: u32| {
        let mut queue = refresh.0.lock().unwrap();
        let activity = queue
            .full_pulls
            .get_mut(&SchoolAuthProfile::TokyoTech)
            .unwrap();
        activity.misses = misses;
        activity.cooldown()
    };
    assert_eq!(retry_after(1), FULL_PULL_COOLDOWN);
    assert_eq!(retry_after(2), FULL_PULL_COOLDOWN * 3);
    assert_eq!(retry_after(3), FULL_PULL_COOLDOWN * 9);
    assert_eq!(retry_after(30), FULL_PULL_MAX_COOLDOWN);
    refresh.enqueue_scheduled(SchoolSite::Kyomu).unwrap();
    assert!(refresh
        .take_ready(&[], now, finished + FULL_PULL_COOLDOWN * 9)
        .unwrap()
        .is_none());
    drop(refresh.begin_full_pull(SchoolSite::Kyomu).unwrap());
    assert_eq!(
        refresh.0.lock().unwrap().full_pulls[&SchoolAuthProfile::TokyoTech].misses,
        31
    );

    // Observing every page fresh forgets the misses.
    refresh.enqueue_scheduled(SchoolSite::Kyomu).unwrap();
    assert!(refresh
        .take_ready(&kyomu_pages(now), now, Instant::now())
        .unwrap()
        .is_none());
    assert_eq!(
        refresh.0.lock().unwrap().full_pulls[&SchoolAuthProfile::TokyoTech].misses,
        0
    );
    assert_eq!(retry_after(0), FULL_PULL_COOLDOWN);
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
            .take_ready(&[], Utc::now(), Instant::now())
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
    let keys = [
        Key::new(Dataset::LmsCourse, "42"),
        Key::new(Dataset::LmsAssign, "42"),
    ];
    store
        .write(None, |transaction| {
            for key in &keys {
                sync_state::commit(
                    transaction,
                    key,
                    sync_state::Commit {
                        account: "science:student",
                        validator: None,
                        outcome: sync_state::Outcome::Complete,
                        message: None,
                    },
                    Utc::now(),
                )?;
            }
            Ok::<_, crate::data::DataError>(())
        })
        .unwrap();
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
    let inputs = refresh.load_inputs(&store).await.unwrap();
    // Opening the lecture marks both of its keys used, for gc.
    for state in store.freshness(None).unwrap() {
        assert!(state.last_used.is_some(), "{}/{}", state.dataset, state.key);
    }
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
    let inputs = refresh.load_inputs(&store).await.unwrap();
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
    let inputs = refresh.load_inputs(&store).await.unwrap();
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
    let inputs = refresh.load_inputs(&store).await.unwrap();
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
