//! The installer's work, as root: install the `titech-lpd` backend and
//! create or update the `InstitutePrinter` queue. `lpadmin -p` updates a
//! queue of the same name in place, keeping its waiting jobs, so installing
//! again overwrites it.

use crate::queue::{self, BACKEND_MODE, BACKEND_NAME, PPD_MODE, PRINTER_NAME};
use crate::student_id::StudentId;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

extern "C" {
    fn geteuid() -> u32;
}

/// Installs for `args` = `[student ID, login name]`, with `backend` as the
/// backend program. Returns what it did, for the log.
pub fn run(args: &[String], backend: &[u8]) -> Result<String, String> {
    let [student, login] = args else {
        return Err("usage: institute-printer-installer <student ID> <login name>".into());
    };
    // SAFETY: geteuid takes no arguments and cannot fail.
    if unsafe { geteuid() } != 0 {
        return Err("the installer must run as root".to_string());
    }
    let student =
        StudentId::parse(student).ok_or_else(|| format!("not a student ID: {student}"))?;
    if !queue::is_login_name(login) {
        return Err(format!("not a login name: {login:?}"));
    }
    let server_bin = Path::new(queue::SERVER_BIN);
    if !server_bin.is_dir() {
        return Err(format!(
            "CUPS is not installed: {} does not exist",
            server_bin.display()
        ));
    }

    install_file_atomic(
        &server_bin.join("backend").join(BACKEND_NAME),
        backend,
        BACKEND_MODE,
    )?;

    let (ppd, rendering) = queue::queue_ppd();
    let ppd_path = temp_path_next_to(&env::temp_dir().join(format!("{PRINTER_NAME}.ppd")));
    let created = (|| {
        write_file(&ppd_path, &ppd, PPD_MODE)?;
        run_tool(
            queue::LPADMIN,
            &queue::lpadmin_args(&student, login, path_str(&ppd_path)?),
        )
    })();
    let _ = fs::remove_file(&ppd_path);
    // `lpadmin -E` has already enabled the queue and made it accept jobs. Do not
    // run `cupsenable` after it: macOS's CUPS policy refuses Resume-Printer to
    // root, which would report a working queue as failed.
    created?;

    Ok(format!(
        "Installed {PRINTER_NAME} for {student}, used by {login}; rendering: {rendering}."
    ))
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("failed to chmod {mode:o} {}: {e}", path.display()))
}

/// Writes `bytes` to `path` owned by root (uid 0, gid 0 `wheel`) with `mode`, replacing it in one rename.
fn install_file_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;

    let temp = temp_path_next_to(path);
    let result = (|| {
        {
            let mut file = File::create(&temp)
                .map_err(|e| format!("failed to create {}: {e}", temp.display()))?;
            file.write_all(bytes)
                .map_err(|e| format!("failed to write {}: {e}", temp.display()))?;
            file.sync_all()
                .map_err(|e| format!("failed to sync {}: {e}", temp.display()))?;
        }
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("failed to chmod {mode:o} {}: {e}", temp.display()))?;
        std::os::unix::fs::chown(&temp, Some(0), Some(0))
            .map_err(|e| format!("failed to chown {}: {e}", temp.display()))?;
        fs::rename(&temp, path).map_err(|e| {
            format!(
                "failed to rename {} to {}: {e}",
                temp.display(),
                path.display()
            )
        })
    })();
    if temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn temp_path_next_to(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("install-file");
    path.with_file_name(format!(".{name}.{}.{nanos}.tmp", std::process::id()))
}

fn run_tool(program: &str, args: &[String]) -> Result<Output, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute {program} {args:?}: {e}"))?;
    if !output.status.success() {
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string());
        return Err(format!(
            "command failed: {program} {args:?}; exit code: {code}; stderr: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn path_str(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}
