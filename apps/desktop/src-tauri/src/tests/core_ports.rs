use super::*;
use isct_core::{
    cloud_sync::SyncState,
    data::WorkspaceChangeSource,
    events::{SchoolDownloadEvent, SchoolDownloadState},
    lifecycle::SchoolSite,
    pull_coordinator::{PullPriority, PullSnapshot, PullState, PullTarget},
    school_auth::SchoolAuthProfile,
    school_browser::{
        SchoolLoginEvent, SchoolPullProgress, SchoolSessionEvent, SchoolSessionState,
    },
    school_service::{IngestOutcome, IngestState},
};
use serde_json::json;

/// Exhaustive on purpose: a new variant must be added to the wire test.
fn sample(event: &CoreEvent) -> &'static str {
    match event {
        CoreEvent::WorkspaceChanged { .. } => "workspace",
        CoreEvent::OcrProgress { .. } => "ocr",
        CoreEvent::SchoolLogin(_) => "login",
        CoreEvent::SchoolPullProgress(_) => "progress",
        CoreEvent::SchoolSession(_) => "session",
        CoreEvent::SchoolWebUrl { .. } => "url",
        CoreEvent::SchoolWebOpen { .. } => "open",
        CoreEvent::SchoolIngest(_) => "ingest",
        CoreEvent::SchoolDownload(_) => "download",
        CoreEvent::PullStateChanged(_) => "pull",
        CoreEvent::CloudSyncState(_) => "cloud",
    }
}

/// Names and payloads React listens for (desktop-bridge.ts). The shapes
/// match the events main emitted before Core owned them; school-session
/// gained `revision` and pull-state-changed is new with Core.
#[test]
fn every_core_event_reaches_the_ui_under_its_listened_name_and_payload() {
    let url = "https://lms.s.isct.ac.jp/course/view.php?id=1".to_string();
    let cases = [
        (
            CoreEvent::WorkspaceChanged {
                source: WorkspaceChangeSource::LocalRestore,
                revision: None,
            },
            "workspace-changed",
            json!({"source":"local-restore"}),
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
            CoreEvent::SchoolPullProgress(SchoolPullProgress {
                site: SchoolSite::ScienceLms,
                phase: "course".into(),
                completed: 1,
                total: 3,
                course_id: 42,
                course_title: "Physics".into(),
                term: "2026-3Q".into(),
                item_title: "Week 1".into(),
            }),
            "school-pull-progress",
            json!({"site":"science-lms","phase":"course","completed":1,"total":3,"courseId":42,"courseTitle":"Physics","term":"2026-3Q","itemTitle":"Week 1"}),
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
            CoreEvent::SchoolWebUrl {
                site: SchoolSite::ScienceLms,
                url: url.clone(),
            },
            "school-web-url",
            json!({"site":"science-lms","url":url}),
        ),
        (
            CoreEvent::SchoolWebOpen {
                site: SchoolSite::Kyomu,
                url: url.clone(),
            },
            "school-web-open",
            json!({"site":"kyomu","url":url}),
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
            CoreEvent::PullStateChanged(vec![PullSnapshot {
                id: "job".into(),
                target: PullTarget::School {
                    site: SchoolSite::ScienceLms,
                    course_id: Some(42),
                },
                priority: PullPriority::Manual,
                state: PullState::RetryWaiting,
                phase: None,
                blocked_by: Some("background".into()),
                attempt: 2,
                waiter_count: 1,
                last_error: None,
            }]),
            "pull-state-changed",
            json!([{"id":"job","target":{"type":"school","site":"science-lms","courseId":42},"priority":"manual","state":"retryWaiting","phase":null,"blockedBy":"background","attempt":2,"waiterCount":1,"lastError":null}]),
        ),
        (
            CoreEvent::CloudSyncState(SyncState::default()),
            "cloud-sync-state",
            json!({"syncing":false,"automatic":false,"localVersion":"","providers":[],"versions":[],"tasks":[],"recoveryPoints":[]}),
        ),
    ];
    let mut covered = std::collections::HashSet::new();
    for (event, name, payload) in cases {
        covered.insert(sample(&event));
        assert_eq!(wire(&event), (name, payload));
    }
    assert_eq!(covered.len(), 11);
}
