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

/// Republish pull conditions after a focus/visibility fact. Safe on the UI
/// thread: Core reads storage on a blocking worker inside the spawned task.
pub(crate) fn publish_conditions(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let visible = app.state::<SchoolWebController>().is_visible();
        let store = app.state::<DataStore>().inner().clone();
        if let Err(error) = runtime(&app).publish_conditions(&store, visible).await {
            eprintln!("[school-refresh] conditions were not published: {error}");
        }
    });
}

/// Bound on draining owned work at exit. Drains are normally bounded by
/// their own timeouts; this guarantees the app can always quit.
pub(crate) const EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Logs a repeating background failure once per distinct message.
#[derive(Default)]
struct FailureLog(Option<String>);

impl FailureLog {
    fn record<E: std::fmt::Display>(&mut self, scope: &str, result: Result<(), E>) {
        match result {
            Ok(()) => self.0 = None,
            Err(error) => {
                let message = error.to_string();
                if self.0.as_deref() != Some(message.as_str()) {
                    eprintln!("[{scope}] {message}");
                    self.0 = Some(message);
                }
            }
        }
    }
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
        if let Err(error) =
            runtime(app).refresh_event(isct_core::school_refresh::RefreshEvent::lifecycle(focused))
        {
            eprintln!("[school-refresh] {error}");
        }
        let tasks = app.state::<Self>().tasks.clone();
        let app = app.clone();
        // Setup is called on the UI thread; enter Tauri's shared Tokio runtime
        // before registering either owned loop.
        tauri::async_runtime::block_on(async move {
            if let Err(error) = core(&app).start(&tokio::runtime::Handle::current()) {
                eprintln!("[events] pull state forwarding did not start: {error}");
            }
            let wake_app = app.clone();
            let wake_tasks = tasks.clone();
            tasks.spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
                let (mut wake_failure, mut cloud_failure, mut reminder_failure) = (
                    FailureLog::default(),
                    FailureLog::default(),
                    FailureLog::default(),
                );
                loop {
                    tokio::select! {
                        _ = wake_tasks.closed() => break,
                        _ = interval.tick() => {}
                    }
                    #[cfg(desktop)]
                    crate::agent_server::AgentAccess::tick(&wake_app);
                    let visible = wake_app.state::<SchoolWebController>().is_visible();
                    let cloud = match crate::cloud_sync::services(&wake_app) {
                        Ok(services) => Some(services),
                        Err(error) => {
                            cloud_failure.record("cloud", Err(error));
                            None
                        }
                    };
                    let report = core(&wake_app)
                        .wake(isct_core::facade::WakeServices {
                            scheduler: services(&wake_app),
                            school_visible: visible,
                            cloud,
                            reminders: Some(isct_core::facade::ReminderWake {
                                sink: Arc::new(
                                    crate::commands::notifications::DesktopNotifications(
                                        wake_app.clone(),
                                    ),
                                ),
                                now: chrono::Utc::now().fixed_offset(),
                            }),
                        })
                        .await;
                    wake_failure.record("runtime", report.runtime);
                    if let Some(result) = report.cloud {
                        cloud_failure.record("cloud", result.map(|_| ()));
                    }
                    if let Some(result) = report.reminders {
                        reminder_failure.record("reminders", result.map(|_| ()));
                    }
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
    }

    /// Allows the next exit request through, drained or not.
    pub(crate) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "tests/core_ports.rs"]
mod tests;
