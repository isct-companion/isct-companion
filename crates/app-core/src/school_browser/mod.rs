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

/// A one-time note about the page the person is looking at, shown next to
/// the address for a while. A failed load is not one: it is part of
/// [`SchoolWebPage`] for as long as it describes the page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SchoolWebNotice {
    /// A reload would have sent a form again; the page was opened with GET.
    Reopened { url: String },
}

/// The page the person sees of a site: the frontmost popup's when one is
/// open over the view, the view's own otherwise. Every surface that shows the
/// view renders this, and only this, for its address and controls.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebPage {
    pub site: SchoolSite,
    /// Grows with every change; a surface keeps the newest it has seen.
    pub revision: u64,
    /// `None` until the view has a page.
    pub url: Option<String>,
    /// Popups open over the view (hosts with inline popups).
    pub popups: usize,
    /// What the app's Back does: step back, or close a popup at the start of
    /// its history.
    pub back: SchoolWebStep,
    pub forward: SchoolWebStep,
    /// The page in front did not load.
    pub notice: Option<SchoolWebFailure>,
}

/// Whether the app's Back or Forward does something for the page in front.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolWebStep {
    Yes,
    /// Core sends nothing; the button is disabled.
    No,
    /// The host does not report history; the engine is asked anyway.
    Unknown,
}

/// A load of the page in front that failed. It stays until that view or
/// popup shows a later page, Core sends it a navigation, it closes, or it
/// fails again.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebFailure {
    /// The address it was going to, offered for Retry and Open in browser.
    pub url: Option<String>,
    pub reason: LoadFailure,
}

/// How a surface brings a site's view on screen. Every open records the
/// surface as the site's presenter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SchoolWebEntry {
    /// As the person left it, popups included; the restored or start page
    /// when the view has none.
    Resume,
    /// An app link: the popups over the view close and the view itself goes
    /// to the URL, unless it is there already.
    Go { url: String },
    /// Back to what the surface showed before: `Resume` when the surface was
    /// the last to present the view, `Go` when another one did meanwhile.
    Return { url: String },
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
