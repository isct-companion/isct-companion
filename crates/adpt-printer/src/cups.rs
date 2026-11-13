//! The `InstitutePrinter` CUPS queue on macOS, so every application prints
//! to the campus printer from its own print dialog with all of the PPD's
//! options. One root step, `lpadmin` with `lpadmin_args`, creates it or
//! updates it in place.
//!
//! The queue uses the system's own lpd backend: it takes the LPD user from
//! the device URI (`lpd://<student ID>@host/queue`) and writes it to the
//! control file's `P` line, which is what the printer bills.

use crate::endpoint::{HOST, QUEUE};
use crate::student_id::StudentId;

pub const PRINTER_NAME: &str = "InstitutePrinter";

/// An unmodified copy of the PPD in Kyocera's macOS driver package
/// (4.0.7420, `*FileVersion: "8.4506"`). Its licence allows redistribution
/// only unaltered, so the file stays byte-for-byte (`.gitattributes`); the
/// queue gets an edited copy (`queue_ppd`).
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

/// The queue's device URI: the campus printer, with the student ID as the
/// LPD user.
pub fn device_uri(student: &StudentId) -> String {
    format!("lpd://{}@{HOST}/{QUEUE}", student.as_str())
}

/// A login name as CUPS's allow list takes it: letters, digits, `.`, `_`
/// and `-`, not starting with `-`. That leaves out `,` (a list separator),
/// `@` (a group) and anything a shell or CUPS would read differently.
fn is_login_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `lpadmin`'s arguments that create or update the queue from the PPD at
/// `ppd`: only `login` may print to it, so another account on the computer
/// cannot bill this student, and it is never shared on the network.
/// `-E` enables it and makes it accept jobs. `None` when `login` is not a
/// login name.
pub fn lpadmin_args(student: &StudentId, login: &str, ppd: &str) -> Option<Vec<String>> {
    if !is_login_name(login) {
        return None;
    }
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
    Some(args)
}

/// `ppd` without the Kyocera driver lines.
fn without_kyocera_driver(ppd: &[u8]) -> Vec<u8> {
    ppd.split_inclusive(|&byte| byte == b'\n')
        .filter(|line| {
            !KYOCERA_ONLY_PPD_KEYWORDS
                .iter()
                .any(|keyword| line.starts_with(keyword.as_bytes()))
        })
        .flatten()
        .copied()
        .collect()
}

/// The queue's copy of the PPD: the bundled one without the Kyocera driver
/// lines, rendered by the standard CUPS filters.
pub fn queue_ppd() -> Vec<u8> {
    without_kyocera_driver(PPD)
}

#[cfg(test)]
#[path = "tests/cups.rs"]
mod tests;
