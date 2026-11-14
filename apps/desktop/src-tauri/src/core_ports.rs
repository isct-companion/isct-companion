//! Desktop adapters. Domain services and their task lifetime belong to Core.
use app_core::{
    events::CoreEvent,
    plan::Hint,
    ports::{EventError, EventSink},
    runtime::CoreRuntime,
    runtime_tasks::RuntimeTasks,
    scheduler::{ExportError, ScheduledExporter},
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
        // The desktop's timer reads directives in Rust (`deadline_timer`);
        // the webview has no use for them.
        if matches!(event, CoreEvent::WakeDirective(_)) {
            return Ok(());
        }
        let (name, payload) = wire(&event);
        self.0.emit_to(MAIN_WINDOW, name, payload).map_err(|error| {
            log::error!("[events] {name} was not delivered: {error}");
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
            .map_err(|error| ExportError::Failed(error.to_string()))
    }
}

pub(crate) fn core(app: &AppHandle) -> Arc<app_core::Core> {
    app.state::<Arc<app_core::Core>>().inner().clone()
}
pub(crate) fn runtime(app: &AppHandle) -> Arc<CoreRuntime> {
    core(app).runtime.clone()
}

/// The main window's focus as a planner fact. The desktop window counts as
/// foreground while focused; the network is reported by the webview.
pub(crate) fn lifecycle_hint(focused: bool) -> Hint {
    if focused {
        Hint::Foreground
    } else {
        Hint::Background
    }
}

/// Bound on draining owned work at exit. Drains are normally bounded by
/// their own timeouts; this guarantees the app can always quit.
pub(crate) const EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// The longest the deadline timer sleeps before it reads the wall clock
/// again: the monotonic clock a sleep runs on stops while the Mac sleeps,
/// so a deadline that came during system sleep is at most this late.
const WALL_CLOCK_CHECK: std::time::Duration = std::time::Duration::from_secs(5 * 60);

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
        let core = core(app);
        core.sync_hint(lifecycle_hint(focused));
        // Core's duties keep these ports until it shuts down.
        core.attach_ports(app_core::facade::PlatformPorts {
            exporter: Some(Arc::new(DesktopExport(app.clone()))),
            notifications: Some(Arc::new(
                crate::commands::notifications::DesktopNotifications(app.clone()),
            )),
            cloud: match crate::cloud_sync::services(app) {
                Ok(services) => Some(services),
                Err(error) => {
                    log::error!("[cloud] automatic sync is unavailable: {error}");
                    None
                }
            },
        });
        // The MCP listener exists only while Allow local agents is on; Core
        // starts, stops and retries it.
        core.attach_agent_listener(Arc::new(crate::agent_server::AgentListenerPort(
            app.clone(),
        )));
        #[cfg(target_os = "macos")]
        if app.path().resource_dir().is_ok_and(|path| {
            path.parent().is_some_and(|contents| {
                contents.join("PlugIns/IsctWidgetsExtension.appex").is_dir()
            })
        }) {
            match crate::widgets_macos::MacWidgets::open() {
                Ok(port) => core.attach_widgets(Arc::new(port), false),
                Err(error) => log::error!("[widgets] {error}"),
            }
        }
        let tasks = app.state::<Self>().tasks.clone();
        // Setup is called on the UI thread; enter Tauri's shared Tokio runtime
        // before registering the owned timer.
        tauri::async_runtime::block_on(async move {
            if let Err(error) = core.start(&tokio::runtime::Handle::current()) {
                log::error!("[events] sync snapshot forwarding did not start: {error}");
            }
            let timer_tasks = tasks.clone();
            tasks.spawn(deadline_timer(core, timer_tasks));
        });
    }

    pub(crate) fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub(crate) fn begin_exit(&self) -> bool {
        !self.exiting.swap(true, Ordering::AcqRel)
    }

    pub(crate) async fn shutdown(&self, core: &app_core::Core) {
        self.tasks.close();
        core.shutdown().await;
        self.tasks.wait().await;
    }

    /// Allows the next exit request through, drained or not.
    pub(crate) fn mark_stopped(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

/// The desktop's one timer for Core: it sleeps until the directive's
/// deadline, tells Core, and sets itself again from the next directive.
/// A deadline it already reported is not reported again until the
/// directive moves.
async fn deadline_timer(core: Arc<app_core::Core>, tasks: RuntimeTasks) {
    let mut directives = core.wake_directives();
    let mut reported = None;
    loop {
        let directive = directives.borrow_and_update().clone();
        let now = chrono::Utc::now();
        let wait = match directive.at {
            Some(_) if reported == Some(directive.revision) => WALL_CLOCK_CHECK,
            Some(at) => (at - now)
                .to_std()
                .unwrap_or_default()
                .min(WALL_CLOCK_CHECK),
            None => WALL_CLOCK_CHECK,
        };
        tokio::select! {
            _ = tasks.closed() => break,
            changed = directives.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep(wait) => {
                if directive.at.is_some_and(|at| at <= chrono::Utc::now())
                    && reported != Some(directive.revision)
                {
                    reported = Some(directive.revision);
                    core.deadline_reached(directive.revision);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/core_ports.rs"]
mod tests;
