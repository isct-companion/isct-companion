//! The campus printer, a Kyocera ECOSYS P8060cdn at
//! lpd://172.20.83.212/kyocera, which bills each job to the student ID in
//! the LPD user field. That field is not authenticated: this crate only
//! keeps it well formed, and that the ID is the person's own is Core's
//! decision (`app_core::campus_printer`).
//!
//! This crate holds no state and decides nothing. It holds the protocols:
//! the student ID's format, an LPD client and the PJL header for printing a
//! PDF without a driver (Windows), and the CUPS queue with its root helper
//! (macOS and Linux). Ported from InstitutePrinter's `cross-platform` and
//! `master` branches.

mod endpoint;
mod student_id;

pub mod document;
pub mod lpd;
pub mod pjl;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod cups;

pub use endpoint::{ADDRESS, HOST, PORT, QUEUE};
pub use student_id::StudentId;

#[cfg(test)]
#[path = "tests/student_id.rs"]
mod student_id_tests;
