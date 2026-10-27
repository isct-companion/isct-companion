//! Closing the main window: quit, keep running in the background (macOS
//! keeps the Dock icon, Windows a tray icon), or ask with the system's own
//! prompt, which can remember the answer as the `closeWindow` device
//! preference. Linux keeps the default: closing the window quits.
use crate::core_ports;
use app_core::workspace::{WorkspaceIntent, WorkspaceMutation};
use serde_json::Value;
use tauri::{AppHandle, Manager};

#[cfg(target_os = "macos")]
use crate::window_close_macos as platform;
#[cfg(target_os = "windows")]
use crate::window_close_windows as platform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseAction {
    Quit,
    Background,
}

/// The prompt's copy in this device's language.
pub(crate) struct Prompt {
    pub title: String,
    pub message: String,
    pub quit: String,
    pub keep: String,
    /// Windows labels its own Cancel button.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub cancel: String,
    pub remember: String,
}

impl Prompt {
    fn new(app: &AppHandle) -> Self {
        Self {
            title: text(app, "Close IsctCompanion?"),
            message: text(app, "Keeping it running lets reminders and sync continue."),
            quit: text(app, "Quit"),
            keep: text(app, KEEP_LABEL),
            cancel: text(app, "Cancel"),
            remember: text(app, "Remember my choice"),
        }
    }
}

#[cfg(target_os = "macos")]
const KEEP_LABEL: &str = "Keep in Dock";
#[cfg(target_os = "windows")]
const KEEP_LABEL: &str = "Minimize to tray";

/// The main window's close button, Cmd-W or Alt-F4.
pub(crate) fn close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    if window.label() != "main" {
        return;
    }
    let app = window.app_handle();
    let stored = core_ports::core(app)
        .store
        .device_preference("/closeWindow")
        .unwrap_or_else(|error| {
            log::warn!("[window] close preference is unreadable: {error}");
            None
        });
    match stored.as_ref().and_then(Value::as_str) {
        // The last window closing exits through the owned shutdown.
        Some("quit") => {}
        Some("background") => {
            api.prevent_close();
            to_background(app);
        }
        _ => {
            api.prevent_close();
            platform::ask(window, Prompt::new(app));
        }
    }
}

/// The prompt's answer; `None` is Cancel, which keeps the window and
/// remembers nothing.
pub(crate) fn answered(app: &AppHandle, action: Option<CloseAction>, remember: bool) {
    let Some(action) = action else { return };
    if remember {
        let stored = match action {
            CloseAction::Quit => "quit",
            CloseAction::Background => "background",
        };
        let workspace = &core_ports::core(app).workspace;
        let saved = workspace.revision().and_then(|revision| {
            workspace.mutate_commit(WorkspaceMutation {
                expected_revision: revision,
                intent: WorkspaceIntent::SetCloseWindow {
                    action: stored.into(),
                },
            })
        });
        if let Err(error) = saved {
            log::warn!("[window] could not remember the close choice: {error}");
        }
    }
    match action {
        CloseAction::Quit => app.exit(0),
        CloseAction::Background => to_background(app),
    }
}

// `get_window`, not `get_webview_window`: once a school view is added as a
// child, tauri no longer counts the main window as a webview window.
fn to_background(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.hide();
    }
    #[cfg(target_os = "windows")]
    platform::show_tray(app);
}

/// Brings the hidden window back: the Dock icon, the tray icon or a second
/// launch.
pub(crate) fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

const JA: &str = include_str!("../../../../resources/locales/ja.json");
const ZH: &str = include_str!("../../../../resources/locales/zh.json");

/// A string from the shared catalogs in `resources/locales`, in this
/// device's language. English keys are their own text.
pub(crate) fn text(app: &AppHandle, key: &str) -> String {
    let locale = core_ports::core(app)
        .store
        .device_preference("/locale")
        .ok()
        .flatten();
    let catalog = match locale.as_ref().and_then(Value::as_str) {
        Some("ja") => JA,
        Some("zh") => ZH,
        _ => return key.to_string(),
    };
    serde_json::from_str::<serde_json::Map<String, Value>>(catalog)
        .ok()
        .and_then(|catalog| catalog.get(key)?.as_str().map(str::to_owned))
        .unwrap_or_else(|| key.to_string())
}
