//! One user-requested upload to an assignment's draft file area. Never submits the form.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::workspaces::{
    SubmissionFile, WorkspaceError, MAX_SUBMISSION_BYTES, MAX_SUBMISSION_FILES,
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use url::Url;

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum SubmissionError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Browser(String),
    #[error("The workspace folder no longer exists")]
    WorkspaceMissing,
}

impl From<WorkspaceError> for SubmissionError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::Input(message) => Self::InvalidInput(message),
            WorkspaceError::Storage(message) | WorkspaceError::Archive(message) => {
                Self::Storage(message)
            }
            WorkspaceError::Missing => Self::WorkspaceMissing,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionMessages {
    pub waiting: String,
    pub queued: String,
    pub failed: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PendingSubmission {
    request: String,
    pub url: String,
    files: Vec<SubmissionFile>,
    messages: SubmissionMessages,
    #[serde(skip)]
    expires: Instant,
}

impl PendingSubmission {
    pub fn new(
        raw: &str,
        files: Vec<SubmissionFile>,
        messages: SubmissionMessages,
    ) -> Result<Self, SubmissionError> {
        let url = submission_url(raw)?;
        if files.is_empty()
            || files.len() > MAX_SUBMISSION_FILES
            || files.iter().map(|file| file.data.len()).sum::<usize>()
                > MAX_SUBMISSION_BYTES.div_ceil(3) * 4 + MAX_SUBMISSION_FILES * 4
        {
            return Err(SubmissionError::InvalidInput(
                "Choose up to 32 files totalling at most 20 MiB.".into(),
            ));
        }
        Ok(Self {
            request: uuid::Uuid::new_v4().to_string(),
            url,
            files,
            messages,
            expires: Instant::now() + Duration::from_secs(300),
        })
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }

    pub fn matches(&self, raw: &str) -> bool {
        let Ok(url) = Url::parse(raw) else {
            return false;
        };
        url.query_pairs().filter(|(key, _)| key == "id").count() == 1
            && url.query_pairs().filter(|(key, _)| key == "action").count() == 1
            && url
                .query_pairs()
                .any(|(key, value)| key == "action" && value == "editsubmission")
            && submission_url(raw).is_ok_and(|target| target == self.url)
    }

    pub fn script(&self) -> Result<String, SubmissionError> {
        let request = serde_json::to_string(self)
            .map_err(|error| SubmissionError::Storage(error.to_string()))?;
        Ok(format!(
            "{}({request});",
            include_str!("assets/assignment-upload.js")
        ))
    }
}

fn submission_url(raw: &str) -> Result<String, SubmissionError> {
    let invalid = || {
        SubmissionError::InvalidInput(
            "This assignment has no supported LMS submission page.".into(),
        )
    };
    let mut url = Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str() != Some("lms.s.isct.ac.jp")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().ends_with("/mod/assign/view.php")
    {
        return Err(invalid());
    }
    if url.query_pairs().filter(|(key, _)| key == "id").count() != 1 {
        return Err(invalid());
    }
    let id = url
        .query_pairs()
        .find(|(key, _)| key == "id")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(invalid)?;
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .append_pair("id", &id.to_string())
        .append_pair("action", "editsubmission");
    Ok(url.to_string())
}
