//! Domain changes; native adapters deliver these through their own event bridge.
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    lifecycle::SchoolSite,
    school_browser::{SchoolLoginEvent, SchoolPullProgress, SchoolSessionEvent},
    school_service::IngestOutcome,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceChangeSource {
    ReferencePull,
    ReferencePoll,
    SchoolPull,
    Scheduler,
    Mcp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreEvent {
    WorkspaceChanged { source: WorkspaceChangeSource },
    SchoolLogin(SchoolLoginEvent),
    SchoolPullProgress(SchoolPullProgress),
    SchoolSession(SchoolSessionEvent),
    SchoolWebUrl { site: SchoolSite, url: String },
    SchoolWebOpen { site: SchoolSite, url: String },
    SchoolIngest(IngestOutcome),
}

impl CoreEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorkspaceChanged { .. } => "workspace-changed",
            Self::SchoolLogin(_) => "school-login",
            Self::SchoolPullProgress(_) => "school-pull-progress",
            Self::SchoolSession(_) => "school-session",
            Self::SchoolWebUrl { .. } => "school-web-url",
            Self::SchoolWebOpen { .. } => "school-web-open",
            Self::SchoolIngest(_) => "school-ingest",
        }
    }

    /// The existing desktop event payload is also the portable event contract.
    pub fn payload(&self) -> Value {
        match self {
            Self::WorkspaceChanged { source } => json!({ "source": source }),
            Self::SchoolLogin(event) => json!(event),
            Self::SchoolPullProgress(event) => json!(event),
            Self::SchoolSession(event) => json!(event),
            Self::SchoolWebUrl { site, url } | Self::SchoolWebOpen { site, url } => {
                json!({ "site": site, "url": url })
            }
            Self::SchoolIngest(outcome) => json!(outcome),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_events_preserve_desktop_names_and_payloads() {
        for (source, expected) in [
            (WorkspaceChangeSource::ReferencePull, "reference-pull"),
            (WorkspaceChangeSource::ReferencePoll, "reference-poll"),
            (WorkspaceChangeSource::SchoolPull, "school-pull"),
            (WorkspaceChangeSource::Scheduler, "scheduler"),
            (WorkspaceChangeSource::Mcp, "mcp"),
        ] {
            let event = CoreEvent::WorkspaceChanged { source };
            assert_eq!(event.name(), "workspace-changed");
            assert_eq!(event.payload(), json!({ "source": expected }));
        }
    }
}
