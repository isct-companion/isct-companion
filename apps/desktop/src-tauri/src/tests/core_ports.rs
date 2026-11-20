use super::*;
use app_core::{
    cloud_sync::SyncState,
    data::WorkspaceChangeSource,
    events::{SchoolDownloadEvent, SchoolDownloadState},
    lifecycle::SchoolSite,
    plan::{
        Held, Label, Priority, RetryPlan, RunView, Scope, SourceView, SyncAttention,
        SyncAttentionProblem, SyncFailure, SyncSnapshot, SyncWork,
    },
    school_browser::{
        LoadFailure, SchoolWebAccess, SchoolWebFailure, SchoolWebNotice, SchoolWebPage,
        SchoolWebProblem, SchoolWebStep,
    },
    school_auth::SchoolAuthProfile,
    school_service::{IngestOutcome, IngestState},
    school_session::{
        SchoolMaintenance, SchoolProfileSession, SchoolSessionProfiles, SchoolSessionSnapshot,
        SchoolSessionState, SchoolSignInProblem, SchoolSignInProblemKind, SignInPresentation,
        SignInRecovery,
    },
    workspace_changes::WorkspaceChange,
};
use serde_json::json;

/// Exhaustive on purpose: a new variant must be added to the wire test.
fn sample(event: &CoreEvent) -> &'static str {
    match event {
        CoreEvent::WorkspaceChanged { .. } => "workspace",
        CoreEvent::CatalogChanged { .. } => "catalog",
        CoreEvent::OcrProgress { .. } => "ocr",
        CoreEvent::SchoolSession(_) => "session",
        CoreEvent::SchoolWebPage(_) => "page",
        CoreEvent::SchoolWebNotice { .. } => "notice",
        CoreEvent::SchoolIngest(_) => "ingest",
        CoreEvent::SchoolDownload(_) => "download",
        CoreEvent::SyncSnapshot(_) => "sync",
        CoreEvent::CloudSyncState(_) => "cloud",
        CoreEvent::WakeDirective(_) => "directive",
        CoreEvent::ExamAccount(_) => "exam",
    }
}

/// Names and payloads React listens for (desktop-bridge.ts).
#[test]
fn every_core_event_reaches_the_ui_under_its_listened_name_and_payload() {
    let url = "https://lms.s.isct.ac.jp/course/view.php?id=1".to_string();
    let retry_at = chrono::DateTime::parse_from_rfc3339("2026-11-20T04:30:00Z")
        .unwrap()
        .to_utc();
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
            CoreEvent::SchoolSession(SchoolSessionSnapshot {
                revision: 7,
                profiles: SchoolSessionProfiles {
                    science: SchoolProfileSession {
                        sign_out_pending: false,
                        refusal_unsaved: false,
                        state: SchoolSessionState::Normal,
                        presentation: SignInPresentation {
                            state_key: "schoolSession.state.normal".into(),
                            reason_key: None,
                            recovery: vec![],
                        },
                        problem: None,
                        retry_at: None,
                        maintenance: Some(SchoolMaintenance {
                            until: retry_at,
                            notice: Some("Scheduled maintenance".into()),
                        }),
                    },
                    tokyo_tech: SchoolProfileSession {
                        sign_out_pending: false,
                        refusal_unsaved: false,
                        state: SchoolSessionState::Refused,
                        presentation: SignInPresentation {
                            state_key: "schoolSession.state.refused".into(),
                            reason_key: Some("failure.theMatrixAnswerWasNot".into()),
                            recovery: vec![SignInRecovery::Retry],
                        },
                        problem: Some(SchoolSignInProblem {
                            kind: SchoolSignInProblemKind::InvalidMatrix,
                            message: "matrix".into(),
                        }),
                        retry_at: None,
                        maintenance: None,
                    },
                },
            }),
            "school-session",
            json!({"revision":7,"profiles":{
                "science":{"signOutPending":false,"refusalUnsaved":false,"state":"normal","presentation":{"stateKey":"schoolSession.state.normal","reasonKey":null,"recovery":[]},"problem":null,"retryAt":null,"maintenance":{"notice":"Scheduled maintenance","until":"2026-11-20T04:30:00Z"}},
                "tokyoTech":{"signOutPending":false,"refusalUnsaved":false,"state":"refused","presentation":{"stateKey":"schoolSession.state.refused","reasonKey":"failure.theMatrixAnswerWasNot","recovery":["retry"]},"problem":{"kind":"invalid-matrix","message":"matrix"},"retryAt":null,"maintenance":null}
            }}),
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
                access: Some(SchoolWebAccess::Problem {
                    problem: SchoolWebProblem::StillSignIn,
                }),
                sign_in: true,
                otp: None,
                outside: false,
            }),
            "school-web-page",
            json!({"site":"science-lms","revision":9,"url":url,"popups":2,"back":"yes","forward":"unknown","notice":{"url":url,"reason":"host-not-found"},"access":{"kind":"problem","problem":"still-sign-in"},"signIn":true,"otp":null,"outside":false}),
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
                state: IngestState::Unavailable,
                changed: 0,
                message: "Not offered".into(),
            }),
            "school-ingest",
            json!({"site":"kyomu","kind":"grades","state":"unavailable","changed":0,"message":"Not offered"}),
        ),
        (
            CoreEvent::SchoolDownload(SchoolDownloadEvent {
                site: SchoolSite::ScienceLms,
                state: SchoolDownloadState::Done,
                name: "notes.pdf".into(),
                path: Some("/Downloads/notes.pdf".into()),
            }),
            "school-download",
            json!({"site":"science-lms","state":"done","name":"notes.pdf","path":"/Downloads/notes.pdf"}),
        ),
        (
            CoreEvent::SyncSnapshot(SyncSnapshot {
                generation: 2,
                resetting: false,
                attention: SyncAttention {
                    primary: Some(SyncWork::Active { run: "run-7".into() }),
                    problem: Some(SyncAttentionProblem::Account {
                        profile: SchoolAuthProfile::TokyoTech,
                        presentation: None,
                        retry_at: Some(retry_at),
                    }),
                    busy: true,
                },
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
                    retry: None,
                }],
                sources: vec![SourceView {
                    source: "lms".into(),
                    last_updated: None,
                    running: true,
                    held: None,
                    failure: None,
                    session: Some(SchoolSessionState::Normal),
                    maintenance: None,
                    retry: None,
                }],
            }),
            "sync-snapshot",
            json!({"generation":2,"resetting":false,"attention":{"primary":{"kind":"active","run":"run-7"},"problem":{"kind":"account","profile":"tokyo-tech","presentation":null,"retryAt":"2026-11-20T04:30:00Z"},"busy":true},"runs":[{"run":"run-7","dataset":"lms.course","label":{"key":"sync.dataset.lms.course","args":{}},"priority":"visible","done":3,"total":9,"current":null,"scope":"42","held":{"kind":"cellular"},"failure":null,"retry":null}],"sources":[{"source":"lms","lastUpdated":null,"running":true,"held":null,"failure":null,"session":"normal","maintenance":null,"retry":null}]}),
        ),
        (
            CoreEvent::SyncSnapshot(SyncSnapshot {
                generation: 2,
                attention: SyncAttention {
                    primary: Some(SyncWork::Problem),
                    problem: Some(SyncAttentionProblem::RunFailed {
                        dataset: "lms.course".into(),
                        failure: SyncFailure {
                            kind: "http-503".into(),
                            reason: Label {
                                key: "syncStatus.failure.serverFailed".into(),
                                args: json!({"status":"503"}),
                            },
                            keys_failed: 1,
                            retry_at: Some(retry_at),
                            detail: None,
                        },
                        retry: Some(RetryPlan {
                            scopes: vec![Scope::Course { lms_course_id: 42 }],
                        }),
                    }),
                    busy: false,
                },
                ..SyncSnapshot::default()
            }),
            "sync-snapshot",
            json!({
                "generation":2,"resetting":false,"runs":[],"sources":[],
                "attention":{
                    "primary":{"kind":"problem"},"busy":false,
                    "problem":{
                        "kind":"run-failed","dataset":"lms.course",
                        "failure":{"kind":"http-503","reason":{"key":"syncStatus.failure.serverFailed","args":{"status":"503"}},"keysFailed":1,"retryAt":"2026-11-20T04:30:00Z","detail":null},
                        "retry":{"scopes":[{"kind":"course","lmsCourseId":42}]}
                    }
                }
            }),
        ),
        (
            CoreEvent::CloudSyncState(SyncState::default()),
            "cloud-sync-state",
            json!({"syncing":false,"automatic":false,"localVersion":"","providers":[],"versions":[],"tasks":[]}),
        ),
        (
            CoreEvent::WakeDirective(app_core::plan::WakeDirective::default()),
            "wake-directive",
            json!({"revision":0,"at":null,"background":[]}),
        ),
        (
            CoreEvent::ExamAccount(app_core::exam_share::ExamAccount {
                status: app_core::exam_share::ExamAccountStatus::Connected,
                email: Some("student@m.isct.ac.jp".into()),
                upload: app_core::exam_share::ExamUploadStatus {
                    phase: app_core::exam_share::ExamUploadPhase::Transferring,
                    sent: 4,
                    total: 9,
                    cancellable: true,
                },
            }),
            "exam-account",
            json!({"status":"connected","email":"student@m.isct.ac.jp","upload":{"phase":"transferring","sent":4,"total":9,"cancellable":true}}),
        ),
    ];
    let mut covered = std::collections::HashSet::new();
    for (event, name, payload) in cases {
        covered.insert(sample(&event));
        assert_eq!(wire(&event), (name, payload));
    }
    assert_eq!(covered.len(), 12);
}
