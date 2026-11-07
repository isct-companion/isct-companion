//! The campus printer, a Kyocera ECOSYS P8060cdn at
//! lpd://172.20.83.212/kyocera, which bills each job to the student ID in
//! the LPD user field. That field is not authenticated: this crate only
//! keeps it well formed, and that the ID is the person's own is Core's
//! decision (`app_core::campus_printer`).
//!
//! This crate holds no state and decides nothing. On every desktop it holds
//! the printer's address and the student ID's format; on Windows also the
//! LPD client and PJL header that print a PDF without a driver. macOS and
//! Linux print through a CUPS queue instead (`adpt-cups`). Ported from
//! InstitutePrinter's `cross-platform` branch.

mod endpoint;
mod student_id;

#[cfg(windows)]
pub mod document;
#[cfg(windows)]
pub mod lpd;
#[cfg(windows)]
pub mod pjl;

pub use endpoint::{ADDRESS, HOST, PORT, QUEUE};
pub use student_id::StudentId;

#[cfg(test)]
#[path = "tests/student_id.rs"]
mod student_id_tests;
