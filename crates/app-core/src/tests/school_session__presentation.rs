use super::*;
use crate::school_session::tests::NoVault;
use crate::school_session::{SchoolAuthService, SchoolSessions};
use std::sync::Arc;

// A revision always carries the view it was numbered with: shells keep the
// highest revision they saw, so a higher revision with an older view (an
// account stuck on "signing in") must be impossible, however publishes and
// reads interleave. An unchanged view keeps its revision.
#[test]
fn a_school_session_revision_always_carries_its_own_view() {
    let sessions = SchoolSessions::new(None, Arc::new(SchoolAuthService::new(NoVault)));
    let first = sessions.snapshot();
    assert_eq!(sessions.snapshot(), first);

    let seen = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for thread in 0..4 {
            let (sessions, seen) = (&sessions, &seen);
            scope.spawn(move || {
                for step in 0..300 {
                    let identity = &sessions.0.account(SchoolAuthProfile::Science).identity;
                    identity.set_configured((thread + step) % 2 == 0);
                    sessions.0.publish();
                    let snapshot = sessions.snapshot();
                    seen.lock().unwrap().push(snapshot);
                }
            });
        }
    });
    let mut views = std::collections::BTreeMap::new();
    for snapshot in seen.into_inner().unwrap() {
        let view = views
            .entry(snapshot.revision)
            .or_insert_with(|| snapshot.profiles.clone());
        assert_eq!(*view, snapshot.profiles, "revision {}", snapshot.revision);
    }
    let last = sessions.snapshot();
    assert!(last.revision > first.revision);
    assert_eq!(
        last.profiles.science.state,
        if sessions
            .0
            .account(SchoolAuthProfile::Science)
            .identity
            .configured()
        {
            SchoolSessionState::Normal
        } else {
            SchoolSessionState::NotConfigured
        }
    );
}

#[test]
fn sign_in_presentation_distinguishes_reasons_and_recovery_actions() {
    use SchoolSessionState::*;
    use SchoolSignInProblemKind::*;
    use SignInRecovery::*;

    for (state, reason, key, actions) in [
        (
            NeedsWeb,
            EmailOtp,
            "schoolSession.problem.emailOtp",
            vec![SetUpOtpOnPortal],
        ),
        (
            Refused,
            OtpNeeded,
            "schoolSession.problem.otpNeeded",
            vec![Retry],
        ),
        (
            NeedsWeb,
            PasswordChange,
            "schoolSession.problem.passwordChange",
            vec![OpenSchoolPage, Retry],
        ),
        (
            NeedsWeb,
            UnknownPage,
            "schoolSession.problem.unknownPage",
            vec![OpenSchoolPage, Retry],
        ),
        (
            Unavailable,
            Network,
            "schoolSession.problem.network",
            vec![Retry],
        ),
        (
            Unavailable,
            Unanswered,
            "schoolSession.problem.unanswered",
            vec![Retry],
        ),
        (
            Unavailable,
            Anomaly,
            "schoolSession.problem.anomaly",
            vec![Retry],
        ),
        (
            Unavailable,
            CredentialStore,
            "schoolSession.problem.credentialStore",
            vec![Retry],
        ),
    ] {
        let presentation = SignInPresentation::for_profile(state, Some(reason), true, false);
        assert_eq!(presentation.reason_key.as_deref(), Some(key));
        assert_eq!(presentation.recovery, actions);
        if state == Unavailable {
            assert_eq!(
                presentation.state_key,
                "schoolSession.state.unavailableRetry"
            );
            let SchoolConnectOutcome::Unreachable { presentation } =
                SchoolConnectOutcome::unreachable(Some(reason))
            else {
                panic!("expected an unreachable connect outcome");
            };
            assert_eq!(presentation.reason_key.as_deref(), Some(key));
        }
    }
    let pending = SignInPresentation::for_profile(NeedsWeb, Some(EmailOtp), false, true);
    assert_eq!(pending.state_key, "schoolSession.state.signOutPending");
    assert_eq!(pending.recovery, [FinishSignOut]);
    assert!(pending.reason_key.is_none());
}
