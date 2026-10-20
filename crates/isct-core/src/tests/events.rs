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
        (WorkspaceChangeSource::Reset, "reset"),
    ] {
        let event = CoreEvent::WorkspaceChanged {
            source,
            revision: None,
            changes: vec![WorkspaceChange::Courses, WorkspaceChange::ScheduledTasks],
        };
        assert_eq!(event.name(), "workspace-changed");
        assert_eq!(
            event.payload(),
            json!({ "source": expected, "changes": ["courses", "scheduled-tasks"] })
        );
    }

    let event = CoreEvent::WorkspaceChanged {
        source: WorkspaceChangeSource::UserIntent,
        revision: Some("r1".into()),
        changes: vec![WorkspaceChange::Settings],
    };
    assert_eq!(
        event.payload(),
        json!({"source":"user-intent","revision":"r1","changes":["settings"]})
    );
}
