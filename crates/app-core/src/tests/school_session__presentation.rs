use super::*;

#[test]
fn sign_in_presentation_leads_to_the_action_each_school_verdict_needs() {
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
    ] {
        let presentation = SignInPresentation::for_profile(state, Some(reason), false, false);
        assert_eq!(presentation.reason_key.as_deref(), Some(key));
        assert_eq!(presentation.recovery, actions);
    }

    let SchoolConnectOutcome::Unreachable { presentation } =
        SchoolConnectOutcome::unreachable(Some(Network))
    else {
        panic!("expected an unreachable connect outcome");
    };
    assert_eq!(
        presentation.reason_key.as_deref(),
        Some("schoolSession.problem.network")
    );
    assert!(presentation.recovery.is_empty());
}
