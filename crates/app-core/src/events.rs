//! Domain changes; native adapters deliver these through their own event bridge.
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    cloud_sync::SyncState,
    data::WorkspaceChangeSource,
    lifecycle::SchoolSite,
    school_browser::{SchoolLoginEvent, SchoolSessionEvent, SchoolWebNotice},
    school_service::IngestOutcome,
    sync_engine::SyncSnapshot,
    workspace_changes::WorkspaceChange,
};

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

/// Every event a shell forwards to its UI. Events are hints: the UI queries
/// what it shows (`Core::schedule_query`, `sync_snapshot`, `cloud_sync_state`)
/// on load, so a delivery that happens before a window exists is not lost state.
#[derive(Clone, Debug)]
pub enum CoreEvent {
    WorkspaceChanged {
        source: WorkspaceChangeSource,
        /// Present for user intents so the initiating shell can correlate the
        /// event with the commit its mutation answered.
        revision: Option<String>,
        /// What the commits behind this event changed, sorted. Background
        /// changes arrive merged, at most once per source and
        /// `workspace_changes::COALESCE_WINDOW`; a query reads again when
        /// one of its reads is named.
        changes: Vec<WorkspaceChange>,
        /// `DataStore::commit_seq` once the commits behind this event had
        /// landed; a query result with an older `seq` is stale.
        seq: u64,
    },
    /// One year's stored catalog changed: its offerings or the official
    /// search form's choices. A finder showing `year` queries again.
    /// Arrives at most once per `workspace_changes::COALESCE_WINDOW`
    /// while a download lands department by department; `version` is the
    /// catalog version (a finder page's `catalogVersion`) once it landed.
    CatalogChanged {
        year: u16,
        version: u64,
    },
    OcrProgress {
        operation_id: u64,
        completed: u64,
        total: u64,
    },
    SchoolLogin(SchoolLoginEvent),
    SchoolSession(SchoolSessionEvent),
    SchoolWebUrl {
        site: SchoolSite,
        url: String,
    },
    /// How many popups are open over the site's view in the web pane (hosts
    /// with inline popups only). The toolbar acts on the frontmost.
    SchoolWebPopups {
        site: SchoolSite,
        open: usize,
    },
    SchoolWebNotice {
        site: SchoolSite,
        notice: SchoolWebNotice,
    },
    SchoolIngest(IngestOutcome),
    SchoolDownload(SchoolDownloadEvent),
    /// Sync progress and source status; read `Core::sync_snapshot` for the
    /// current value.
    SyncSnapshot(SyncSnapshot),
    CloudSyncState(SyncState),
}

impl CoreEvent {
    /// The event name the desktop UI listens for.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorkspaceChanged { .. } => "workspace-changed",
            Self::CatalogChanged { .. } => "catalog-changed",
            Self::OcrProgress { .. } => "ocr-progress",
            Self::SchoolLogin(_) => "school-login-status",
            Self::SchoolSession(_) => "school-session",
            Self::SchoolWebUrl { .. } => "school-web-url",
            Self::SchoolWebPopups { .. } => "school-web-popups",
            Self::SchoolWebNotice { .. } => "school-web-notice",
            Self::SchoolIngest(_) => "school-ingest",
            Self::SchoolDownload(_) => "school-download",
            Self::SyncSnapshot(_) => "sync-snapshot",
            Self::CloudSyncState(_) => "cloud-sync-state",
        }
    }

    /// The existing desktop event payload is also the portable event contract.
    pub fn payload(&self) -> Value {
        match self {
            Self::WorkspaceChanged {
                source,
                revision,
                changes,
                seq,
            } => {
                let mut payload = json!({ "source": source, "changes": changes, "seq": seq });
                if let Some(revision) = revision {
                    payload["revision"] = json!(revision);
                }
                payload
            }
            Self::CatalogChanged { year, version } => {
                json!({ "year": year, "version": version })
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
            Self::SchoolSession(event) => json!(event),
            Self::SchoolWebUrl { site, url } => json!({ "site": site, "url": url }),
            Self::SchoolWebPopups { site, open } => json!({ "site": site, "open": open }),
            Self::SchoolWebNotice { site, notice } => {
                let mut payload = json!(notice);
                payload["site"] = json!(site);
                payload
            }
            Self::SchoolIngest(outcome) => json!(outcome),
            Self::SchoolDownload(event) => json!(event),
            Self::SyncSnapshot(snapshot) => json!(snapshot),
            Self::CloudSyncState(state) => json!(state),
        }
    }
}

#[cfg(test)]
#[path = "tests/events.rs"]
mod tests;
