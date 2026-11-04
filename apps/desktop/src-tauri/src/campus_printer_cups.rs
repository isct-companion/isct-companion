//! Installs the `InstitutePrinter` CUPS queue (macOS and Linux) behind one
//! administrator prompt: Core hands over the root helper and its arguments
//! for the verified student ID, this writes the helper to a private folder
//! and runs it as root. Nothing is downloaded, and nothing records whether
//! the queue exists: installing again overwrites it, and the system printer
//! settings remove it.
//!
//! The prompt is the system's own: AppleScript's `with administrator
//! privileges` on macOS, polkit's `pkexec` on Linux. `runas`, the crate for
//! this, does not fit: on Linux it only calls `sudo`, which has no window
//! and would wait on a terminal the app does not have, and on macOS it uses
//! the deprecated `AuthorizationExecuteWithPrivileges` and does not report
//! the child's exit status reliably.
use crate::campus_printer::{CampusPrinterSetup, Setup};
use crate::{core_ports, shell, CommandError};
use app_core::campus_printer::CupsInstall;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, Manager};

/// The helper's result as the prompt reports it.
enum Elevated {
    Done,
    Cancelled,
    Failed(String),
}

/// `prompt` is the macOS dialog's text, in the UI language; polkit words
/// its own.
#[tauri::command]
pub async fn install_campus_printer(
    webview: tauri::Webview,
    app: AppHandle,
    state: tauri::State<'_, Setup>,
    prompt: String,
) -> Result<CampusPrinterSetup, CommandError> {
    crate::require_local_main(&webview)?;
    let _busy = state
        .0
        .try_lock()
        .map_err(|_| "The printer is already being added.".to_string())?;
    let install = match core_ports::core(&app).campus_printer_install().await {
        Ok(install) => install,
        Err(account) => return Ok(CampusPrinterSetup::Account { account }),
    };
    let cache = app.path().app_cache_dir().map_err(shell)?;
    let outcome = tauri::async_runtime::spawn_blocking(move || run(&cache, &install, &prompt))
        .await
        .map_err(shell)?;
    log::info!("[campus-printer] install {outcome:?}");
    Ok(outcome)
}

fn run(cache: &Path, install: &CupsInstall, prompt: &str) -> CampusPrinterSetup {
    // A folder only this user can enter, made new for this run.
    let dir = cache.join(format!("campus-printer-{}", uuid::Uuid::new_v4()));
    let result = (|| -> io::Result<Elevated> {
        fs::create_dir_all(cache)?;
        DirBuilder::new().mode(0o700).create(&dir)?;
        let helper = dir.join("institute-printer-helper");
        // Closed before it runs: Linux refuses to execute a file open for writing.
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&helper)?
            .write_all(install.helper)?;
        elevate(&helper, &install.args, prompt)
    })();
    let _ = fs::remove_dir_all(&dir);
    match result {
        Ok(Elevated::Done) => CampusPrinterSetup::Added,
        Ok(Elevated::Cancelled) => CampusPrinterSetup::Cancelled,
        Ok(Elevated::Failed(detail)) => {
            log::warn!("[campus-printer] the installer failed: {detail}");
            CampusPrinterSetup::Failed
        }
        Err(error) => {
            log::warn!("[campus-printer] the installer did not start: {error}");
            CampusPrinterSetup::Failed
        }
    }
}

#[cfg(target_os = "macos")]
fn elevate(helper: &Path, args: &[String; 2], prompt: &str) -> io::Result<Elevated> {
    // Every value goes in as an argument and through `quoted form of`, so no
    // text reaches the shell unquoted.
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "do shell script (quoted form of item 1 of argv) & \" \" & (quoted form of item 2 of argv) & \" \" & (quoted form of item 3 of argv) with prompt (item 4 of argv) with administrator privileges",
            "-e",
            "end run",
        ])
        .arg(helper)
        .args(args)
        .arg(prompt)
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(if output.status.success() {
        Elevated::Done
    } else if stderr.contains("(-128)") {
        // errAEEventUserCanceled: the person closed the prompt.
        Elevated::Cancelled
    } else {
        Elevated::Failed(stderr.trim().to_owned())
    })
}

#[cfg(target_os = "linux")]
fn elevate(helper: &Path, args: &[String; 2], _prompt: &str) -> io::Result<Elevated> {
    let output = Command::new("pkexec").arg(helper).args(args).output()?;
    Ok(match output.status.code() {
        Some(0) => Elevated::Done,
        // pkexec: the person dismissed the authentication dialog.
        Some(126) => Elevated::Cancelled,
        _ => Elevated::Failed(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
    })
}
