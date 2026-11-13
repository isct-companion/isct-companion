//! The campus printer, a Kyocera ECOSYS P8060cdn at
//! lpd://172.20.83.212/kyocera, which bills each job to the student ID in
//! the LPD user field. That field is not authenticated: this crate only
//! keeps it well formed, and that the ID is the person's own is Core's
//! decision (`app_core::campus_printer`).
//!
//! This crate holds the printer's address and the student ID's format.
//! On macOS it prepares a CUPS queue with `lpadmin` arguments and a PPD.
//! On Windows and Android it sends documents through LPD with a PJL header.
//! Ported from InstitutePrinter's `master` (CUPS) and `cross-platform` branches.

mod endpoint;
mod student_id;

#[cfg(target_os = "macos")]
pub mod cups;

#[cfg(any(windows, target_os = "android"))]
pub mod document;
#[cfg(any(windows, target_os = "android"))]
pub mod lpd;
#[cfg(any(windows, target_os = "android"))]
pub mod pjl;

pub use endpoint::{ADDRESS, HOST, PORT, QUEUE};
pub use student_id::StudentId;

#[cfg(test)]
#[path = "tests/student_id.rs"]
mod student_id_tests;
