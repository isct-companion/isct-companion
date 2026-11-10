//! The campus printer, a Kyocera ECOSYS P8060cdn at
//! lpd://172.20.83.212/kyocera, which bills each job to the student ID in
//! the LPD user field. That field is not authenticated: this crate only
//! keeps it well formed, and that the ID is the person's own is Core's
//! decision (`app_core::campus_printer`).
//!
//! This crate holds no state and decides nothing. Everywhere it holds the
//! printer's address and the student ID's format; on Windows and Android
//! also the LPD client and PJL header that print a PDF without a driver.
//! macOS prints through a CUPS queue instead (`adpt-cups`). Ported from
//! InstitutePrinter's `cross-platform` branch.

mod endpoint;
mod student_id;

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
