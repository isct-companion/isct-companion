use super::*;

#[test]
fn workspace_events_preserve_desktop_names_and_payloads() {
    for (source, expected) in [
        (WorkspaceChangeSource::UserIntent, "user-intent"),
        (WorkspaceChangeSource::ReferencePull, "reference-pull"),
        (WorkspaceChangeSource::ReferencePoll, "reference-poll"),
        (WorkspaceChangeSource::SchoolPull, "school-pull"),
        (WorkspaceChangeSource::Scheduler, "scheduler"),
        (WorkspaceChangeSource::Mcp, "mcp"),
        (WorkspaceChangeSource::CloudSync, "cloud-sync"),
        (WorkspaceChangeSource::CloudRestore, "cloud-restore"),
        (WorkspaceChangeSource::LocalRestore, "local-restore"),
    ] {
        let event = CoreEvent::WorkspaceChanged {
            source,
            revision: None,
        };
        assert_eq!(event.name(), "workspace-changed");
        assert_eq!(event.payload(), json!({ "source": expected }));
    }

    let event = CoreEvent::WorkspaceChanged {
        source: WorkspaceChangeSource::UserIntent,
        revision: Some("r1".into()),
    };
    assert_eq!(
        event.payload(),
        json!({"source":"user-intent","revision":"r1"})
    );
}
