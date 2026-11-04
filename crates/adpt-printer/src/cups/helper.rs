//! The root CUPS helper, one executable with two roles:
//!
//! - `institute-printer-helper install <student ID> <login name>`, run once
//!   as root by the app's admin prompt: installs a copy of itself as the
//!   `titech-lpd` backend and creates or updates the `InstitutePrinter`
//!   queue, for that login only.
//! - anything else is CUPS running it as the `titech-lpd` backend, as root:
//!   with no arguments to discover it, or with a job's 5 or 6 arguments,
//!   when argv[0] is the device URI rather than a name. It reads the student
//!   ID from the device URI, then delegates the upload to the system lpd
//!   backend with that ID as the LPD user, one upload per copy.
//!
//! It uses std only: `build.rs` compiles this file with rustc alone for the
//! library to embed, and the shared modules come in by path.
// The shared modules carry items only the library uses.
#![allow(dead_code)]

#[path = "../endpoint.rs"]
mod endpoint;
#[path = "queue.rs"]
mod queue;
#[path = "../student_id.rs"]
mod student_id;

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    // A CUPS job's first argument is a job id, never `install`; argv[0]
    // says nothing (CUPS sets it to the device URI).
    let result = if args.get(1).map(String::as_str) == Some(queue::INSTALL) {
        match &args[..] {
            [_, _, student, login] => unix::install(student, login).map(|()| 0),
            _ => Err("usage: institute-printer-helper install <student ID> <login name>".into()),
        }
    } else {
        unix::backend(&args)
    };
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
    }
}

/// The helper exists only where CUPS does; the bin target still has to
/// build on Windows.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    std::process::exit(2);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::queue::{self, BACKEND_MODE, BACKEND_NAME, PPD, PPD_MODE, PRINTER_NAME};
    use super::student_id::StudentId;
    use std::env;
    use std::ffi::OsStr;
    use std::fs::{self, File};
    use std::io::{self, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::time::{SystemTime, UNIX_EPOCH};

    extern "C" {
        fn geteuid() -> u32;
    }

    pub(super) fn install(student: &str, login: &str) -> Result<(), String> {
        // SAFETY: geteuid takes no arguments and cannot fail.
        if unsafe { geteuid() } != 0 {
            return Err("the installer must run as root".to_string());
        }
        let student =
            StudentId::parse(student).ok_or_else(|| format!("not a student ID: {student}"))?;
        if !queue::is_login_name(login) {
            return Err(format!("not a login name: {login:?}"));
        }

        let serverbin = cups_serverbin();
        let backend_path = serverbin.join("backend").join(BACKEND_NAME);
        let filter_dir = serverbin.join("filter");
        let kyocera_driver =
            queue::check_kyocera_driver(&queue::kyocera_driver_files(PPD, &filter_dir));

        let me = env::current_exe().map_err(|e| format!("cannot find the helper: {e}"))?;
        let backend = fs::read(&me).map_err(|e| format!("cannot read {}: {e}", me.display()))?;
        install_file_atomic(&backend_path, &backend, BACKEND_MODE)?;

        // `lpadmin -p` updates a queue of the same name in place, keeping
        // its waiting jobs: installing again overwrites it.
        let lpadmin = command_path("/usr/sbin/lpadmin", "lpadmin");

        let ppd_path = temp_path_next_to(&env::temp_dir().join(format!("{PRINTER_NAME}.ppd")));
        let created = (|| {
            write_file(
                &ppd_path,
                &queue::queue_ppd(PPD, kyocera_driver.is_ok()),
                PPD_MODE,
            )?;
            run(
                &lpadmin,
                &queue::lpadmin_args(&student, login, path_str(&ppd_path)?),
            )
        })();
        let _ = fs::remove_file(&ppd_path);
        created?;

        run(
            &command_path("/usr/sbin/cupsaccept", "cupsaccept"),
            &[PRINTER_NAME.to_string()],
        )?;
        run(
            &command_path("/usr/sbin/cupsenable", "cupsenable"),
            &[PRINTER_NAME.to_string()],
        )?;

        println!("Installed {PRINTER_NAME} for {student}, used by {login}.");
        match kyocera_driver {
            Ok(()) => println!("Rendering: Kyocera driver software."),
            Err(reason) => println!("Rendering: standard CUPS filters ({reason})."),
        }
        Ok(())
    }

    pub(super) fn backend(args: &[String]) -> Result<i32, String> {
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
        let delegate = lpd_backend_path();
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

    fn lpd_backend_path() -> PathBuf {
        if let Ok(serverbin) = env::var("CUPS_SERVERBIN") {
            let path = PathBuf::from(serverbin).join("backend/lpd");
            if path.exists() {
                return path;
            }
        }
        let candidates = queue::default_lpd_backend_paths();
        candidates
            .iter()
            .find(|candidate| candidate.exists())
            .or(candidates.first())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("/usr/libexec/cups/backend/lpd"))
    }

    fn spool_stdin_to_temp() -> Result<PathBuf, String> {
        let path = env::temp_dir().join(format!(
            "titech-lpd-{}-{}.print",
            std::process::id(),
            nanos()
        ));
        let mut file =
            File::create(&path).map_err(|e| format!("failed to create {}: {e}", path.display()))?;
        io::copy(&mut io::stdin(), &mut file)
            .map_err(|e| format!("failed to spool stdin to {}: {e}", path.display()))?;
        file.flush()
            .map_err(|e| format!("failed to flush {}: {e}", path.display()))?;
        Ok(path)
    }

    fn cups_serverbin() -> PathBuf {
        for candidate in [
            "/usr/sbin/cups-config",
            "/usr/bin/cups-config",
            "cups-config",
        ] {
            if let Ok(output) = Command::new(candidate).arg("--serverbin").output() {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !text.is_empty() {
                        return PathBuf::from(text);
                    }
                }
            }
        }
        queue::default_cups_serverbin()
    }

    fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
        fs::write(path, bytes).map_err(|e| format!("failed to write {}: {e}", path.display()))?;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("failed to chmod {mode:o} {}: {e}", path.display()))
    }

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
            run(
                &command_path("/usr/sbin/chown", "chown"),
                &[
                    queue::ROOT_OWNER_GROUP.to_string(),
                    path_str(&temp)?.to_string(),
                ],
            )?;
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

    fn nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn temp_path_next_to(path: &Path) -> PathBuf {
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("install-file");
        path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), nanos()))
    }

    fn command_path(abs: &str, fallback: &str) -> PathBuf {
        if Path::new(abs).exists() {
            PathBuf::from(abs)
        } else {
            PathBuf::from(fallback)
        }
    }

    fn run(program: &Path, args: &[String]) -> Result<Output, String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("failed to execute {} {args:?}: {e}", program.display()))?;
        if !output.status.success() {
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "terminated by signal".to_string());
            return Err(format!(
                "command failed: {} {args:?}; exit code: {code}; stderr: {}",
                program.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(output)
    }

    fn path_str(path: &Path) -> Result<&str, String> {
        path.to_str()
            .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
    }
}
