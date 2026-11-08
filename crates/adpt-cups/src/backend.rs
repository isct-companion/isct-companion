//! The `titech-lpd` CUPS backend, run by CUPS as root (mode 0700). With no
//! arguments CUPS is discovering backends; with a job's 5 or 6 arguments it
//! reads the student ID from `DEVICE_URI` and becomes the system lpd
//! backend (exec) for the same job, with that ID as the user and the plain
//! `lpd://` URI. Everything else stays as CUPS gave it, so copies keep the
//! lpd backend's own meaning: a job on stdin was already copied by the
//! filters and goes once, and a job given as a file is sent once per copy.

use crate::queue;
use std::env;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

/// The exit code for CUPS, or why the job failed. A job never returns
/// here once the lpd backend has taken over.
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
    eprintln!(
        "INFO: titech-lpd: handing the job to {} as {student}",
        delegate.display()
    );
    let error = Command::new(&delegate)
        .arg0(&delegate_uri)
        .args(queue::delegate_args(args, &student))
        .env("DEVICE_URI", &delegate_uri)
        .exec();
    Err(format!("failed to execute {}: {error}", delegate.display()))
}

/// The system lpd backend: where macOS keeps it.
fn lpd_backend() -> Result<PathBuf, String> {
    Ok(PathBuf::from(queue::SERVER_BIN).join("backend/lpd"))
}
