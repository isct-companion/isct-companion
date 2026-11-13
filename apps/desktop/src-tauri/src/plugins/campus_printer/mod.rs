//! The campus printer tool, as the `campus-printer` plugin. Core decides
//! whose jobs the printer bills and whether it answers
//! (`app_core::campus_printer`); this plugin adds the printer to the system:
//! the CUPS queue on macOS (`cups`) or the system IPP queue on Windows
//! (`queue`). `build.rs` lists the commands granted to the main window.
use crate::{core_ports, CommandError};
use app_core::campus_printer::{CampusPrinterStatus, PrinterAccount};
use serde::Serialize;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::Manager;

#[cfg(target_os = "macos")]
mod cups;
#[cfg(windows)]
pub(crate) mod queue;

pub fn init() -> TauriPlugin<tauri::Wry> {
    Builder::new("campus-printer")
        .invoke_handler(tauri::generate_handler![
            status,
            #[cfg(target_os = "macos")]
            cups::install,
            #[cfg(windows)]
            queue::install,
        ])
        .setup(|app, _| {
            app.manage(Setup::default());
            super::grant_main(app, "campus-printer")?;
            Ok(())
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
    #[cfg(target_os = "macos")]
    Failed,
}

/// Serialises clicks: one setup at a time.
#[derive(Default)]
pub struct Setup(pub(crate) tokio::sync::Mutex<()>);
