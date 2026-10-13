//! Domain changes; native adapters deliver these through their own event bridge.
use serde::Serialize;
use serde_json::{json, Value};

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
}

impl CoreEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorkspaceChanged { .. } => "workspace-changed",
        }
    }

    /// The existing desktop event payload is also the portable event contract.
    pub fn payload(&self) -> Value {
        match self {
            Self::WorkspaceChanged { source } => json!({ "source": source }),
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
