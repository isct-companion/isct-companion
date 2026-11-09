//! Printing to the campus printer on Windows: an entry in Explorer's
//! right-click menu for PDF files starts the app with the file, and a small
//! window asks for copies, duplex and color before Core sends it straight
//! to the printer over LPD.
//!
//! The entry is one named verb under the current user's
//! `SystemFileAssociations\.pdf`, so it needs no administrator and leaves
//! the default PDF app, double-click and Open with alone. Every other format
//! prints through the system printer (`queue`), which uses Windows' own IPP
//! class driver: a port monitor needs SYSTEM, and Windows Protected Print
//! mode blocks third-party ones anyway.
use super::{CampusPrinterSetup, Setup};
use crate::{core_ports, CommandError};
use app_core::campus_printer::{CampusPrintOutcome, PrintOptions};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindowBuilder};

const VERB_KEY: &str =
    r"Software\Classes\SystemFileAssociations\.pdf\shell\IsctCompanionCampusPrint";
/// The argument before the file in the verb's command line.
const PRINT_FLAG: &str = "--campus-print";
/// Print windows are `campus-print-<n>`; `capabilities/campus-print.json`
/// (core permissions) and `setup` (printing) match the same prefix.
const WINDOW_PREFIX: &str = "campus-print-";

/// The print windows' state, and their grant: the two printing commands,
/// for the bundled page only.
pub(super) fn setup(app: &AppHandle) -> tauri::Result<()> {
    app.manage(PrintRequests::default());
    app.add_capability(
        tauri::ipc::CapabilityBuilder::new("campus-print-window")
            .window(format!("{WINDOW_PREFIX}*"))
            .permission("campus-printer:allow-print-request")
            .permission("campus-printer:allow-print"),
    )
}

/// The file each open print window prints, by window label.
#[derive(Default)]
pub(super) struct PrintRequests(Mutex<HashMap<String, PathBuf>>);

/// Opens a print window when a launch's arguments ask to print a file, from
/// a task: a second launch reaches the app inside the single-instance
/// plugin's window-message handler, where building a window can deadlock.
/// False when they ask for nothing to print.
pub(crate) fn handles_launch<I, S>(app: &AppHandle, args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let Some(file) = requested_print(args) else {
        return false;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move { open_print_window(&app, file) });
    true
}

/// The file a launch's arguments ask to print, if any.
fn requested_print<I, S>(args: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args.into_iter().map(Into::into);
    args.find(|arg| arg == PRINT_FLAG)?;
    args.next().map(PathBuf::from)
}

/// Opens a print window for `file`, one per request.
fn open_print_window(app: &AppHandle, file: PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let label = format!("{WINDOW_PREFIX}{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let requests = app.state::<PrintRequests>();
    requests
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(label.clone(), file);
    let built = WebviewWindowBuilder::new(app, &label, crate::app_url())
        .title("IsctCompanion")
        .inner_size(400.0, 380.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .center()
        .focused(true)
        .disable_drag_drop_handler()
        .initialization_script(format!(
            "{}Object.defineProperty(window,'__ISCT_COMPANION_VIEW__',{{value:'campus-print',writable:false,configurable:false}});",
            crate::native_bootstrap()
        ))
        .build();
    match built {
        Ok(window) => {
            let app = app.clone();
            let closed = label.clone();
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Destroyed) {
                    app.state::<PrintRequests>()
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&closed);
                }
            });
        }
        Err(error) => {
            log::warn!("[campus-printer] the print window did not open: {error}");
            requests
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&label);
        }
    }
}

/// A print window's caller: one of ours, showing the bundled application.
fn require_print_window(webview: &tauri::Webview) -> Result<(), CommandError> {
    if !webview.label().starts_with(WINDOW_PREFIX) {
        return Err("print commands are restricted to the print window".into());
    }
    crate::require_bundled_origin(webview)
}

fn requested_file(app: &AppHandle, label: &str) -> Result<PathBuf, CommandError> {
    app.state::<PrintRequests>()
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(label)
        .cloned()
        .ok_or_else(|| "This print window has no file.".into())
}

/// Adds the PDF right-click entry, worded `label` (the UI language), for
/// the verified student. Adding it again rewrites it, with this copy of the
/// app as its program.
#[tauri::command]
pub(super) async fn add_menu(
    webview: tauri::Webview,
    app: AppHandle,
    state: tauri::State<'_, Setup>,
    label: String,
) -> Result<CampusPrinterSetup, CommandError> {
    crate::require_local_main(&webview)?;
    let _busy = state
        .0
        .try_lock()
        .map_err(|_| "The printer is already being added.".to_string())?;
    let account = core_ports::core(&app).campus_printer_account().await;
    if !matches!(
        account,
        app_core::campus_printer::PrinterAccount::Ready { .. }
    ) {
        return Ok(CampusPrinterSetup::Account { account });
    }
    let outcome = match register(&label) {
        Ok(()) => CampusPrinterSetup::Added,
        Err(error) => {
            log::warn!("[campus-printer] the right-click entry was not added: {error}");
            CampusPrinterSetup::Failed
        }
    };
    log::info!("[campus-printer] menu {outcome:?}");
    Ok(outcome)
}

/// Writes the verb, reads it back, and tells Explorer the associations
/// changed so an open window offers the entry without a restart.
fn register(label: &str) -> Result<(), String> {
    use ::windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.to_str().ok_or("the app's path is not Unicode")?;
    let command = format!("\"{exe}\" {PRINT_FLAG} \"%1\"");
    let verb = windows_registry::CURRENT_USER
        .create(VERB_KEY)
        .map_err(|e| e.to_string())?;
    verb.set_string("", label).map_err(|e| e.to_string())?;
    verb.set_string("Icon", format!("\"{exe}\",0"))
        .map_err(|e| e.to_string())?;
    verb.create("command")
        .and_then(|key| key.set_string("", &command))
        .map_err(|e| e.to_string())?;
    let written = windows_registry::CURRENT_USER
        .open(VERB_KEY)
        .and_then(|key| Ok((key.get_string("")?, key.open("command")?.get_string("")?)))
        .map_err(|e| e.to_string())?;
    if written != (label.to_owned(), command) {
        return Err("the right-click entry did not read back as written".into());
    }
    // SAFETY: SHCNE_ASSOCCHANGED takes no items; both pointers are null.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

/// What the print window shows before printing.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusPrintRequest {
    file_name: String,
    /// This device's UI language, appearance and theme, which the main
    /// window otherwise reads from the workspace.
    locale: Option<Value>,
    appearance: Option<Value>,
    theme: Option<Value>,
}

#[tauri::command]
pub(super) fn print_request(
    webview: tauri::Webview,
    app: AppHandle,
) -> Result<CampusPrintRequest, CommandError> {
    require_print_window(&webview)?;
    let file = requested_file(&app, webview.label())?;
    let core = core_ports::core(&app);
    let preference = |pointer| core.store.device_preference(pointer).ok().flatten();
    Ok(CampusPrintRequest {
        file_name: file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        locale: preference("/locale"),
        appearance: preference("/appearance"),
        theme: preference("/theme"),
    })
}

/// Prints the window's file with `options`, billed to the verified student
/// ID.
#[tauri::command]
pub(super) async fn print(
    webview: tauri::Webview,
    app: AppHandle,
    options: PrintOptions,
) -> Result<CampusPrintOutcome, CommandError> {
    require_print_window(&webview)?;
    let file = requested_file(&app, webview.label())?;
    Ok(core_ports::core(&app).campus_print(file, options).await)
}
