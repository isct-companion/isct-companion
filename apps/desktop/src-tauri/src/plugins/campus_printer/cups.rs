//! Installs the `InstitutePrinter` CUPS queue on macOS behind one
//! administrator prompt: Core hands over the root installer and its
//! arguments for the verified student ID, this writes the installer to a
//! private folder and runs it as root. Nothing is downloaded, and nothing records whether
//! the queue exists: installing again overwrites it, and the system printer
//! settings remove it.
//!
//! The prompt is the system's own: AppleScript's `with administrator
//! privileges`. `runas`, the crate for this, does not fit: on macOS it uses
//! the deprecated `AuthorizationExecuteWithPrivileges` and does not report
//! the child's exit status reliably.
use super::{CampusPrinterSetup, Setup};
use crate::{core_ports, shell, CommandError};
use app_core::campus_printer::CupsInstall;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, Manager};

/// The installer's result as the prompt reports it.
enum Elevated {
    Done,
    Cancelled,
    Failed(String),
}

/// `prompt` is the administrator dialog's text, in the UI language.
#[tauri::command]
pub(super) async fn install(
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
    let login = login_name().map_err(shell)?;
    let install = match core_ports::core(&app).campus_printer_install(login).await {
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
        let installer = dir.join("institute-printer-installer");
        // Closed before it runs.
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&installer)?
            .write_all(install.installer)?;
        elevate(&installer, &install.args, prompt)
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

/// This process's login name, from the user database rather than `$USER`,
/// so the queue goes to the account that is really running the app.
fn login_name() -> io::Result<String> {
    // SAFETY: getuid cannot fail. getpwuid_r writes only into `entry` and
    // `buffer`, both outliving the call, and `found` points at `entry` or is
    // null; `pw_name` then points into `buffer`, which is still alive.
    unsafe {
        let mut entry: libc::passwd = std::mem::zeroed();
        let mut buffer = vec![0 as libc::c_char; 16 * 1024];
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let code = libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        );
        if code != 0 || found.is_null() || entry.pw_name.is_null() {
            return Err(io::Error::other("this user has no user database entry"));
        }
        std::ffi::CStr::from_ptr(entry.pw_name)
            .to_str()
            .map(str::to_owned)
            .map_err(|_| io::Error::other("the login name is not UTF-8"))
    }
}

fn elevate(installer: &Path, args: &[String; 2], prompt: &str) -> io::Result<Elevated> {
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
        .arg(installer)
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
