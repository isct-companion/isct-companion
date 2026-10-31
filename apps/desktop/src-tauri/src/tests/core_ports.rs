use super::*;
use app_core::{
    cloud_sync::SyncState,
    data::WorkspaceChangeSource,
    events::{SchoolDownloadEvent, SchoolDownloadState},
    lifecycle::SchoolSite,
    school_auth::SchoolAuthProfile,
    school_browser::{
        LoadFailure, SchoolLoginEvent, SchoolSessionEvent, SchoolSessionState, SchoolWebFailure,
        SchoolWebNotice, SchoolWebPage, SchoolWebStep,
    },
    school_service::{IngestOutcome, IngestState},
    sync_engine::{Held, Label, Priority, RunView, SourceView, SyncSnapshot},
    workspace_changes::WorkspaceChange,
};
use serde_json::json;

/// Exhaustive on purpose: a new variant must be added to the wire test.
fn sample(event: &CoreEvent) -> &'static str {
    match event {
        CoreEvent::WorkspaceChanged { .. } => "workspace",
        CoreEvent::CatalogChanged { .. } => "catalog",
        CoreEvent::OcrProgress { .. } => "ocr",
        CoreEvent::SchoolLogin(_) => "login",
        CoreEvent::SchoolSession(_) => "session",
        CoreEvent::SchoolWebPage(_) => "page",
        CoreEvent::SchoolWebNotice { .. } => "notice",
        CoreEvent::SchoolIngest(_) => "ingest",
        CoreEvent::SchoolDownload(_) => "download",
        CoreEvent::SyncSnapshot(_) => "sync",
        CoreEvent::CloudSyncState(_) => "cloud",
    }
}

/// Names and payloads React listens for (desktop-bridge.ts). The shapes
/// match the events main emitted before Core owned them; school-session
/// gained `revision` and sync-snapshot is new with Core's planner.
#[test]
fn every_core_event_reaches_the_ui_under_its_listened_name_and_payload() {
    let url = "https://lms.s.isct.ac.jp/course/view.php?id=1".to_string();
    let cases = [
        (
            CoreEvent::WorkspaceChanged {
                source: WorkspaceChangeSource::CloudRestore,
                revision: None,
                changes: vec![WorkspaceChange::Plans, WorkspaceChange::ScheduledTasks],
                seq: 3,
            },
            "workspace-changed",
            json!({"source":"cloud-restore","changes":["plans","scheduled-tasks"],"seq":3}),
        ),
        (
            CoreEvent::CatalogChanged {
                year: 2026,
                version: 12,
            },
            "catalog-changed",
            json!({"year":2026,"version":12}),
        ),
        (
            CoreEvent::OcrProgress {
                operation_id: 42,
                completed: 7,
                total: 70,
            },
            "ocr-progress",
            json!({"operationId":42,"completed":7,"total":70}),
        ),
        (
            CoreEvent::SchoolLogin(SchoolLoginEvent {
                profile: SchoolAuthProfile::TokyoTech,
                state: "connected".into(),
                message: "Connected".into(),
            }),
            "school-login-status",
            json!({"profile":"tokyo-tech","state":"connected","message":"Connected"}),
        ),
        (
            CoreEvent::SchoolSession(SchoolSessionEvent {
                profile: SchoolAuthProfile::Science,
                state: SchoolSessionState::NotConfigured,
                revision: 7,
            }),
            "school-session",
            json!({"profile":"science","state":"not-configured","revision":7}),
        ),
        (
            CoreEvent::SchoolWebPage(SchoolWebPage {
                site: SchoolSite::ScienceLms,
                revision: 9,
                url: Some(url.clone()),
                popups: 2,
                back: SchoolWebStep::Yes,
                forward: SchoolWebStep::Unknown,
                notice: Some(SchoolWebFailure {
                    url: Some(url.clone()),
                    reason: LoadFailure::HostNotFound,
                }),
            }),
            "school-web-page",
            json!({"site":"science-lms","revision":9,"url":url,"popups":2,"back":"yes","forward":"unknown","notice":{"url":url,"reason":"host-not-found"}}),
        ),
        (
            CoreEvent::SchoolWebNotice {
                site: SchoolSite::Kyomu,
                notice: SchoolWebNotice::Reopened { url: url.clone() },
            },
            "school-web-notice",
            json!({"site":"kyomu","kind":"reopened","url":url}),
        ),
        (
            CoreEvent::SchoolIngest(IngestOutcome {
                site: SchoolSite::Kyomu,
                kind: "grades".into(),
                state: IngestState::ReconnectRequired,
                changed: 0,
                message: "Sign in".into(),
            }),
            "school-ingest",
            json!({"site":"kyomu","kind":"grades","state":"reconnect-required","changed":0,"message":"Sign in"}),
        ),
        (
            CoreEvent::SchoolDownload(SchoolDownloadEvent {
                site: SchoolSite::ScienceLms,
                state: SchoolDownloadState::Done,
                name: "notes.pdf".into(),
                url: None,
                path: Some("/Downloads/notes.pdf".into()),
            }),
            "school-download",
            json!({"site":"science-lms","state":"done","name":"notes.pdf","path":"/Downloads/notes.pdf"}),
        ),
        (
            CoreEvent::SyncSnapshot(SyncSnapshot {
                generation: 2,
                resetting: false,
                runs: vec![RunView {
                    run: "run-7".into(),
                    dataset: "lms.course".into(),
                    label: Label {
                        key: "sync.dataset.lms.course".into(),
                        args: json!({}),
                    },
                    priority: Priority::Visible,
                    done: 3,
                    total: Some(9),
                    current: None,
                    scope: Some("42".into()),
                    held: Some(Held::Cellular),
                    failure: None,
                }],
                sources: vec![SourceView {
                    source: "lms".into(),
                    last_updated: None,
                    running: true,
                    held: None,
                    failure: None,
                    session: Some("active".into()),
                    maintenance: None,
                }],
            }),
            "sync-snapshot",
            json!({"generation":2,"resetting":false,"runs":[{"run":"run-7","dataset":"lms.course","label":{"key":"sync.dataset.lms.course","args":{}},"priority":"visible","done":3,"total":9,"current":null,"scope":"42","held":{"kind":"cellular"},"failure":null}],"sources":[{"source":"lms","lastUpdated":null,"running":true,"held":null,"failure":null,"session":"active","maintenance":null}]}),
        ),
        (
            CoreEvent::CloudSyncState(SyncState::default()),
            "cloud-sync-state",
            json!({"syncing":false,"automatic":false,"localVersion":"","providers":[],"versions":[],"tasks":[]}),
        ),
    ];
    let mut covered = std::collections::HashSet::new();
    for (event, name, payload) in cases {
        covered.insert(sample(&event));
        assert_eq!(wire(&event), (name, payload));
    }
    assert_eq!(covered.len(), 11);
}
