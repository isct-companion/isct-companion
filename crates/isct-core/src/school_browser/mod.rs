mod assets;
mod engine;
mod policy;
mod port;
#[cfg(feature = "desktop-workspaces")]
mod submission;

pub use assets::{ANDROID_SUPPORT, KYOMU_POPUP, SCHOOL_LINKS, SCHOOL_LOGIN};
pub use engine::SchoolBrowserEngine;
pub use policy::{
    browsing_target, browsing_url, is_login_url, is_school_cookie_domain, sanitize_school_url,
    SCHOOL_HOSTS,
};
pub use port::*;
#[cfg(feature = "desktop-workspaces")]
pub use submission::{SubmissionError, SubmissionMessages};

use crate::{lifecycle::SchoolSite, school_auth::SchoolAuthProfile};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolWebTarget {
    pub site: SchoolSite,
    pub url: String,
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
    pub states: SchoolSessionStates,
    pub kyomu_maintenance: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionStates {
    pub science: SchoolSessionState,
    pub tokyo_tech: SchoolSessionState,
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
