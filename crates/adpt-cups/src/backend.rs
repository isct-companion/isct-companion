//! The `titech-lpd` CUPS backend, run by CUPS as root (mode 0700). With no
//! arguments CUPS is discovering backends; with a job's 5 or 6 arguments
//! it reads the student ID from `DEVICE_URI` and hands the upload to the
//! system lpd backend with that ID as the LPD user, once per copy, so the
//! printer never has to interpret a copy count.

use crate::queue;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The exit code for CUPS, or why the job failed.
pub fn run(args: &[String]) -> Result<i32, String> {
    if args.len() == 1 {
        println!(r#"network titech-lpd "Unknown" "Science Tokyo campus printer""#);
        return Ok(0);
    }
    if args.len() != 6 && args.len() != 7 {
        return Err(format!(
            "expected 5 or 6 CUPS arguments, got {}",
            args.len().saturating_sub(1)
        ));
    }

    let device_uri = env::var("DEVICE_URI").map_err(|_| "DEVICE_URI is not set".to_string())?;
    let (student, delegate_uri) = queue::parse_device_uri(&device_uri)?;
    let delegate = lpd_backend()?;
    let copies = queue::parse_copies(&args[4])?;
    let stdin_file = if args.len() == 6 && copies > 1 {
        Some(spool_stdin_to_temp()?)
    } else {
        None
    };

    eprintln!(
        "INFO: titech-lpd: delegating to {} as {student}, {copies} upload(s)",
        delegate.display()
    );

    let result = (|| {
        for copy_index in 0..copies {
            let status = Command::new(&delegate)
                .args(queue::delegate_args(
                    args,
                    copy_index,
                    &student,
                    stdin_file.as_deref(),
                ))
                .env("DEVICE_URI", &delegate_uri)
                .status()
                .map_err(|e| format!("failed to execute {}: {e}", delegate.display()))?;
            let code = status.code().unwrap_or(1);
            if code != 0 {
                return Ok(code);
            }
        }
        Ok(0)
    })();

    if let Some(path) = stdin_file {
        let _ = fs::remove_file(path);
    }
    result
}

/// The system lpd backend: where macOS keeps it.
#[cfg(target_os = "macos")]
fn lpd_backend() -> Result<PathBuf, String> {
    Ok(PathBuf::from(queue::SERVER_BIN).join("backend/lpd"))
}

/// The system lpd backend: in the `ServerBin` cupsd reports to every
/// backend, which differs between distributions.
#[cfg(target_os = "linux")]
fn lpd_backend() -> Result<PathBuf, String> {
    env::var_os("CUPS_SERVERBIN")
        .map(|serverbin| PathBuf::from(serverbin).join("backend/lpd"))
        .ok_or_else(|| "CUPS_SERVERBIN is not set; titech-lpd runs only under cupsd".to_string())
}

fn spool_stdin_to_temp() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = env::temp_dir().join(format!("titech-lpd-{}-{nanos}.print", std::process::id()));
    let mut file =
        File::create(&path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
    io::copy(&mut io::stdin(), &mut file)
        .map_err(|e| format!("failed to spool stdin to {}: {e}", path.display()))?;
    file.flush()
        .map_err(|e| format!("failed to flush {}: {e}", path.display()))?;
    Ok(path)
}
