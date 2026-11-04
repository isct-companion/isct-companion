//! The `InstitutePrinter` CUPS queue, so every application prints to the
//! campus printer from its own print dialog with all of the PPD's options.
//!
//! One root step installs it: the helper (`helper.rs`, embedded here as
//! `HELPER`) copies itself into the CUPS backend folder as `titech-lpd`
//! (root, mode 0700), and recreates the queue from Kyocera's unmodified PPD
//! with the student ID in its device URI. The URI lives in CUPS's
//! root-owned configuration, so only an administrator can change the ID a
//! queue bills. Installing again overwrites the same queue; removing it is
//! the system printer settings' business.

pub mod queue;

/// The helper executable for this target, built by `build.rs`. Run as root
/// with `install <student ID>`.
pub const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/institute-printer-helper"));

/// The argument the helper's install mode takes before the student ID.
pub const INSTALL: &str = "install";

// Beside `queue`, not inside it: the helper includes `queue.rs` too.
#[cfg(test)]
#[path = "../tests/cups__queue.rs"]
mod tests;
