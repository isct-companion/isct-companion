use super::*;
use futures_util::task::noop_waker_ref;
use serde_json::json;
use std::{
    future::Future,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

fn reference(kind: &str) -> PullTarget {
    PullTarget::Reference {
        kind: kind.into(),
        index_url: "https://example.com/index.json".into(),
        year: 2026,
    }
}

fn lms(course_id: Option<u64>) -> PullTarget {
    PullTarget::School {
        site: SchoolSite::ScienceLms,
        course_id,
    }
}

#[test]
fn reducer_is_total_for_every_state_and_event() {
    let states = [
        PullState::Queued,
        PullState::Checking,
        PullState::Blocked,
        PullState::Running,
        PullState::Applying,
        PullState::RetryWaiting,
        PullState::Succeeded,
        PullState::Failed,
        PullState::Cancelling,
        PullState::Cancelled,
        PullState::Yielding,
    ];
    let events = [
        PullEvent::Enqueue,
        PullEvent::Check,
        PullEvent::Block,
        PullEvent::Ready,
        PullEvent::Start,
        PullEvent::Progress,
        PullEvent::Apply,
        PullEvent::Applied,
        PullEvent::WaitRetry,
        PullEvent::Retry,
        PullEvent::Succeed,
        PullEvent::Fail,
        PullEvent::Cancel,
        PullEvent::Stop,
        PullEvent::Yield,
    ];
    for state in states {
        for event in events.clone() {
            let transition = reduce(state, event);
            assert!(states.contains(&transition.state));
            assert!(!transition.diagnostic.is_empty());
        }
    }
}

#[test]
fn run_without_an_async_runtime_fails_before_admission() {
    let coordinator = PullCoordinator::default();
    let mut request = Box::pin(coordinator.run(
        reference("no-runtime"),
        PullOptions::manual(),
        |_| async { Ok(json!(true)) },
    ));
    let mut context = Context::from_waker(noop_waker_ref());
    assert_eq!(
        Future::poll(request.as_mut(), &mut context),
        Poll::Ready(Err(PullError::RuntimeUnavailable))
    );
    drop(request);
    let state = coordinator.0.state.try_lock().unwrap();
    assert!(state.jobs.is_empty());
    assert!(state.queue.is_empty());
}

#[test]
fn targets_are_validated_and_serialized_for_the_frontend() {
    assert!(PullTarget::School {
        site: SchoolSite::ScienceLms,
        course_id: Some(0),
    }
    .normalized()
    .is_err());
    assert!(PullTarget::School {
        site: SchoolSite::SciencePortal,
        course_id: None,
    }
    .normalized()
    .is_err());
    assert!(PullTarget::Reference {
        kind: "calendar".into(),
        index_url: "https://user:secret@example.com/index.json".into(),
        year: 2026,
    }
    .normalized()
    .is_err());
    assert!(PullTarget::Reference {
        kind: "calendar".into(),
        index_url: "http://example.com/index.json".into(),
        year: 2026,
    }
    .normalized()
    .is_err());
    assert!(PullTarget::Reference {
        kind: "calendar".into(),
        index_url: "https://example.com/index.json".into(),
        year: 1999,
    }
    .normalized()
    .is_err());

    let value = serde_json::to_value(lms(Some(42))).unwrap();
    assert_eq!(value["type"], "school");
    assert_eq!(value["courseId"], 42);
    assert!(value.get("course_id").is_none());
}

#[tokio::test]
async fn same_key_runs_once_and_keeps_all_waiters() {
    let coordinator = PullCoordinator::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(tokio::sync::Notify::new());
    let run = |coordinator: PullCoordinator| {
        let calls = calls.clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            coordinator
                .run(reference("calendar"), PullOptions::automatic(), move |_| {
                    let calls = calls.clone();
                    let gate = gate.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        gate.notified().await;
                        Ok(json!({"ok":true}))
                    }
                })
                .await
        })
    };
    let first = run(coordinator.clone());
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let second = run(coordinator.clone());
    loop {
        let snapshots = coordinator.snapshots().await;
        if snapshots[0].waiter_count == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    gate.notify_waiters();
    assert_eq!(first.await.unwrap().unwrap(), json!({"ok":true}));
    assert_eq!(second.await.unwrap().unwrap(), json!({"ok":true}));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn successor_waits_for_cancelling_run_and_keeps_its_key() {
    let coordinator = PullCoordinator::default();
    let old_started = Arc::new(tokio::sync::Notify::new());
    let old_finish = Arc::new(tokio::sync::Notify::new());
    let old = {
        let coordinator = coordinator.clone();
        let old_started = old_started.clone();
        let old_finish = old_finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("successor"),
                    PullOptions::automatic(),
                    move |_| {
                        let old_started = old_started.clone();
                        let old_finish = old_finish.clone();
                        async move {
                            old_started.notify_one();
                            old_finish.notified().await;
                            Ok(json!("old"))
                        }
                    },
                )
                .await
        })
    };
    old_started.notified().await;
    let old_id = coordinator.snapshots().await[0].id.clone();
    assert!(coordinator.cancel(&old_id).await);

    let successor_started = Arc::new(AtomicBool::new(false));
    let successor_finish = Arc::new(tokio::sync::Notify::new());
    let successor = {
        let coordinator = coordinator.clone();
        let successor_started = successor_started.clone();
        let successor_finish = successor_finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(reference("successor"), PullOptions::manual(), move |_| {
                    let successor_started = successor_started.clone();
                    let successor_finish = successor_finish.clone();
                    async move {
                        successor_started.store(true, Ordering::SeqCst);
                        successor_finish.notified().await;
                        Ok(json!("new"))
                    }
                })
                .await
        })
    };
    loop {
        if coordinator.snapshots().await.len() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    assert!(!successor_started.load(Ordering::SeqCst));
    old_finish.notify_one();
    assert_eq!(old.await.unwrap().unwrap_err(), PullError::Cancelled);
    while !successor_started.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }

    let joined = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(reference("successor"), PullOptions::manual(), |_| async {
                    panic!("successor key was lost")
                })
                .await
        })
    };
    loop {
        if coordinator
            .snapshots()
            .await
            .iter()
            .any(|snapshot| snapshot.state == PullState::Running && snapshot.waiter_count == 2)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    successor_finish.notify_one();
    assert_eq!(successor.await.unwrap().unwrap(), json!("new"));
    assert_eq!(joined.await.unwrap().unwrap(), json!("new"));
}

#[tokio::test]
async fn queued_manual_join_promotes_executor_context_priority() {
    let coordinator = PullCoordinator::default();
    coordinator
        .set_conditions(PullConditions {
            foreground: false,
            ..PullConditions::default()
        })
        .await;
    let observed = Arc::new(Mutex::new(None));
    let automatic = {
        let coordinator = coordinator.clone();
        let observed = observed.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(Some(99)), PullOptions::automatic(), move |context| {
                    let observed = observed.clone();
                    async move {
                        *observed.lock().await = Some(context.priority());
                        Ok(json!("promoted"))
                    }
                })
                .await
        })
    };
    loop {
        if coordinator
            .snapshots()
            .await
            .iter()
            .any(|snapshot| snapshot.state == PullState::Blocked)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    let manual = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(Some(99)), PullOptions::manual(), |_| async {
                    panic!("manual join must reuse the queued executor")
                })
                .await
        })
    };
    assert_eq!(automatic.await.unwrap().unwrap(), json!("promoted"));
    assert_eq!(manual.await.unwrap().unwrap(), json!("promoted"));
    assert_eq!(*observed.lock().await, Some(PullPriority::Manual));
}

#[tokio::test]
async fn running_join_upgrades_login_first_priority_and_scope_for_the_executor() {
    let coordinator = PullCoordinator::default();
    let started = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let saved = Arc::new(Mutex::new(None));
    let automatic = {
        let coordinator = coordinator.clone();
        let started = started.clone();
        let finish = finish.clone();
        let saved = saved.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    lms(None),
                    PullOptions {
                        current_quarter: true,
                        ..PullOptions::automatic()
                    },
                    move |context| {
                        let started = started.clone();
                        let finish = finish.clone();
                        let saved = saved.clone();
                        async move {
                            *saved.lock().await = Some(context.clone());
                            started.notify_one();
                            finish.notified().await;
                            Ok(json!({ "loginFirst": context.login_first() }))
                        }
                    },
                )
                .await
        })
    };
    started.notified().await;
    let context = saved.lock().await.clone().unwrap();
    assert!(!context.login_first());
    assert!(context.current_quarter());
    assert_eq!(context.priority(), PullPriority::Automatic);
    let forced = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    lms(None),
                    PullOptions {
                        login_first: true,
                        ..PullOptions::manual()
                    },
                    |_| async { panic!("a join must reuse the running executor") },
                )
                .await
        })
    };
    while !context.login_first() {
        tokio::task::yield_now().await;
    }
    assert_eq!(context.priority(), PullPriority::Manual);
    // A caller that wants every quarter widens a current-quarter job.
    assert!(!context.current_quarter());
    finish.notify_one();
    assert_eq!(
        automatic.await.unwrap().unwrap(),
        json!({ "loginFirst": true })
    );
    assert_eq!(
        forced.await.unwrap().unwrap(),
        json!({ "loginFirst": true })
    );
}

#[tokio::test]
async fn unchanged_conditions_do_not_publish_duplicate_snapshots() {
    let coordinator = PullCoordinator::default();
    let receiver = coordinator.subscribe();
    coordinator.set_conditions(PullConditions::default()).await;
    assert!(!receiver.has_changed().unwrap());
    coordinator
        .set_conditions(PullConditions {
            network: PullNetwork::Offline,
            ..PullConditions::default()
        })
        .await;
    assert!(receiver.has_changed().unwrap());
}

#[tokio::test]
async fn automatic_school_admission_matches_the_previous_condition_matrix() {
    fn previously_allowed(conditions: PullConditions) -> bool {
        conditions.foreground
            && conditions.network != PullNetwork::Offline
            && (conditions.network != PullNetwork::Cellular || conditions.allow_cellular)
            && !conditions.school_visible
    }

    let coordinator = PullCoordinator::default();
    let mut course_id = 1u64;
    for network in [
        PullNetwork::Unknown,
        PullNetwork::Offline,
        PullNetwork::Wifi,
        PullNetwork::Cellular,
    ] {
        for foreground in [false, true] {
            for school_visible in [false, true] {
                for allow_cellular in [false, true] {
                    let conditions = PullConditions {
                        network,
                        foreground,
                        school_visible,
                        allow_cellular,
                    };
                    coordinator.set_conditions(conditions).await;
                    let calls = Arc::new(AtomicUsize::new(0));
                    let task = {
                        let coordinator = coordinator.clone();
                        let calls = calls.clone();
                        let target = lms(Some(course_id));
                        tokio::spawn(async move {
                            coordinator
                                .run(target, PullOptions::automatic(), move |_| {
                                    let calls = calls.clone();
                                    async move {
                                        calls.fetch_add(1, Ordering::SeqCst);
                                        Ok(json!("ran"))
                                    }
                                })
                                .await
                        })
                    };
                    let id = loop {
                        if let Some(snapshot) = coordinator
                            .snapshots()
                            .await
                            .into_iter()
                            .find(|snapshot| snapshot.target == lms(Some(course_id)))
                        {
                            if previously_allowed(conditions)
                                || snapshot.state == PullState::Blocked
                            {
                                break snapshot.id;
                            }
                        }
                        tokio::task::yield_now().await;
                    };
                    if previously_allowed(conditions) {
                        assert_eq!(task.await.unwrap().unwrap(), json!("ran"));
                        assert_eq!(calls.load(Ordering::SeqCst), 1);
                    } else {
                        assert_eq!(calls.load(Ordering::SeqCst), 0);
                        assert!(coordinator.cancel(&id).await);
                        assert_eq!(task.await.unwrap().unwrap_err(), PullError::Cancelled);
                    }
                    course_id += 1;
                }
            }
        }
    }

    coordinator
        .set_conditions(PullConditions {
            network: PullNetwork::Offline,
            foreground: false,
            school_visible: true,
            allow_cellular: false,
        })
        .await;
    let manual = coordinator
        .run(lms(Some(course_id)), PullOptions::manual(), |_| async {
            Ok(json!("manual"))
        })
        .await;
    assert_eq!(manual.unwrap(), json!("manual"));
}

#[tokio::test]
async fn stale_context_cannot_update_a_retried_run() {
    let coordinator = PullCoordinator::default();
    let attempts = Arc::new(AtomicUsize::new(0));
    let contexts = Arc::new(Mutex::new(Vec::<PullContext>::new()));
    let second_started = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let task = {
        let coordinator = coordinator.clone();
        let attempts = attempts.clone();
        let contexts = contexts.clone();
        let second_started = second_started.clone();
        let finish = finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("stale-context"),
                    PullOptions {
                        retry_limit: 1,
                        ..PullOptions::automatic()
                    },
                    move |context| {
                        let attempts = attempts.clone();
                        let contexts = contexts.clone();
                        let second_started = second_started.clone();
                        let finish = finish.clone();
                        async move {
                            contexts.lock().await.push(context);
                            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                                Err(PullError::Execution("retry".into()))
                            } else {
                                second_started.notify_one();
                                finish.notified().await;
                                Ok(json!("done"))
                            }
                        }
                    },
                )
                .await
        })
    };
    second_started.notified().await;
    let saved = contexts.lock().await.clone();
    assert_eq!(saved.len(), 2);
    assert!(!saved[0].phase("stale").await);
    assert!(saved[1].phase("current").await);
    finish.notify_one();
    assert_eq!(task.await.unwrap().unwrap(), json!("done"));
}

#[tokio::test]
async fn full_lms_and_course_detail_keep_distinct_work() {
    let coordinator = PullCoordinator::default();
    coordinator
        .set_conditions(PullConditions {
            foreground: false,
            ..PullConditions::default()
        })
        .await;
    let detail_calls = Arc::new(AtomicUsize::new(0));
    let detail = {
        let coordinator = coordinator.clone();
        let calls = detail_calls.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(Some(42)), PullOptions::automatic(), move |_| {
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(json!("detail"))
                    }
                })
                .await
        })
    };
    tokio::task::yield_now().await;
    let full = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(None), PullOptions::manual(), |_| async {
                    Ok(json!("full"))
                })
                .await
        })
    };
    assert_eq!(full.await.unwrap().unwrap(), json!("full"));
    assert!(!detail.is_finished());
    assert_eq!(detail_calls.load(Ordering::SeqCst), 0);
    coordinator.set_conditions(PullConditions::default()).await;
    assert_eq!(detail.await.unwrap().unwrap(), json!("detail"));
    assert_eq!(detail_calls.load(Ordering::SeqCst), 1);

    let coordinator = PullCoordinator::default();
    let started = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let detail_calls = Arc::new(AtomicUsize::new(0));
    let full = {
        let coordinator = coordinator.clone();
        let started = started.clone();
        let finish = finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(None), PullOptions::manual(), move |_| {
                    let started = started.clone();
                    let finish = finish.clone();
                    async move {
                        started.notify_one();
                        finish.notified().await;
                        Ok(json!("running-full"))
                    }
                })
                .await
        })
    };
    started.notified().await;
    let detail = {
        let coordinator = coordinator.clone();
        let detail_calls = detail_calls.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(Some(7)), PullOptions::manual(), move |_| {
                    let detail_calls = detail_calls.clone();
                    async move {
                        detail_calls.fetch_add(1, Ordering::SeqCst);
                        Ok(json!("running-detail"))
                    }
                })
                .await
        })
    };
    loop {
        if coordinator
            .snapshots()
            .await
            .iter()
            .any(|snapshot| snapshot.target == lms(Some(7)))
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(!detail.is_finished());
    assert_eq!(detail_calls.load(Ordering::SeqCst), 0);
    finish.notify_one();
    assert_eq!(full.await.unwrap().unwrap(), json!("running-full"));
    assert_eq!(detail.await.unwrap().unwrap(), json!("running-detail"));
    assert_eq!(detail_calls.load(Ordering::SeqCst), 1);

    let coordinator = PullCoordinator::default();
    let finish = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let enqueue_full = |coordinator: PullCoordinator| {
        let finish = finish.clone();
        let calls = calls.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(None), PullOptions::manual(), move |_| {
                    let finish = finish.clone();
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        finish.notified().await;
                        Ok(json!("deduplicated-full"))
                    }
                })
                .await
        })
    };
    let first = enqueue_full(coordinator.clone());
    while calls.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let second = enqueue_full(coordinator.clone());
    loop {
        if coordinator.snapshots().await[0].waiter_count == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    finish.notify_one();
    assert_eq!(first.await.unwrap().unwrap(), json!("deduplicated-full"));
    assert_eq!(second.await.unwrap().unwrap(), json!("deduplicated-full"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn school_lane_prefers_manual_then_preserves_fifo() {
    let coordinator = PullCoordinator::default();
    let gate = Arc::new(tokio::sync::Notify::new());
    let started = Arc::new(tokio::sync::Notify::new());
    let order = Arc::new(Mutex::new(Vec::new()));
    let first = {
        let coordinator = coordinator.clone();
        let gate = gate.clone();
        let started = started.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(Some(1)), PullOptions::manual(), move |_| {
                    let gate = gate.clone();
                    let started = started.clone();
                    async move {
                        started.notify_one();
                        gate.notified().await;
                        Ok(json!("first"))
                    }
                })
                .await
        })
    };
    started.notified().await;

    let enqueue = |target, priority, label: &'static str| {
        let coordinator = coordinator.clone();
        let order = order.clone();
        tokio::spawn(async move {
            coordinator
                .run(target, priority, move |_| {
                    let order = order.clone();
                    async move {
                        order.lock().await.push(label);
                        Ok(json!(label))
                    }
                })
                .await
        })
    };
    let automatic_a = enqueue(
        PullTarget::School {
            site: SchoolSite::Kyomu,
            course_id: None,
        },
        PullOptions::automatic(),
        "automatic-a",
    );
    let automatic_b = enqueue(lms(Some(2)), PullOptions::automatic(), "automatic-b");
    let manual = enqueue(lms(Some(3)), PullOptions::manual(), "manual");
    loop {
        if coordinator
            .snapshots()
            .await
            .iter()
            .filter(|snapshot| snapshot.state == PullState::Queued)
            .count()
            == 3
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    gate.notify_one();
    first.await.unwrap().unwrap();
    manual.await.unwrap().unwrap();
    automatic_a.await.unwrap().unwrap();
    automatic_b.await.unwrap().unwrap();
    assert_eq!(
        *order.lock().await,
        ["manual", "automatic-a", "automatic-b"]
    );
}

#[tokio::test]
async fn retry_uses_the_factory_and_queued_cancel_settles() {
    let coordinator = PullCoordinator::default();
    let attempts = Arc::new(AtomicUsize::new(0));
    let result = coordinator
        .run(
            reference("retry"),
            PullOptions {
                retry_limit: 1,
                ..PullOptions::automatic()
            },
            {
                let attempts = attempts.clone();
                move |_| {
                    let attempts = attempts.clone();
                    async move {
                        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            Err(PullError::Execution("try again".into()))
                        } else {
                            Ok(json!("recovered"))
                        }
                    }
                }
            },
        )
        .await;
    assert_eq!(result.unwrap(), json!("recovered"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let outage_attempts = Arc::new(AtomicUsize::new(0));
    let error = coordinator
        .run(
            reference("maintenance"),
            PullOptions {
                retry_limit: 3,
                ..PullOptions::automatic()
            },
            {
                let attempts = outage_attempts.clone();
                move |_| {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    async { Err(PullError::Maintenance("Live school notice".into())) }
                }
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error, PullError::Maintenance("Live school notice".into()));
    assert_eq!(outage_attempts.load(Ordering::SeqCst), 1);

    let delayed = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("cancel"),
                    PullOptions {
                        delay: Duration::from_secs(60),
                        ..PullOptions::automatic()
                    },
                    |_| async { Ok(json!("too late")) },
                )
                .await
        })
    };
    let id = loop {
        if let Some(snapshot) = coordinator
                .snapshots()
                .await
                .into_iter()
                .find(|snapshot| matches!(&snapshot.target, PullTarget::Reference { kind, .. } if kind == "cancel"))
            {
                break snapshot.id;
            }
        tokio::task::yield_now().await;
    };
    assert!(coordinator.cancel(&id).await);
    assert_eq!(delayed.await.unwrap().unwrap_err(), PullError::Cancelled);
}

#[tokio::test]
async fn http_lane_runs_two_jobs_and_executor_panic_settles_waiter() {
    let coordinator = PullCoordinator::default();
    let gate = Arc::new(tokio::sync::Notify::new());
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut jobs = Vec::new();
    for kind in ["calendar", "exams"] {
        let coordinator = coordinator.clone();
        let gate = gate.clone();
        let running = running.clone();
        let peak = peak.clone();
        jobs.push(tokio::spawn(async move {
            coordinator
                .run(reference(kind), PullOptions::automatic(), move |_| {
                    let gate = gate.clone();
                    let running = running.clone();
                    let peak = peak.clone();
                    async move {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        gate.notified().await;
                        running.fetch_sub(1, Ordering::SeqCst);
                        Ok(json!(kind))
                    }
                })
                .await
        }));
    }
    while running.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    gate.notify_waiters();
    for job in jobs {
        job.await.unwrap().unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), 2);

    let error = coordinator
        .run(reference("panic"), PullOptions::manual(), |_| async {
            panic!("executor bug")
        })
        .await
        .unwrap_err();
    assert_eq!(error, PullError::ExecutorPanicked);
}

#[tokio::test]
async fn running_cancel_waits_for_executor_and_late_progress_is_rejected() {
    let coordinator = PullCoordinator::default();
    let started = Arc::new(tokio::sync::Notify::new());
    let stop = Arc::new(tokio::sync::Notify::new());
    let saved_context = Arc::new(Mutex::new(None));
    let task = {
        let coordinator = coordinator.clone();
        let started = started.clone();
        let stop = stop.clone();
        let saved_context = saved_context.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("calendar"),
                    PullOptions::manual(),
                    move |context| {
                        let started = started.clone();
                        let stop = stop.clone();
                        let saved_context = saved_context.clone();
                        async move {
                            *saved_context.lock().await = Some(context.clone());
                            started.notify_one();
                            stop.notified().await;
                            Ok(json!("late success"))
                        }
                    },
                )
                .await
        })
    };
    started.notified().await;
    let snapshot = coordinator.snapshots().await;
    let id = snapshot[0].id.clone();
    assert!(coordinator.cancel(&id).await);
    assert_eq!(
        coordinator.snapshots().await[0].state,
        PullState::Cancelling
    );
    stop.notify_one();
    assert_eq!(task.await.unwrap().unwrap_err(), PullError::Cancelled);
    let context = saved_context.lock().await.clone().unwrap();
    assert!(!context.phase("late").await);
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Cancelled);
}

#[tokio::test]
async fn shutdown_drains_applying_cancels_running_and_releases_captures() {
    let coordinator = PullCoordinator::default();
    let capture = Arc::new(());
    let weak = Arc::downgrade(&capture);
    let entered = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let applying = {
        let coordinator = coordinator.clone();
        let entered = entered.clone();
        let finish = finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("committing"),
                    PullOptions::manual(),
                    move |context| {
                        let capture = capture.clone();
                        let entered = entered.clone();
                        let finish = finish.clone();
                        async move {
                            assert!(context.applying().await);
                            entered.notify_one();
                            finish.notified().await;
                            assert!(!context.is_cancelled());
                            drop(capture);
                            Ok(json!("committed"))
                        }
                    },
                )
                .await
        })
    };
    entered.notified().await;
    let (sent, received) = oneshot::channel();
    let sent = Arc::new(Mutex::new(Some(sent)));
    let running = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("fetching"),
                    PullOptions::manual(),
                    move |context| {
                        let sent = sent.clone();
                        async move {
                            sent.lock().await.take().unwrap().send(context.clone()).ok();
                            while !context.is_cancelled() {
                                tokio::task::yield_now().await;
                            }
                            assert!(!context.applying().await);
                            Ok(json!("discarded"))
                        }
                    },
                )
                .await
        })
    };
    let late = received.await.unwrap();
    let shutdown = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move { coordinator.shutdown().await })
    };
    assert_eq!(running.await.unwrap(), Err(PullError::Cancelled));
    assert!(!shutdown.is_finished());
    assert_eq!(
        coordinator
            .run(reference("late"), PullOptions::manual(), |_| async {
                Ok(Value::Null)
            })
            .await,
        Err(PullError::CoordinatorStopped)
    );
    finish.notify_one();
    assert_eq!(applying.await.unwrap().unwrap(), json!("committed"));
    shutdown.await.unwrap();
    assert!(!late.phase("late").await);
    assert!(
        weak.upgrade().is_none(),
        "history must not retain executor captures"
    );
    assert!(coordinator.0.tasks.is_closed());
    assert_eq!(coordinator.0.state.lock().await.http_running, 0);
}

#[tokio::test]
async fn suspend_resume_and_shutdown_of_delayed_work_are_finite() {
    let coordinator = PullCoordinator::default();
    coordinator.suspend().await;
    let mut states = coordinator.subscribe();
    let task = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(reference("paused"), PullOptions::manual(), |_| async {
                    Ok(json!(42))
                })
                .await
        })
    };
    loop {
        states.changed().await.unwrap();
        if states
            .borrow()
            .iter()
            .any(|row| row.blocked_by.as_deref() == Some("suspended"))
        {
            break;
        }
    }
    assert!(!task.is_finished());
    coordinator.resume().await;
    assert_eq!(task.await.unwrap().unwrap(), json!(42));
    let task = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            let mut options = PullOptions::manual();
            options.delay = Duration::from_secs(3600);
            coordinator
                .run(reference("delayed"), options, |_| async {
                    panic!("cancelled delay executed")
                })
                .await
        })
    };
    loop {
        states.changed().await.unwrap();
        if states
            .borrow()
            .iter()
            .any(|row| row.state == PullState::Queued)
        {
            break;
        }
    }
    tokio::time::timeout(Duration::from_secs(1), coordinator.shutdown())
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(PullError::Cancelled));
    coordinator.resume().await;
    coordinator.shutdown().await;
}

#[tokio::test]
async fn applying_rejects_cancel_and_terminal_history_is_bounded() {
    let coordinator = PullCoordinator::default();
    let applying = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let task = {
        let coordinator = coordinator.clone();
        let applying = applying.clone();
        let finish = finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    reference("applying"),
                    PullOptions::manual(),
                    move |context| {
                        let applying = applying.clone();
                        let finish = finish.clone();
                        async move {
                            assert!(context.applying().await);
                            assert!(context.applying().await);
                            applying.notify_one();
                            finish.notified().await;
                            Ok(json!("applied"))
                        }
                    },
                )
                .await
        })
    };
    applying.notified().await;
    let id = coordinator.snapshots().await[0].id.clone();
    assert!(!coordinator.cancel(&id).await);
    finish.notify_one();
    assert_eq!(task.await.unwrap().unwrap(), json!("applied"));

    for index in 0..=TERMINAL_HISTORY_LIMIT {
        coordinator
            .run(
                reference(&format!("history-{index}")),
                PullOptions::manual(),
                |_| async { Ok(json!(true)) },
            )
            .await
            .unwrap();
    }
    let snapshots = coordinator.snapshots().await;
    assert_eq!(snapshots.len(), TERMINAL_HISTORY_LIMIT);
    assert!(snapshots.iter().all(|snapshot| matches!(
        snapshot.state,
        PullState::Succeeded | PullState::Failed | PullState::Cancelled
    )));
}

#[tokio::test]
async fn applied_page_reopens_cancellation_before_the_next_page() {
    let coordinator = PullCoordinator::default();
    let committed = Arc::new(tokio::sync::Notify::new());
    let saved = Arc::new(Mutex::new(None));
    let task = {
        let coordinator = coordinator.clone();
        let committed = committed.clone();
        let saved = saved.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(None), PullOptions::manual(), move |context| {
                    let committed = committed.clone();
                    let saved = saved.clone();
                    async move {
                        assert!(context.applying().await);
                        assert!(context.applied().await);
                        *saved.lock().await = Some(context.clone());
                        committed.notify_one();
                        while !context.is_cancelled() {
                            tokio::task::yield_now().await;
                        }
                        assert!(!context.applying().await);
                        Ok(json!("first page only"))
                    }
                })
                .await
        })
    };
    committed.notified().await;
    let id = coordinator.snapshots().await[0].id.clone();
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Running);
    assert!(coordinator.cancel(&id).await);
    assert_eq!(task.await.unwrap(), Err(PullError::Cancelled));
    let context = saved.lock().await.clone().unwrap();
    assert!(!context.applied().await);
}

#[tokio::test]
async fn shutdown_during_a_page_commit_cancels_the_following_pages() {
    let coordinator = PullCoordinator::default();
    let entered = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let task = {
        let coordinator = coordinator.clone();
        let entered = entered.clone();
        let finish = finish.clone();
        tokio::spawn(async move {
            coordinator
                .run(lms(None), PullOptions::manual(), move |context| {
                    let entered = entered.clone();
                    let finish = finish.clone();
                    async move {
                        assert!(context.applying().await);
                        entered.notify_one();
                        finish.notified().await;
                        assert!(!context.is_cancelled());
                        assert!(!context.applied().await);
                        assert!(context.is_cancelled());
                        Ok(json!("first page only"))
                    }
                })
                .await
        })
    };
    entered.notified().await;
    let shutdown = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move { coordinator.shutdown().await })
    };
    while !coordinator.0.state.lock().await.closed {
        tokio::task::yield_now().await;
    }
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Applying);
    finish.notify_one();
    assert_eq!(task.await.unwrap(), Err(PullError::Cancelled));
    shutdown.await.unwrap();
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Cancelled);
}

#[tokio::test]
async fn finishing_an_unknown_run_still_releases_its_lane() {
    let coordinator = PullCoordinator::default();
    occupy_lane(&mut *coordinator.0.state.lock().await, PullLane::School);
    coordinator
        .finish("pull-missing".into(), 7, PullLane::School, Ok(Value::Null))
        .await;
    assert!(!coordinator.0.state.lock().await.school_running);
    assert_eq!(
        coordinator
            .run(lms(None), PullOptions::manual(), |_| async {
                Ok(json!("ran"))
            })
            .await
            .unwrap(),
        json!("ran")
    );
}

#[tokio::test]
async fn partial_failure_keeps_payload_and_marks_snapshot_failed() {
    let coordinator = PullCoordinator::default();
    let value = coordinator
        .run(
            reference("partial"),
            PullOptions::manual(),
            |context| async move {
                assert!(context.outcome_error("one course could not be read").await);
                Ok(json!({"successful": 3, "failed": 1}))
            },
        )
        .await
        .unwrap();
    assert_eq!(value, json!({"successful": 3, "failed": 1}));
    let snapshot = coordinator.snapshots().await.pop().unwrap();
    assert_eq!(snapshot.state, PullState::Failed);
    assert_eq!(
        snapshot.last_error.as_deref(),
        Some("one course could not be read")
    );
}
