//! What the root helper does that needs no root: the device URI that
//! carries the student ID, the queue's copy of the PPD, the arguments for
//! `lpadmin` and for the system lpd backend, and whether Kyocera's own
//! driver software can be used. Shared with the helper by `#[path]`, so it
//! uses std only.

use crate::endpoint::{HOST, QUEUE};
use crate::student_id::StudentId;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const PRINTER_NAME: &str = "InstitutePrinter";
pub const BACKEND_NAME: &str = "titech-lpd";
pub const BACKEND_MODE: u32 = 0o700;
pub const PPD_MODE: u32 = 0o644;
const BACKEND_SCHEME: &str = "titech-lpd://";
const DELEGATE_SCHEME: &str = "lpd://";
const STUDENT_PARAMETER: &str = "student=";

/// An unmodified copy of the PPD in Kyocera's macOS driver package
/// (4.0.7420, `*FileVersion: "8.4506"`). Its licence allows redistribution
/// only unaltered, so the file stays byte-for-byte (`.gitattributes`); the
/// queue gets an edited copy (`queue_ppd`).
pub const PPD: &[u8] = include_bytes!("../../assets/Kyocera ECOSYS P8060cdn.PPD");

/// Lines that point CUPS at the Kyocera driver software. They are kept only
/// when that software is installed and runnable; otherwise CUPS uses its own
/// pdftops/pstops filters, which already emit every PJL and PostScript
/// option command defined in the PPD.
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

/// `lpadmin`'s arguments that create the queue from `ppd`.
pub fn lpadmin_args(student: &StudentId, ppd: &str) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        PRINTER_NAME.to_string(),
        "-E".to_string(),
        "-v".to_string(),
        device_uri(student),
        "-P".to_string(),
        ppd.to_string(),
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

/// Files the PPD's Kyocera-only lines refer to: filters inside the CUPS
/// filter directory and the print dialog plugin bundle at an absolute path.
pub fn kyocera_driver_files(ppd: &[u8], filter_dir: &Path) -> Vec<PathBuf> {
    String::from_utf8_lossy(ppd)
        .lines()
        .filter(|line| is_kyocera_driver_line(line.as_bytes()))
        .filter_map(|line| {
            let value = line.split('"').nth(1)?;
            if line.starts_with("*APDialogExtension") {
                Some(PathBuf::from(value))
            } else {
                value
                    .split_whitespace()
                    .last()
                    .map(|name| filter_dir.join(name))
            }
        })
        .collect()
}

/// `Err` explains why the Kyocera driver software cannot be used.
pub fn check_kyocera_driver(files: &[PathBuf]) -> Result<(), String> {
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

/// The queue's copy of the PPD: without the Kyocera driver lines unless
/// that software is usable.
pub fn queue_ppd(ppd: &[u8], use_kyocera_driver: bool) -> Vec<u8> {
    if use_kyocera_driver {
        return ppd.to_vec();
    }

    ppd.split_inclusive(|&byte| byte == b'\n')
        .filter(|line| !is_kyocera_driver_line(line))
        .flatten()
        .copied()
        .collect()
}

/// CUPS backend arguments: job-id user title copies options [file].
pub fn parse_copies(text: &str) -> Result<u32, String> {
    let copies = text
        .parse::<u32>()
        .map_err(|e| format!("invalid copies value {text}: {e}"))?;
    if copies == 0 {
        return Err("copies value must be at least 1".to_string());
    }
    Ok(copies)
}

/// The system lpd backend's arguments for upload `copy_index` (from 0):
/// the student ID as the user, one copy, and the spooled file when stdin
/// was spooled for repeated uploads.
pub fn delegate_args(
    args: &[String],
    copy_index: u32,
    student: &StudentId,
    stdin_file: Option<&Path>,
) -> Vec<String> {
    let mut delegate = Vec::with_capacity(6);
    delegate.push(copy_job_id(&args[1], copy_index));
    delegate.push(student.as_str().to_string());
    delegate.push(args[3].clone());
    delegate.push("1".to_string());
    delegate.push(args[5].clone());

    if let Some(path) = stdin_file {
        delegate.push(path.to_string_lossy().into_owned());
    } else if args.len() == 7 {
        delegate.push(args[6].clone());
    }

    delegate
}

fn copy_job_id(job_id: &str, copy_index: u32) -> String {
    if copy_index == 0 {
        return job_id.to_string();
    }

    match job_id.parse::<u32>() {
        Ok(n) => n.saturating_add(copy_index).to_string(),
        Err(_) => job_id.to_string(),
    }
}

/// Where CUPS keeps its programs when `cups-config` cannot say.
#[cfg(target_os = "macos")]
pub fn default_cups_serverbin() -> PathBuf {
    PathBuf::from("/usr/libexec/cups")
}

#[cfg(target_os = "linux")]
pub fn default_cups_serverbin() -> PathBuf {
    PathBuf::from("/usr/lib/cups")
}

/// Where the system lpd backend is, the usual place first.
#[cfg(target_os = "macos")]
pub fn default_lpd_backend_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/libexec/cups/backend/lpd"),
        PathBuf::from("/usr/lib/cups/backend/lpd"),
    ]
}

#[cfg(target_os = "linux")]
pub fn default_lpd_backend_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/lib/cups/backend/lpd"),
        PathBuf::from("/usr/libexec/cups/backend/lpd"),
    ]
}

#[cfg(target_os = "macos")]
pub const ROOT_OWNER_GROUP: &str = "root:wheel";
#[cfg(target_os = "linux")]
pub const ROOT_OWNER_GROUP: &str = "root:root";
