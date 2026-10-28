mod engine;
mod policy;
mod port;
mod submission;

pub use adpt_isct::scripts::{SCHOOL_LOGIN, SCHOOL_STOP};
pub use engine::SchoolBrowserEngine;
pub use policy::{
    automation_target, browsing_target, browsing_url, download_permitted, is_login_url,
    is_school_cookie_domain, restore_record, school_file_url, DenyReason, ViewRole,
};
pub use port::*;
pub use submission::{
    SubmissionError, SubmissionFile, SubmissionMessages, MAX_SUBMISSION_BYTES, MAX_SUBMISSION_FILES,
};

use crate::{lifecycle::SchoolSite, school_auth::SchoolAuthProfile};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebTarget {
    pub site: SchoolSite,
    pub url: String,
}

/// Something about the page the person is looking at that the app shows
/// next to the address, outside the school page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SchoolWebNotice {
    /// The page did not load. `url` is the address it was going to, offered
    /// for Retry and Open in browser.
    #[serde(rename_all = "camelCase")]
    LoadFailed {
        url: Option<String>,
        reason: LoadFailure,
    },
    /// A reload would have sent a form again; the page was opened with GET.
    Reopened { url: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebResult {
    pub site: SchoolSite,
    pub url: String,
    pub phase: &'static str,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolPageCustomization {
    pub css: String,
    pub javascript: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolLoginEvent {
    pub profile: SchoolAuthProfile,
    pub state: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSessionState {
    Pending,
    Active,
    Maintenance,
    Failed,
    NotConfigured,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionEvent {
    pub profile: SchoolAuthProfile,
    pub state: SchoolSessionState,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionSnapshot {
    pub revision: u64,
    pub profiles: SchoolSessionProfiles,
    pub kyomu_maintenance: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionProfiles {
    pub science: SchoolProfileSession,
    pub tokyo_tech: SchoolProfileSession,
}

/// One school account's session as shells render it: the state, why the
/// last sign-in failed, and whether Core will sign in on its own.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolProfileSession {
    pub state: SchoolSessionState,
    pub problem: Option<SchoolSignInProblem>,
    pub auto: SchoolAutoSignIn,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSignInProblem {
    pub kind: SchoolSignInProblemKind,
    /// The school page's own words, or Core's when the page said nothing.
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSignInProblemKind {
    InvalidCredentials,
    Locked,
    InvalidOtp,
    InvalidMatrix,
    /// The sign-in page showed a warning this run did not cause.
    PageWarning,
    NeedsUser,
    /// The page did not answer in time or the view went away.
    Unavailable,
}

/// Whether the hidden session check and visible pages may sign in with the
/// saved credentials.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolAutoSignIn {
    #[default]
    Allowed,
    /// Tried again once a later one-time-password window has started.
    RetryLater,
    /// Waits for new credentials: from Settings, a connect, or another device.
    Paused,
}

/// What `Core::connect_school` ended with. `verified` says the school accepted
/// exactly these credentials in this run, so they now sync to other devices.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolConnectResult {
    pub state: SchoolSessionState,
    pub problem: Option<SchoolSignInProblem>,
    pub verified: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SchoolSessionLifecycleKind {
    Launch,
    Focus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionLifecycle {
    pub kind: SchoolSessionLifecycleKind,
    pub visible: bool,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolBrowserError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Platform(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    Maintenance(String),
    #[error("{0}")]
    SiteChanged(String),
    #[error("{0}")]
    Timeout(String),
    #[error("Pull cancelled.")]
    Cancelled,
    #[error("School job paused for visible navigation.")]
    Preempted,
    #[error("The school browser has stopped.")]
    Stopped,
}

impl From<BrowserError> for SchoolBrowserError {
    fn from(error: BrowserError) -> Self {
        match error {
            BrowserError::Stopped => Self::Stopped,
            BrowserError::Unavailable(message) | BrowserError::Dispatch(message) => {
                Self::Platform(message)
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/school_browser.rs"]
mod tests;
