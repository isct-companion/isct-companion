use super::*;
use serde_json::json;

fn pull(id: &str, site: SchoolSite, priority: PullPriority, state: PullState) -> PullSnapshot {
    PullSnapshot {
        id: id.into(),
        target: PullTarget::School {
            site,
            course_id: None,
        },
        priority,
        state,
        phase: None,
        blocked_by: None,
        attempt: 1,
        waiter_count: 0,
        last_error: None,
    }
}

fn blocked(mut snapshot: PullSnapshot, by: &str) -> PullSnapshot {
    snapshot.state = PullState::Blocked;
    snapshot.blocked_by = Some(by.into());
    snapshot
}

fn pulls(tracker: &mut SchoolStatusTracker, pulls: Vec<PullSnapshot>) -> bool {
    tracker.observe(&CoreEvent::PullStateChanged(pulls))
}

fn lms_progress(phase: &str, completed: usize, total: usize) -> CoreEvent {
    CoreEvent::SchoolPullProgress(SchoolPullProgress {
        site: SchoolSite::ScienceLms,
        phase: phase.into(),
        completed,
        total,
        course_id: 7,
        course_title: "Linear Algebra".into(),
        term: "2026-3Q".into(),
        item_title: "Week 3".into(),
    })
}

fn kyomu(kind: &str, state: IngestState) -> CoreEvent {
    CoreEvent::SchoolIngest(IngestOutcome {
        site: SchoolSite::Kyomu,
        kind: kind.into(),
        state,
        changed: 0,
        message: "教務Webはメンテナンス中です".into(),
    })
}

fn busy(tracker: &SchoolStatusTracker) -> (SchoolSite, u8, PullStepKind) {
    match tracker.status().headline {
        SchoolHeadline::Busy {
            site,
            progress,
            step,
            ..
        } => (site, progress, step.kind),
        other => panic!("expected a busy headline, got {other:?}"),
    }
}

#[test]
fn lms_progress_moves_through_its_bands_and_restarts_with_the_next_pull() {
    let lms = SchoolSite::ScienceLms;
    let mut tracker = SchoolStatusTracker::default();
    pulls(
        &mut tracker,
        vec![pull("a", lms, PullPriority::Manual, PullState::Running)],
    );
    // Before LMS reports a step the pull is reading the course list.
    assert_eq!(busy(&tracker), (lms, 5, PullStepKind::Catalog));

    tracker.observe(&lms_progress("course", 1, 2));
    assert_eq!(busy(&tracker), (lms, 17, PullStepKind::Course));
    let SchoolHeadline::Busy { step, .. } = tracker.status().headline else {
        unreachable!()
    };
    assert_eq!(
        (
            step.completed,
            step.total,
            step.course_title.as_str(),
            step.item_title.as_str()
        ),
        (1, 2, "Linear Algebra", "Week 3")
    );

    // An earlier band never moves the ring backwards.
    tracker.observe(&lms_progress("catalog", 0, 1));
    assert_eq!(busy(&tracker).1, 17);
    // An unknown phase is ignored rather than guessed.
    tracker.observe(&lms_progress("unknown", 1, 1));
    assert_eq!(busy(&tracker), (lms, 17, PullStepKind::Catalog));
    // A step with nothing to count is complete.
    tracker.observe(&lms_progress("timetable", 0, 0));
    assert_eq!(busy(&tracker).1, 95);

    pulls(
        &mut tracker,
        vec![pull("a", lms, PullPriority::Manual, PullState::Applying)],
    );
    assert_eq!(busy(&tracker), (lms, 95, PullStepKind::Saving));

    pulls(
        &mut tracker,
        vec![
            pull("a", lms, PullPriority::Manual, PullState::Succeeded),
            pull("b", lms, PullPriority::Manual, PullState::Running),
        ],
    );
    assert_eq!(busy(&tracker), (lms, 5, PullStepKind::Catalog));
}

#[test]
fn automatic_pulls_stay_in_the_background_and_say_why_they_wait() {
    let mut tracker = SchoolStatusTracker::default();
    let lms = pull(
        "a",
        SchoolSite::ScienceLms,
        PullPriority::Automatic,
        PullState::Queued,
    );
    pulls(&mut tracker, vec![blocked(lms.clone(), "cellular")]);
    let status = tracker.status();
    assert_eq!(
        status.headline,
        SchoolHeadline::Background {
            site: SchoolSite::ScienceLms,
            waiting: Some(WaitReason::Cellular),
        }
    );
    assert_eq!(
        status.sources[0].activity,
        SourceActivity::Waiting {
            reason: WaitReason::Cellular,
            manual: false,
        }
    );
    assert_eq!(status.sources[1].activity, SourceActivity::Idle);

    // A running pull is shown before a blocked one of the same source, and
    // 教務Web is named before LMS.
    pulls(
        &mut tracker,
        vec![
            blocked(lms.clone(), "mystery"),
            pull(
                "b",
                SchoolSite::ScienceLms,
                PullPriority::Automatic,
                PullState::Running,
            ),
            pull(
                "c",
                SchoolSite::Kyomu,
                PullPriority::Automatic,
                PullState::Queued,
            ),
        ],
    );
    let status = tracker.status();
    assert_eq!(
        status.headline,
        SchoolHeadline::Background {
            site: SchoolSite::Kyomu,
            waiting: Some(WaitReason::Queued),
        }
    );
    assert!(matches!(
        status.sources[0].activity,
        SourceActivity::Running {
            manual: false,
            progress: 5,
            ..
        }
    ));

    pulls(&mut tracker, vec![blocked(lms.clone(), "mystery")]);
    assert_eq!(
        tracker.status().headline,
        SchoolHeadline::Background {
            site: SchoolSite::ScienceLms,
            waiting: Some(WaitReason::Other),
        }
    );

    // A course's running detail pull is shown ahead of an older blocked full
    // pull; once it succeeds the blocked one is shown again.
    let mut detail = pull(
        "d",
        SchoolSite::ScienceLms,
        PullPriority::Manual,
        PullState::Running,
    );
    detail.target = PullTarget::School {
        site: SchoolSite::ScienceLms,
        course_id: Some(42),
    };
    let full = blocked(lms, "schoolVisible");
    pulls(&mut tracker, vec![full.clone(), detail.clone()]);
    assert!(matches!(
        tracker.status().sources[0].activity,
        SourceActivity::Running { manual: true, .. }
    ));
    detail.state = PullState::Succeeded;
    pulls(&mut tracker, vec![full, detail]);
    assert_eq!(
        tracker.status().headline,
        SchoolHeadline::Background {
            site: SchoolSite::ScienceLms,
            waiting: Some(WaitReason::SchoolVisible),
        }
    );
    assert_eq!(tracker.status().sources[1].activity, SourceActivity::Idle);
}

#[test]
fn a_running_manual_pull_names_the_manual_pull_queued_behind_it() {
    let (lms, kyomu) = (SchoolSite::ScienceLms, SchoolSite::Kyomu);
    let mut tracker = SchoolStatusTracker::default();
    pulls(
        &mut tracker,
        vec![
            pull("a", lms, PullPriority::Manual, PullState::Running),
            pull("b", kyomu, PullPriority::Manual, PullState::Queued),
        ],
    );
    assert_eq!(
        tracker.status().headline,
        SchoolHeadline::Busy {
            site: lms,
            progress: 5,
            step: empty_step(PullStepKind::Catalog),
            waiting: None,
            queued: Some(kyomu),
        }
    );

    // Core's automatic pull holds the school lane; the person's queued
    // request must not hide it behind a static estimate.
    pulls(
        &mut tracker,
        vec![
            pull("a", lms, PullPriority::Automatic, PullState::Running),
            pull("b", kyomu, PullPriority::Manual, PullState::Queued),
        ],
    );
    assert!(matches!(
        tracker.status().headline,
        SchoolHeadline::Busy {
            site: SchoolSite::ScienceLms,
            queued: Some(SchoolSite::Kyomu),
            ..
        }
    ));

    // Alone, a waiting manual pull is still the person's: busy, with the reason.
    pulls(
        &mut tracker,
        vec![blocked(
            pull("b", kyomu, PullPriority::Manual, PullState::Queued),
            "offline",
        )],
    );
    assert_eq!(
        tracker.status().headline,
        SchoolHeadline::Busy {
            site: kyomu,
            progress: 5,
            step: empty_step(PullStepKind::Kyomu),
            waiting: Some(WaitReason::Offline),
            queued: None,
        }
    );
}

#[test]
fn a_failure_shows_once_nothing_runs_and_a_later_success_clears_it() {
    let lms = SchoolSite::ScienceLms;
    let mut failed = pull("a", lms, PullPriority::Manual, PullState::Failed);
    failed.last_error = Some("LMS did not answer.".into());
    let mut tracker = SchoolStatusTracker::default();

    pulls(&mut tracker, vec![failed.clone()]);
    let problem = SchoolProblem::Failed {
        detail: Some("LMS did not answer.".into()),
    };
    assert_eq!(
        tracker.status().headline,
        SchoolHeadline::Problem {
            site: lms,
            problem: problem.clone(),
        }
    );
    assert_eq!(
        tracker.status().sources[0].activity,
        SourceActivity::Problem { problem }
    );
    assert_eq!(tracker.status().sources[1].activity, SourceActivity::Idle);

    let mut partial = failed.clone();
    partial.phase = Some("partial".into());
    pulls(&mut tracker, vec![partial]);
    assert!(matches!(
        tracker.status().headline,
        SchoolHeadline::Problem {
            problem: SchoolProblem::Partial { .. },
            ..
        }
    ));

    pulls(
        &mut tracker,
        vec![
            failed.clone(),
            pull(
                "b",
                SchoolSite::Kyomu,
                PullPriority::Automatic,
                PullState::Running,
            ),
        ],
    );
    assert!(matches!(
        tracker.status().headline,
        SchoolHeadline::Background { .. }
    ));

    pulls(
        &mut tracker,
        vec![
            failed,
            pull("c", lms, PullPriority::Automatic, PullState::Succeeded),
        ],
    );
    assert_eq!(tracker.status().headline, SchoolHeadline::Idle);
}

#[test]
fn kyomu_maintenance_and_page_reads_drive_the_kyomu_source() {
    let mut tracker = SchoolStatusTracker::default();
    // An extractor result may have written an import run.
    assert!(tracker.observe(&kyomu("service", IngestState::Maintenance)));
    let maintenance = SchoolProblem::Maintenance {
        detail: "教務Webはメンテナンス中です".into(),
    };
    assert_eq!(
        tracker.status().headline,
        SchoolHeadline::Problem {
            site: SchoolSite::Kyomu,
            problem: maintenance.clone(),
        }
    );
    assert_eq!(
        tracker.status().sources[1].activity,
        SourceActivity::Problem {
            problem: maintenance
        }
    );

    pulls(
        &mut tracker,
        vec![pull(
            "k",
            SchoolSite::Kyomu,
            PullPriority::Manual,
            PullState::Running,
        )],
    );
    // The service probe clears maintenance but reads no page.
    tracker.observe(&kyomu("service", IngestState::Ok));
    assert_eq!(busy(&tracker), (SchoolSite::Kyomu, 5, PullStepKind::Kyomu));
    tracker.observe(&kyomu("grades", IngestState::Ok));
    assert_eq!(busy(&tracker).1, 35);
    tracker.observe(&kyomu("gpa", IngestState::Unavailable));
    assert_eq!(busy(&tracker).1, 55);
    tracker.observe(&kyomu("registrations", IngestState::Error));
    assert_eq!(busy(&tracker).1, 55);
    for kind in ["requirements", "declared", "exams"] {
        tracker.observe(&kyomu(kind, IngestState::Ok));
    }
    assert_eq!(busy(&tracker).1, 95);

    pulls(
        &mut tracker,
        vec![pull(
            "k",
            SchoolSite::Kyomu,
            PullPriority::Manual,
            PullState::Succeeded,
        )],
    );
    assert_eq!(tracker.status().headline, SchoolHeadline::Idle);
}

#[test]
fn sign_in_reports_become_typed_states_on_their_source() {
    let mut tracker = SchoolStatusTracker::default();
    let report = |profile, state: &str, message: &str| {
        CoreEvent::SchoolLogin(SchoolLoginEvent {
            profile,
            state: state.into(),
            message: message.into(),
        })
    };
    tracker.observe(&report(SchoolAuthProfile::Science, "progress", "otp"));
    tracker.observe(&report(
        SchoolAuthProfile::TokyoTech,
        "connected",
        "Connected",
    ));
    let status = tracker.status();
    assert_eq!(
        status.sources[0].sign_in,
        Some(SignInState::Step {
            step: SignInStep::Otp
        })
    );
    assert_eq!(status.sources[1].sign_in, Some(SignInState::Connected));

    for (state, message, expected) in [
        (
            "progress",
            "captcha",
            SignInState::Step {
                step: SignInStep::Other,
            },
        ),
        (
            "needs-user",
            "Enter the Matrix code.",
            SignInState::NeedsUser {
                message: "Enter the Matrix code.".into(),
            },
        ),
        (
            "maintenance",
            "Under maintenance",
            SignInState::Failed {
                message: "Under maintenance".into(),
            },
        ),
    ] {
        tracker.observe(&report(SchoolAuthProfile::Science, state, message));
        assert_eq!(tracker.status().sources[0].sign_in, Some(expected));
    }
}

#[test]
fn last_updated_takes_the_newest_run_of_each_source_only() {
    let mut tracker = SchoolStatusTracker::default();
    assert_eq!(tracker.status().sources[0].last_updated, None);
    tracker.set_runs(json!({
        "lms:all": "2026-10-01T03:00:00+00:00",
        "lms:summary": "2026-10-02T03:00:00+00:00",
        "lms:page": "2026-10-09T03:00:00+00:00",
        "kyomu:grades": "2026-10-03T03:00:00+00:00",
        "kyomu:gpa": "2026-10-04T03:00:00+00:00",
        "reference:calendar": "2026-10-10T03:00:00+00:00",
    }));
    let status = tracker.status();
    assert_eq!(
        status.sources[0].last_updated.as_deref(),
        Some("2026-10-02T03:00:00+00:00")
    );
    assert_eq!(
        status.sources[1].last_updated.as_deref(),
        Some("2026-10-04T03:00:00+00:00")
    );
}

#[test]
fn only_a_school_pull_that_just_finished_asks_for_the_runs_again() {
    let lms = SchoolSite::ScienceLms;
    let mut tracker = SchoolStatusTracker::default();
    assert!(!pulls(
        &mut tracker,
        vec![pull("a", lms, PullPriority::Manual, PullState::Running)]
    ));
    let done = vec![pull("a", lms, PullPriority::Manual, PullState::Succeeded)];
    assert!(pulls(&mut tracker, done.clone()));
    assert!(!pulls(&mut tracker, done));

    let mut reference = pull("r", lms, PullPriority::Automatic, PullState::Succeeded);
    reference.target = PullTarget::Reference {
        kind: "calendar".into(),
        index_url: "https://example.test".into(),
        year: 2026,
    };
    assert!(!pulls(&mut tracker, vec![reference]));
    assert!(!tracker.observe(&lms_progress("course", 1, 2)));
}
