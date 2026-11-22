//! One user-requested upload to an assignment's draft file area. Never submits the form.
//! Every shell hands files here: desktop from an assignment workspace, the
//! phones from what was shared into the app.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::{
    data::{DataError, DataStore},
    schedule::{self, AssignmentDetail, HandInAvailability, WorkspaceQuery},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};
use url::Url;

pub const MAX_SUBMISSION_FILES: usize = 32;
pub const MAX_SUBMISSION_BYTES: usize = 20 * 1024 * 1024;
const REQUEST_PARAM: &str = "isctDraftRequest";

/// One file for the draft area, base64 as the page's uploader takes it.
#[derive(Clone, Debug, Serialize)]
pub struct SubmissionFile {
    pub name: String,
    pub data: String,
}

impl SubmissionFile {
    /// A file a shell read, under the name the draft area shows.
    pub fn from_bytes(name: String, bytes: &[u8]) -> Self {
        Self {
            name,
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}

#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum SubmissionError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Browser(String),
    #[error("This assignment is no longer available for draft upload.")]
    AssignmentUnavailable,
    #[error("Only files can be handed in.")]
    FilesOnly,
    #[error("Upload assistance supports at most 20 MiB.")]
    TooLarge,
    #[cfg(feature = "desktop-workspaces")]
    #[error("The workspace folder no longer exists")]
    WorkspaceMissing,
}

#[cfg(feature = "desktop-workspaces")]
impl From<crate::workspaces::WorkspaceError> for SubmissionError {
    fn from(error: crate::workspaces::WorkspaceError) -> Self {
        use crate::workspaces::WorkspaceError;
        match error {
            WorkspaceError::Input(message) => Self::InvalidInput(message),
            WorkspaceError::Storage(message) | WorkspaceError::Archive(message) => {
                Self::Storage(message)
            }
            WorkspaceError::Missing => Self::WorkspaceMissing,
        }
    }
}

/// Reads the same current detail used for the assignment page and the desktop workspace.
pub(crate) fn assignment_detail_in(
    store: &DataStore,
    assignment: &str,
) -> Result<(Arc<serde_json::Value>, Option<AssignmentDetail>), DataError> {
    let fields = crate::workspace_changes::loaded_fields(schedule::reads_of(&[
        WorkspaceQuery::AssignmentDetail {
            assignment_id: assignment.into(),
        },
    ]));
    let (workspace, _, _) = store.view(&fields)?;
    let detail =
        schedule::assignment_detail(&workspace, assignment, crate::clock::now().fixed_offset());
    Ok((workspace, detail))
}

pub(crate) fn draft_page(detail: &AssignmentDetail) -> Option<String> {
    if detail.hand_in_availability == HandInAvailability::PrepareDraft {
        detail.source_url.clone()
    } else {
        None
    }
}

pub(crate) fn current_draft_page(
    store: &DataStore,
    assignment: &str,
) -> Result<String, SubmissionError> {
    let (_, detail) = assignment_detail_in(store, assignment)
        .map_err(|error| SubmissionError::Storage(error.to_string()))?;
    let page = detail
        .as_ref()
        .and_then(draft_page)
        .ok_or(SubmissionError::AssignmentUnavailable)?;
    submission_url(&page).map(|url| url.to_string())
}

/// Reads an entire selection without changing its files or any other stored data.
pub(crate) fn selected_files(
    paths: Vec<(PathBuf, String)>,
) -> Result<Vec<SubmissionFile>, SubmissionError> {
    if paths.is_empty() || paths.len() > MAX_SUBMISSION_FILES {
        return Err(SubmissionError::InvalidInput(
            "Choose up to 32 files totalling at most 20 MiB.".into(),
        ));
    }
    let storage = |error: std::io::Error| SubmissionError::Storage(error.to_string());
    let mut remaining = MAX_SUBMISSION_BYTES;
    let mut files = Vec::with_capacity(paths.len());
    for (path, name) in paths {
        if !path.symlink_metadata().map_err(storage)?.is_file() {
            return Err(SubmissionError::FilesOnly);
        }
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(storage)?
            .take(remaining as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        remaining = remaining
            .checked_sub(bytes.len())
            .ok_or(SubmissionError::TooLarge)?;
        files.push(SubmissionFile::from_bytes(name, &bytes));
    }
    Ok(files)
}

/// The pending browser request a phone's hand-in page can cancel when it closes.
pub struct DraftUpload {
    pub id: String,
    pub url: String,
    dispatched: watch::Receiver<Option<bool>>,
    expires: Instant,
}

impl DraftUpload {
    /// The phone waits for dispatch of this request while the draft page is open.
    pub async fn dispatched(&self) -> bool {
        let mut result = self.dispatched.clone();
        result
            .wait_for(Option::is_some)
            .await
            .is_ok_and(|outcome| *outcome == Some(true))
    }

    pub(crate) fn wait_for_expiry(
        &self,
    ) -> impl std::future::Future<Output = bool> + Send + 'static {
        let expires = self.expires;
        let mut result = self.dispatched.clone();
        async move {
            tokio::time::timeout_at(expires, result.wait_for(Option::is_some))
                .await
                .is_err()
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
    pub request: String,
    pub url: String,
    files: Vec<SubmissionFile>,
    messages: SubmissionMessages,
    scoped: bool,
    #[serde(skip)]
    expires: Instant,
    #[serde(skip)]
    dispatch_result: Option<watch::Sender<Option<bool>>>,
}

impl PendingSubmission {
    pub fn new(
        raw: &str,
        files: Vec<SubmissionFile>,
        messages: SubmissionMessages,
    ) -> Result<Self, SubmissionError> {
        let url = submission_url(raw)?.to_string();
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
            scoped: false,
            expires: Instant::now() + Duration::from_secs(300),
            dispatch_result: None,
        })
    }

    /// A distinct URL lets both native hosts correlate a load even when their
    /// WebView callbacks carry no navigation action ID.
    pub fn scope_to_request(&mut self) -> Result<DraftUpload, SubmissionError> {
        let mut url = submission_url(&self.url)?;
        url.query_pairs_mut()
            .append_pair(REQUEST_PARAM, &self.request);
        self.url = url.to_string();
        self.scoped = true;
        let (sender, dispatched) = watch::channel(None);
        self.dispatch_result = Some(sender);
        Ok(DraftUpload {
            id: self.request.clone(),
            url: self.url.clone(),
            dispatched,
            expires: self.expires,
        })
    }

    pub fn report_dispatch(&self, sent: bool) {
        if let Some(result) = &self.dispatch_result {
            result.send_replace(Some(sent));
        }
    }

    pub fn scoped(&self) -> bool {
        self.scoped
    }

    pub fn expired(&self) -> bool {
        Instant::now() >= self.expires
    }

    pub fn matches(&self, raw: &str) -> bool {
        let Ok(url) = Url::parse(raw) else {
            return false;
        };
        if url.query_pairs().filter(|(key, _)| key == "id").count() != 1
            || url.query_pairs().filter(|(key, _)| key == "action").count() != 1
            || !url
                .query_pairs()
                .any(|(key, value)| key == "action" && value == "editsubmission")
        {
            return false;
        }
        let Ok(mut canonical) = submission_url(raw) else {
            return false;
        };
        if self.scoped {
            let mut requests = url.query_pairs().filter(|(key, _)| key == REQUEST_PARAM);
            if !requests
                .next()
                .is_some_and(|(_, value)| value == self.request)
                || requests.next().is_some()
            {
                return false;
            }
            canonical
                .query_pairs_mut()
                .append_pair(REQUEST_PARAM, &self.request);
        }
        canonical.as_str() == self.url
    }

    pub fn script(&self) -> Result<String, SubmissionError> {
        let request = serde_json::to_string(self)
            .map_err(|error| SubmissionError::Storage(error.to_string()))?;
        Ok(format!(
            "{}({request});",
            adpt_isct::scripts::ASSIGNMENT_UPLOAD
        ))
    }
}

pub(crate) fn cancel_script(id: &str) -> String {
    let request = serde_json::json!({ "cancelRequest": id });
    format!("{}({request});", adpt_isct::scripts::ASSIGNMENT_UPLOAD)
}

fn submission_url(raw: &str) -> Result<Url, SubmissionError> {
    let invalid = || {
        SubmissionError::InvalidInput(
            "This assignment has no supported LMS submission page.".into(),
        )
    };
    let mut url = Url::parse(raw).map_err(|_| invalid())?;
    if !adpt_isct::lms::is_lms_origin(&url) || !url.path().ends_with("/mod/assign/view.php") {
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
    Ok(url)
}
