//! The two programs as built for this machine, run for real.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output};

const URI: &str = "titech-lpd://172.20.83.212/kyocera?student=24M12345";

/// `bytes` as an executable in a folder of its own.
fn program(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("adpt-cups-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// CUPS runs a backend with the device URI as argv[0]: with no arguments to
// discover it, and with a job's arguments to print, which reach the URI
// check (a job with no student ID in its URI goes nowhere).
#[test]
fn the_backend_answers_cups_whatever_argv0_says() {
    let backend = program(
        "titech-lpd",
        &fs::read(concat!(env!("OUT_DIR"), "/titech-lpd")).unwrap(),
    );
    let run = |args: &[&str], device: &str| {
        Command::new(&backend)
            .arg0(URI)
            .args(args)
            .env("DEVICE_URI", device)
            .output()
            .unwrap()
    };

    let discovered = run(&[], URI);
    assert!(discovered.status.success());
    assert!(String::from_utf8_lossy(&discovered.stdout).starts_with("network titech-lpd "));

    let job = run(
        &["7", "user", "title", "1", "", "/nonexistent.ps"],
        "titech-lpd://172.20.83.212/kyocera",
    );
    assert!(!job.status.success());
    assert!(
        stderr(&job).contains("names no student ID"),
        "{}",
        stderr(&job)
    );
    fs::remove_dir_all(backend.parent().unwrap()).unwrap();
}

// The installer takes exactly a student ID and a login name, and does
// nothing without root.
#[test]
fn the_installer_needs_its_two_arguments_and_root() {
    let installer = program("institute-printer-installer", crate::INSTALLER);
    let run = |args: &[&str]| Command::new(&installer).args(args).output().unwrap();
    assert!(stderr(&run(&["24M12345"])).contains("usage:"));
    assert!(stderr(&run(&["install", "24M12345", "taro"])).contains("usage:"));
    let refused = run(&["24M12345", "taro"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("must run as root"),
        "{}",
        stderr(&refused)
    );
    fs::remove_dir_all(installer.parent().unwrap()).unwrap();
}
