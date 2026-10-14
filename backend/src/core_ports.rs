//! Desktop adapters. Domain services and their task lifetime belong to Core.
use crate::{data::DataStore, school_web::SchoolWebController};
use isct_core::{
    events::CoreEvent,
    ports::{EventError, EventSink},
    runtime::CoreRuntime,
    runtime_tasks::RuntimeTasks,
    scheduler::{ExportError, ScheduledExporter, SchedulerServices},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Manager};

/// The one desktop event path: every Core event reaches the main webview
/// under its UI name. Delivery failures are logged here, so Core callers may
/// treat events as hints without hiding the failure.
pub(crate) struct DesktopEvents(pub(crate) AppHandle);
impl EventSink for DesktopEvents {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        let (name, payload) = wire(&event);
        self.0.emit_to(MAIN_WINDOW, name, payload).map_err(|error| {
            eprintln!("[events] {name} was not delivered: {error}");
            EventError::Delivery(error.to_string())
        })
    }
}

const MAIN_WINDOW: &str = "main";

/// Name and JSON payload exactly as the webview receives them.
fn wire(event: &CoreEvent) -> (&'static str, serde_json::Value) {
    (event.name(), event.payload())
}

struct DesktopExport(AppHandle);
impl ScheduledExporter for DesktopExport {
    fn export(&self) -> Result<(), ExportError> {
        crate::commands::backup::write_scheduled_export_inner(&self.0)
            .map(|_| ())
            .map_err(ExportError::Failed)
    }
}

pub(crate) fn core(app: &AppHandle) -> Arc<isct_core::Core> {
    app.state::<Arc<isct_core::Core>>().inner().clone()
}
pub(crate) fn runtime(app: &AppHandle) -> Arc<CoreRuntime> {
    core(app).runtime.clone()
}

// Operation-scoped ports avoid retaining AppHandle in the core's permanent
// service state. Completed coordinator jobs release their work closures too.
pub(crate) fn services(app: &AppHandle) -> SchedulerServices {
    core(app).services(
        Arc::new(DesktopEvents(app.clone())),
        app.state::<SchoolWebController>().engine.clone(),
        Arc::new(DesktopExport(app.clone())),
    )
}

pub(crate) fn publish_conditions(app: &AppHandle) -> Result<(), String> {
    let runtime = runtime(app);
    tauri::async_runtime::block_on(runtime.refresh.publish_conditions(
        &app.state::<DataStore>(),
        &runtime.coordinator,
        app.state::<SchoolWebController>().is_visible(),
    ))
    .map_err(|error| error.to_string())
}

#[derive(Default)]
pub(crate) struct DesktopRuntime {
    tasks: RuntimeTasks,
    exiting: AtomicBool,
    stopped: AtomicBool,
}

impl DesktopRuntime {
    pub(crate) fn start(app: &AppHandle) {
        let focused = app
            .get_webview_window("main")
            .and_then(|window| window.is_focused().ok())
            == Some(true);
        let _ = runtime(app)
            .refresh
            .event(isct_core::school_refresh::RefreshEvent {
                kind: if focused { "foreground" } else { "background" }.into(),
                profile: None,
                course_id: None,
                network: None,
            });
        let tasks = app.state::<Self>().tasks.clone();
        let app = app.clone();
        // Setup is called on the UI thread; enter Tauri's shared Tokio runtime
        // before registering either owned loop.
        tauri::async_runtime::block_on(async move {
            let wake_app = app.clone();
            let wake_tasks = tasks.clone();
            tasks.spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                loop {
                    tokio::select! {
                        _ = wake_tasks.closed() => break,
                        _ = interval.tick() => {}
                    }
                    crate::cloud_sync::CloudSync::tick(&wake_app);
                    let visible = wake_app.state::<SchoolWebController>().is_visible();
                    let _ = runtime(&wake_app).wake(services(&wake_app), visible).await;
                    let _ = core(&wake_app)
                        .reminder_tick(
                            Arc::new(crate::commands::notifications::DesktopNotifications(
                                wake_app.clone(),
                            )),
                            chrono::Utc::now().fixed_offset(),
                        )
                        .await;
                }
            });
            // Started after the main window exists. The UI also reads
            // `pull_state` after subscribing, so no snapshot depends on an emit.
            let mut states = runtime(&app).coordinator.subscribe();
            let event_tasks = tasks.clone();
            let events = DesktopEvents(app.clone());
            tasks.spawn(async move {
                loop {
                    tokio::select! {
                        _ = event_tasks.closed() => break,
                        changed = states.changed() => { if changed.is_err() { break; } }
                    }
                    let snapshot = states.borrow_and_update().clone();
                    let _ = events.emit(CoreEvent::PullStateChanged(snapshot));
                }
            });
        });
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn begin_exit(&self) -> bool {
        !self.exiting.swap(true, Ordering::AcqRel)
    }

    pub(crate) async fn shutdown(&self, core: &isct_core::Core) {
        self.tasks.close();
        core.shutdown().await;
        self.tasks.wait().await;
        self.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use isct_core::{
        cloud_sync::SyncState,
        events::{SchoolDownloadEvent, SchoolDownloadState, WorkspaceChangeSource},
        lifecycle::SchoolSite,
        pull_coordinator::{PullPriority, PullSnapshot, PullState, PullTarget},
        school_auth::SchoolAuthProfile,
        school_browser::{
            SchoolLoginEvent, SchoolPullProgress, SchoolSessionEvent, SchoolSessionState,
        },
        school_service::IngestOutcome,
    };
    use serde_json::json;

    /// Exhaustive on purpose: a new variant must be added to the wire test.
    fn sample(event: &CoreEvent) -> &'static str {
        match event {
            CoreEvent::WorkspaceChanged { .. } => "workspace",
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
                    source: WorkspaceChangeSource::CloudRestore,
                },
                "workspace-changed",
                json!({"source":"cloud-restore"}),
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
                    state: "reconnect-required".into(),
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
        assert_eq!(covered.len(), 10);
    }
}
