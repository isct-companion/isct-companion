//! The campus printer tool, as the `campus-printer` plugin. Core decides
//! whose jobs the printer bills and whether it answers
//! (`app_core::campus_printer`); this plugin adds the printer to the system:
//! the CUPS queue on macOS (`cups`); on Windows the PDF right-click entry and
//! its print window (`windows`) and a system printer for every other app
//! (`queue`). Its commands are
//! `plugin:campus-printer|<name>`: `build.rs` lists them and which the main
//! window may call, and the Windows half grants its print windows theirs.
use crate::{core_ports, CommandError};
use app_core::campus_printer::{CampusPrinterStatus, PrinterAccount};
use serde::Serialize;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::Manager;

#[cfg(target_os = "macos")]
mod cups;
#[cfg(windows)]
mod queue;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::handles_launch;

pub fn init() -> TauriPlugin<tauri::Wry> {
    Builder::new("campus-printer")
        .invoke_handler(tauri::generate_handler![
            status,
            #[cfg(target_os = "macos")]
            cups::install,
            #[cfg(windows)]
            windows::add_menu,
            #[cfg(windows)]
            queue::install,
            #[cfg(windows)]
            queue::queue_status,
            #[cfg(windows)]
            windows::print_request,
            #[cfg(windows)]
            windows::print,
        ])
        .setup(|app, _| {
            app.manage(Setup::default());
            super::grant_main(app, "campus-printer")?;
            #[cfg(windows)]
            windows::setup(app)?;
            #[cfg(windows)]
            queue::setup(app);
            Ok(())
        })
        // After the app's setup, so the main window and Core's runtime exist
        // before a print window started by the right-click entry opens.
        .on_event(|_app, _event| {
            #[cfg(windows)]
            if let tauri::RunEvent::Ready = _event {
                queue::resume(_app);
                windows::handles_launch(_app, std::env::args_os());
            }
        })
        .build()
}

/// What the setting shows when it opens and on each Check: found once per
/// call, never in the background.
#[tauri::command]
async fn status(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<CampusPrinterStatus, CommandError> {
    crate::require_local_main(&webview)?;
    Ok(core_ports::core(&app).campus_printer_status().await)
}

/// How adding the printer to the system ended.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CampusPrinterSetup {
    Added,
    /// The person dismissed the administrator prompt.
    Cancelled,
    /// No verified student ID: nothing was changed.
    Account {
        account: PrinterAccount,
    },
    /// The system step failed; the log has why.
    Failed,
}

/// Serialises clicks: one setup at a time.
#[derive(Default)]
pub struct Setup(pub(crate) tokio::sync::Mutex<()>);
