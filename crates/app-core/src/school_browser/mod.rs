mod engine;
mod policy;
mod port;
mod submission;

pub use engine::SchoolBrowserEngine;
pub use policy::{
    attachment_shown_in_place, automation_target, browsing_target, browsing_url,
    download_permitted, is_login_url, restore_record, school_file_url, DenyReason,
};
pub(crate) use policy::{domain_under, school_domains};
pub use port::*;
pub use submission::{
    SubmissionError, SubmissionFile, SubmissionMessages, MAX_SUBMISSION_BYTES, MAX_SUBMISSION_FILES,
};

use crate::lifecycle::SchoolSite;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebTarget {
    pub site: SchoolSite,
    pub url: String,
}

/// A one-time note about the page the person is looking at, shown next to
/// the address for a while. A failed load or a sign-in problem is not one:
/// it is part of [`SchoolWebPage`] for as long as it describes the page.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SchoolWebNotice {
    /// A reload the page or the engine started itself (never the App's
    /// Reload) would have sent a form again; the page was opened with GET.
    Reopened { url: String },
}

/// Why a web operation (signing a view in, on an App link or on Reload at
/// the sign-in page) stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolWebProblem {
    /// The school could not be reached (offline, the network, maintenance,
    /// a pause after a failure). No password was sent.
    Unreachable,
    /// The school refused the saved password: Settings updates it.
    Rejected,
    /// The sign-in could not be written into the web view's store.
    Storage,
    /// The page still asks for a sign-in: Core signed in and the page did
    /// not take it, or the school wants the person on its own page.
    StillSignIn,
}

/// The sign-in state of the visit in front of a site.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SchoolWebAccess {
    /// Core is signing the view in: the view is covered, and the person may
    /// cancel to sign in on the page themselves.
    SigningIn,
    /// The last web operation of the view stopped, or ended with something
    /// to say. It stays until the view shows another page.
    Problem { problem: SchoolWebProblem },
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
    /// A web operation runs for the view, or what its last one left to say.
    pub access: Option<SchoolWebAccess>,
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
    /// The site's start page as it is: Core writes no identity and starts no
    /// web operation, so the person signs in on the page by hand. The
    /// Connect form offers it for the Science Tokyo portal when the school
    /// sends its one-time password by email, to set up App OTP there.
    Unsigned,
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

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum SchoolBrowserError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Platform(String),
    #[error("{0}")]
    Storage(String),
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
