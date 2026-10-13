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

pub(crate) struct DesktopEvents(pub(crate) AppHandle);
impl EventSink for DesktopEvents {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        self.0
            .emit(event.name(), event.payload())
            .map_err(|error| EventError::Delivery(error.to_string()))
    }
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
                }
            });
            let mut states = runtime(&app).coordinator.subscribe();
            let event_tasks = tasks.clone();
            tasks.spawn(async move {
                loop {
                    tokio::select! {
                        _ = event_tasks.closed() => break,
                        changed = states.changed() => { if changed.is_err() { break; } }
                    }
                    let snapshot = states.borrow_and_update().clone();
                    let _ = app.emit_to("main", "pull-state-changed", snapshot);
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
