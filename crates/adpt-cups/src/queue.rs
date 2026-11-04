//! What the queue is made of, shared by the installer, the backend and the
//! library's tests: the device URI that carries the student ID, the
//! queue's copy of the PPD, the arguments for `lpadmin` and for the system
//! lpd backend, and the system's own paths. Included by the programs by
//! path, so it uses std only.

use crate::endpoint::{HOST, QUEUE};
use crate::student_id::StudentId;
#[cfg(target_os = "macos")]
use std::path::Path;

pub const PRINTER_NAME: &str = "InstitutePrinter";
pub const BACKEND_NAME: &str = "titech-lpd";
pub const BACKEND_MODE: u32 = 0o700;
pub const PPD_MODE: u32 = 0o644;
const BACKEND_SCHEME: &str = "titech-lpd://";
const DELEGATE_SCHEME: &str = "lpd://";
const STUDENT_PARAMETER: &str = "student=";

/// CUPS's program folder (`ServerBin`), where backends live.
#[cfg(target_os = "macos")]
pub const SERVER_BIN: &str = "/usr/libexec/cups";
#[cfg(target_os = "linux")]
pub const SERVER_BIN: &str = "/usr/lib/cups";

/// The system's CUPS administration tools.
pub const LPADMIN: &str = "/usr/sbin/lpadmin";
pub const CUPSACCEPT: &str = "/usr/sbin/cupsaccept";
pub const CUPSENABLE: &str = "/usr/sbin/cupsenable";

/// An unmodified copy of the PPD in Kyocera's macOS driver package
/// (4.0.7420, `*FileVersion: "8.4506"`). Its licence allows redistribution
/// only unaltered, so the file stays byte-for-byte (`.gitattributes`); the
/// queue gets an edited copy (`without_kyocera_driver`).
pub const PPD: &[u8] = include_bytes!("../assets/Kyocera ECOSYS P8060cdn.PPD");

/// Lines that point CUPS at the Kyocera driver software. Without them CUPS
/// uses its own pdftops/pstops filters, which already emit every PJL and
/// PostScript option command defined in the PPD.
const KYOCERA_ONLY_PPD_KEYWORDS: &[&str] = &["*cupsPreFilter", "*cupsFilter", "*APDialogExtension"];

pub const INSTALLABLE_OPTIONS: &[(&str, &str)] = &[
    ("Option19", "False"),    // Paper feeders
    ("Option26", "False"),    // Job separator
    ("Option25", "False"),    // Mailbox
    ("Option21", "False"),    // Punch unit
    ("Option22", "False"),    // Folding unit
    ("Option17", "None"),     // Document finisher
    ("Option18", "HardDisk"), // Optional disk
];

/// The queue's device URI: the campus printer, and the student ID the
/// backend writes into the LPD user field.
pub fn device_uri(student: &StudentId) -> String {
    format!(
        "{BACKEND_SCHEME}{HOST}/{QUEUE}?{STUDENT_PARAMETER}{}",
        student.as_str()
    )
}

/// The student ID in a device URI and the system lpd backend's URI for the
/// same printer.
pub fn parse_device_uri(uri: &str) -> Result<(StudentId, String), String> {
    let rest = uri
        .strip_prefix(BACKEND_SCHEME)
        .ok_or_else(|| format!("unsupported DEVICE_URI scheme: {uri}"))?;
    let (printer, query) = rest
        .split_once('?')
        .ok_or_else(|| format!("DEVICE_URI names no student ID: {uri}"))?;
    let student = query
        .split('&')
        .find_map(|pair| pair.strip_prefix(STUDENT_PARAMETER))
        .and_then(StudentId::parse)
        .ok_or_else(|| format!("DEVICE_URI names no valid student ID: {uri}"))?;
    Ok((student, format!("{DELEGATE_SCHEME}{printer}")))
}

/// A login name as CUPS's allow list takes it: letters, digits, `.`, `_`
/// and `-`, not starting with `-`. That leaves out `,` (a list separator),
/// `@` (a group) and anything a shell or CUPS would read differently.
pub fn is_login_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `lpadmin`'s arguments that create or update the queue from `ppd`: only
/// `login` may print to it, so another account on the computer cannot bill
/// this student, and it is never shared on the network.
pub fn lpadmin_args(student: &StudentId, login: &str, ppd: &str) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        PRINTER_NAME.to_string(),
        "-E".to_string(),
        "-v".to_string(),
        device_uri(student),
        "-P".to_string(),
        ppd.to_string(),
        "-u".to_string(),
        format!("allow:{login}"),
        "-o".to_string(),
        "printer-is-shared=false".to_string(),
    ];
    for (name, value) in INSTALLABLE_OPTIONS {
        args.push("-o".to_string());
        args.push(format!("{name}={value}"));
    }
    args
}

fn is_kyocera_driver_line(line: &[u8]) -> bool {
    KYOCERA_ONLY_PPD_KEYWORDS
        .iter()
        .any(|keyword| line.starts_with(keyword.as_bytes()))
}

/// The PPD without the Kyocera driver lines.
pub fn without_kyocera_driver(ppd: &[u8]) -> Vec<u8> {
    ppd.split_inclusive(|&byte| byte == b'\n')
        .filter(|line| !is_kyocera_driver_line(line))
        .flatten()
        .copied()
        .collect()
}

/// Files the PPD's Kyocera-only lines refer to: filters inside the CUPS
/// filter directory and the print dialog plugin bundle. Kyocera ships this
/// software for macOS only, so only macOS looks for it.
#[cfg(target_os = "macos")]
pub fn kyocera_driver_files(ppd: &[u8], filter_dir: &Path) -> Vec<std::path::PathBuf> {
    String::from_utf8_lossy(ppd)
        .lines()
        .filter(|line| is_kyocera_driver_line(line.as_bytes()))
        .filter_map(|line| {
            let value = line.split('"').nth(1)?;
            if line.starts_with("*APDialogExtension") {
                Some(std::path::PathBuf::from(value))
            } else {
                value
                    .split_whitespace()
                    .last()
                    .map(|name| filter_dir.join(name))
            }
        })
        .collect()
}

/// Whether the person installed Kyocera's driver software and this Mac can
/// run it; `Err` says why not.
#[cfg(target_os = "macos")]
pub fn check_kyocera_driver(files: &[std::path::PathBuf]) -> Result<(), String> {
    use std::process::{Command, Stdio};
    if files.is_empty() {
        return Err("the PPD references no Kyocera driver files".to_string());
    }

    for path in files {
        if !path.exists() {
            return Err(format!("{} is not installed", path.display()));
        }

        // Started without arguments, a CUPS filter exits immediately. Failing
        // to start means this machine cannot run it, e.g. an Intel-only filter
        // on Apple Silicon without Rosetta. A file the kernel cannot execute at
        // all is retried through /bin/sh, which then exits with 126 or 127.
        if path.is_file() {
            let status = Command::new(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| format!("{} cannot run on this machine: {e}", path.display()))?;
            if matches!(status.code(), Some(126 | 127)) {
                return Err(format!(
                    "{} cannot run on this machine: not an executable for this system",
                    path.display()
                ));
            }
        }
    }

    Ok(())
}

/// The queue's copy of the PPD and how it renders: unchanged when
/// Kyocera's driver software is usable, otherwise without its lines.
#[cfg(target_os = "macos")]
pub fn queue_ppd() -> (Vec<u8>, String) {
    let filters = Path::new(SERVER_BIN).join("filter");
    match check_kyocera_driver(&kyocera_driver_files(PPD, &filters)) {
        Ok(()) => (PPD.to_vec(), "Kyocera driver software".to_string()),
        Err(reason) => (
            without_kyocera_driver(PPD),
            format!("standard CUPS filters ({reason})"),
        ),
    }
}

/// Kyocera's driver software does not exist for Linux: CUPS's own filters.
#[cfg(target_os = "linux")]
pub fn queue_ppd() -> (Vec<u8>, String) {
    (
        without_kyocera_driver(PPD),
        "standard CUPS filters".to_string(),
    )
}

/// The system lpd backend's arguments for the job CUPS gave this backend
/// (`args`: argv[0], job-id, user, title, copies, options [, file]): the
/// same, with the student ID as the user.
pub fn delegate_args(args: &[String], student: &StudentId) -> Vec<String> {
    let mut delegate = args[1..].to_vec();
    delegate[1] = student.as_str().to_string();
    delegate
}
