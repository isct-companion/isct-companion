use super::*;

/// Each school verdict leads to the action it needs, and every key the
/// presentation names is copy each catalog has: shells show it as is.
#[test]
fn sign_in_presentation_leads_to_the_action_each_school_verdict_needs() {
    use SchoolSessionState::*;
    use SchoolSignInProblemKind::*;
    use SignInRecovery::*;

    let catalogs = [
        include_str!("../../../../resources/locales/en.json"),
        include_str!("../../../../resources/locales/ja.json"),
        include_str!("../../../../resources/locales/zh.json"),
    ]
    .map(|catalog| serde_json::from_str::<serde_json::Value>(catalog).unwrap());
    let in_catalogs = |presentation: &SignInPresentation| {
        for key in std::iter::once(&presentation.state_key).chain(&presentation.reason_key) {
            for catalog in &catalogs {
                assert!(catalog[key].is_string(), "{key}");
            }
        }
    };

    for (state, reason, actions) in [
        (NeedsWeb, EmailOtp, vec![SetUpOtpOnPortal]),
        (Refused, OtpNeeded, vec![Retry]),
        (NeedsWeb, PasswordChange, vec![OpenSchoolPage, Retry]),
        (NeedsWeb, UnknownPage, vec![OpenSchoolPage, Retry]),
        (Unavailable, Unanswered, vec![Retry]),
        (Unavailable, Anomaly, vec![Retry]),
        (Unavailable, CredentialStore, vec![Retry]),
    ] {
        let presentation =
            SignInPresentation::for_profile(state, Some(reason), state == Unavailable, false);
        assert!(presentation.reason_key.is_some(), "{reason:?}");
        in_catalogs(&presentation);
        assert_eq!(presentation.recovery, actions, "{reason:?}");
    }
    let pending = SignInPresentation::for_profile(Normal, None, false, true);
    in_catalogs(&pending);
    assert!(pending.reason_key.is_none());

    let SchoolConnectOutcome::Unreachable { presentation } =
        SchoolConnectOutcome::unreachable(Some(Network))
    else {
        panic!("expected an unreachable connect outcome");
    };
    assert!(presentation.reason_key.is_some());
    in_catalogs(&presentation);
    assert!(presentation.recovery.is_empty());
}
