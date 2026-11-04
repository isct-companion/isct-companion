//! The `InstitutePrinter` CUPS queue (macOS and Linux), so every
//! application prints to the campus printer from its own print dialog with
//! all of the PPD's options. Ported from InstitutePrinter's `master` branch.
//!
//! Two root programs, each with one job, built by `build.rs`:
//! - the installer (`install`, embedded here as `INSTALLER`), run once
//!   behind the system's administrator prompt: it installs the backend
//!   (root, mode 0700) and creates or updates the queue from Kyocera's
//!   unmodified PPD, with the student ID in the queue's root-owned device
//!   URI, for the installing login only;
//! - `titech-lpd`, the backend (`backend`), run by CUPS for each job: it
//!   reads the student ID from the device URI and hands the upload to the
//!   system lpd backend with that ID as the LPD user.
//!
//! Installing again updates the same queue; removing it is the system
//! printer settings' business. On other systems this crate is empty.
#![cfg(any(target_os = "macos", target_os = "linux"))]

pub mod backend;
pub mod install;
pub mod queue;

/// What the programs include by path from `adpt-printer`, as the library
/// sees it.
mod endpoint {
    pub use adpt_printer::{HOST, QUEUE};
}
mod student_id {
    pub use adpt_printer::StudentId;
}

/// The installer for this target. Run as root with
/// `<student ID> <login name>`.
pub const INSTALLER: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/institute-printer-installer"));

#[cfg(test)]
#[path = "tests/queue.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "tests/programs.rs"]
mod program_tests;
