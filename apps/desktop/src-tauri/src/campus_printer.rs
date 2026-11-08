//! The campus printer setting, on every desktop. Core decides whose jobs
//! the printer bills and whether it answers (`app_core::campus_printer`);
//! this shell adds the printer to the system: the CUPS queue on macOS and
//! Linux (`campus_printer_cups`), the PDF right-click entry and local IPP
//! queue on Windows (`campus_printer_windows`, `campus_printer_ipp_windows`).
use crate::{core_ports, CommandError};
use app_core::campus_printer::{CampusPrinterStatus, PrinterAccount};
use serde::Serialize;

/// What the setting shows when it opens and on each Check: found once per
/// call, never in the background.
#[tauri::command]
pub async fn campus_printer_status(
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
