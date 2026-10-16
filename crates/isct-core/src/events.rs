//! Domain changes; native adapters deliver these through their own event bridge.
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    cloud_sync::SyncState,
    lifecycle::SchoolSite,
    pull_coordinator::PullSnapshot,
    school_browser::{SchoolLoginEvent, SchoolPullProgress, SchoolSessionEvent},
    school_service::IngestOutcome,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceChangeSource {
    UserIntent,
    ReferencePull,
    ReferencePoll,
    SchoolPull,
    Scheduler,
    Mcp,
    CloudSync,
    CloudRestore,
    LocalRestore,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SchoolDownloadState {
    Started,
    Done,
    Failed,
}

/// A file a school page handed over: a download, or a save from the PDF viewer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolDownloadEvent {
    pub site: SchoolSite,
    pub state: SchoolDownloadState,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Every event a shell forwards to its UI. Events are hints: the UI re-reads
/// the matching snapshot (`workspace_snapshot`, `pull_state`, `cloud_sync_state`)
/// on load, so a delivery that happens before a window exists is not lost state.
#[derive(Clone, Debug)]
pub enum CoreEvent {
    WorkspaceChanged {
        source: WorkspaceChangeSource,
        /// Present for user intents so the initiating shell can correlate the
        /// event with the authoritative snapshot returned by the mutation.
        revision: Option<String>,
    },
    OcrProgress {
        operation_id: u64,
        completed: u64,
        total: u64,
    },
    SchoolLogin(SchoolLoginEvent),
    SchoolPullProgress(SchoolPullProgress),
    SchoolSession(SchoolSessionEvent),
    SchoolWebUrl {
        site: SchoolSite,
        url: String,
    },
    SchoolWebOpen {
        site: SchoolSite,
        url: String,
    },
    SchoolIngest(IngestOutcome),
    SchoolDownload(SchoolDownloadEvent),
    PullStateChanged(Vec<PullSnapshot>),
    CloudSyncState(SyncState),
}

impl CoreEvent {
    /// The event name the desktop UI listens for.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorkspaceChanged { .. } => "workspace-changed",
            Self::OcrProgress { .. } => "ocr-progress",
            Self::SchoolLogin(_) => "school-login-status",
            Self::SchoolPullProgress(_) => "school-pull-progress",
            Self::SchoolSession(_) => "school-session",
            Self::SchoolWebUrl { .. } => "school-web-url",
            Self::SchoolWebOpen { .. } => "school-web-open",
            Self::SchoolIngest(_) => "school-ingest",
            Self::SchoolDownload(_) => "school-download",
            Self::PullStateChanged(_) => "pull-state-changed",
            Self::CloudSyncState(_) => "cloud-sync-state",
        }
    }

    /// The existing desktop event payload is also the portable event contract.
    pub fn payload(&self) -> Value {
        match self {
            Self::WorkspaceChanged { source, revision } => {
                let mut payload = json!({ "source": source });
                if let Some(revision) = revision {
                    payload["revision"] = json!(revision);
                }
                payload
            }
            Self::OcrProgress {
                operation_id,
                completed,
                total,
            } => json!({
                "operationId": operation_id,
                "completed": completed,
                "total": total,
            }),
            Self::SchoolLogin(event) => json!(event),
            Self::SchoolPullProgress(event) => json!(event),
            Self::SchoolSession(event) => json!(event),
            Self::SchoolWebUrl { site, url } | Self::SchoolWebOpen { site, url } => {
                json!({ "site": site, "url": url })
            }
            Self::SchoolIngest(outcome) => json!(outcome),
            Self::SchoolDownload(event) => json!(event),
            Self::PullStateChanged(snapshots) => json!(snapshots),
            Self::CloudSyncState(state) => json!(state),
        }
    }
}

#[cfg(test)]
#[path = "tests/events.rs"]
mod tests;
