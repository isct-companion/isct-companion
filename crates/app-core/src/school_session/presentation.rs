use serde::{Deserialize, Serialize};

/// One account as shells show it, derived from what Core keeps about it
/// (`identity::Identity::view`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSessionState {
    /// No credentials are saved.
    NotConfigured,
    /// Nothing stands in the way: Core signs in when it needs to.
    Normal,
    SigningIn,
    /// The school refused the saved credentials: Core does not sign in with
    /// them on its own until they change or the person retries.
    Refused,
    /// The school wants the person on its own page (an emailed code, a new
    /// password, a page Core does not know).
    NeedsWeb,
    /// No verdict (network, an unanswered sign-in, an anomaly); Core tries
    /// again after `retry_at`.
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSignInProblemKind {
    InvalidCredentials,
    InvalidOtp,
    /// The school asks for the App one-time password and the Connect
    /// entered no OTP setup.
    OtpNeeded,
    InvalidMatrix,
    EmailOtp,
    PasswordChange,
    UnknownPage,
    Network,
    /// The password went out and no answer came: counted as submitted.
    Unanswered,
    /// A fresh sign-in was sent straight back to the sign-in page.
    Anomaly,
    /// The system vault refused to give the saved credentials (a locked or
    /// denied keychain): nothing was sent, and it is not the network.
    CredentialStore,
}

impl SchoolSignInProblemKind {
    /// The school wants the person on its own page, not other credentials.
    pub(crate) fn needs_web(self) -> bool {
        matches!(
            self,
            Self::EmailOtp | Self::PasswordChange | Self::UnknownPage
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum SignInRecovery {
    Retry,
    OpenSchoolPage,
    SetUpOtpOnPortal,
    FinishSignOut,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SignInPresentation {
    pub state_key: String,
    pub reason_key: Option<String>,
    pub recovery: Vec<SignInRecovery>,
}

impl SchoolSignInProblemKind {
    fn reason_key(self) -> &'static str {
        match self {
            Self::InvalidCredentials => "failure.theIDOrPasswordWas",
            Self::InvalidOtp => "failure.theOneTimePasswordWas",
            Self::OtpNeeded => "schoolSession.problem.otpNeeded",
            Self::InvalidMatrix => "failure.theMatrixAnswerWasNot",
            Self::EmailOtp => "schoolSession.problem.emailOtp",
            Self::PasswordChange => "schoolSession.problem.passwordChange",
            Self::UnknownPage => "schoolSession.problem.unknownPage",
            Self::Network => "schoolSession.problem.network",
            Self::Unanswered => "schoolSession.problem.unanswered",
            Self::Anomaly => "schoolSession.problem.anomaly",
            Self::CredentialStore => "schoolSession.problem.credentialStore",
        }
    }

    pub(super) fn page_recovery(self) -> Option<SignInRecovery> {
        match self {
            Self::EmailOtp => Some(SignInRecovery::SetUpOtpOnPortal),
            Self::PasswordChange | Self::UnknownPage => Some(SignInRecovery::OpenSchoolPage),
            _ => None,
        }
    }
}

impl SignInPresentation {
    pub(super) fn new(
        state_key: &str,
        reason: Option<SchoolSignInProblemKind>,
        recovery: impl IntoIterator<Item = SignInRecovery>,
    ) -> Self {
        Self {
            state_key: state_key.into(),
            reason_key: reason.map(|kind| kind.reason_key().into()),
            recovery: recovery.into_iter().collect(),
        }
    }

    pub(crate) fn for_profile(
        state: SchoolSessionState,
        reason: Option<SchoolSignInProblemKind>,
        has_retry_at: bool,
        sign_out_pending: bool,
    ) -> Self {
        use SchoolSessionState::*;
        use SignInRecovery::*;
        if sign_out_pending {
            return Self::new("schoolSession.state.signOutPending", None, [FinishSignOut]);
        }
        let page = reason.and_then(SchoolSignInProblemKind::page_recovery);
        let (key, retry) = match state {
            NotConfigured => ("common.notConnected", None),
            Normal => ("schoolSession.state.normal", None),
            SigningIn => ("common.signingIn", None),
            Refused => ("schoolSession.state.refused", Some(Retry)),
            NeedsWeb if page == Some(SetUpOtpOnPortal) => ("schoolSession.state.otpNotSetUp", None),
            NeedsWeb => ("schoolSession.state.needsWeb", Some(Retry)),
            Unavailable if has_retry_at => ("schoolSession.state.unavailableRetry", Some(Retry)),
            Unavailable => ("schoolSession.state.unavailable", Some(Retry)),
        };
        Self::new(key, reason, page.into_iter().chain(retry))
    }
}

/// Why an account is not signed in. A refusal is kept with the credentials
/// it is about (`school_auth`'s refusal record), so a relaunch never sends
/// a refused password again.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SchoolSignInProblem {
    pub kind: SchoolSignInProblemKind,
    /// Core's short diagnostic, for logs; shells show text for `kind`.
    pub message: String,
}

/// How `Core::connect_school` ended, for the Connect form. Only `Connected`
/// saved anything: the entered credentials are the verified ones now and
/// sync to the person's other devices. Every other outcome keeps the
/// form's input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SchoolConnectOutcome {
    Connected {
        presentation: SignInPresentation,
    },
    /// The school refused the entered credentials, OTP or Matrix.
    Refused {
        reason: SchoolSignInProblemKind,
        #[serde(rename = "refusalUnsaved")]
        refusal_unsaved: bool,
        presentation: SignInPresentation,
    },
    /// The school wants the person on its own page.
    NeedsWeb {
        reason: SchoolSignInProblemKind,
        #[serde(rename = "refusalUnsaved")]
        refusal_unsaved: bool,
        presentation: SignInPresentation,
    },
    /// No verdict: offline, the network, out of time.
    Unreachable {
        presentation: SignInPresentation,
    },
    /// The school accepted the credentials and the vault did not save them.
    NotSaved {
        presentation: SignInPresentation,
    },
    /// A newer Connect, a sign-out or a reset stopped it.
    Stopped {
        presentation: SignInPresentation,
    },
}

impl SchoolConnectOutcome {
    pub(super) fn stopped() -> Self {
        Self::Stopped {
            presentation: SignInPresentation::new("schoolSession.connect.stopped", None, []),
        }
    }

    pub(super) fn unreachable(reason: Option<SchoolSignInProblemKind>) -> Self {
        Self::Unreachable {
            presentation: SignInPresentation::new("schoolSession.connect.unreachable", reason, []),
        }
    }
}

#[cfg(test)]
#[path = "../tests/school_session__presentation.rs"]
mod tests;
