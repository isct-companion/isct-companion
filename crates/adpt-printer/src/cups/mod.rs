//! The `InstitutePrinter` CUPS queue, so every application prints to the
//! campus printer from its own print dialog with all of the PPD's options.
//!
//! One root step installs it: the helper (`helper.rs`, embedded here as
//! `HELPER`) copies itself into the CUPS backend folder as `titech-lpd`
//! (root, mode 0700), and creates or updates the queue from Kyocera's
//! unmodified PPD with the student ID in its device URI. The URI lives in
//! CUPS's root-owned configuration, so only an administrator can change the
//! ID a queue bills, and only the installing login may print to it.
//! Installing again updates the same queue; removing it is the system
//! printer settings' business.

pub mod queue;

/// The helper executable for this target, built by `build.rs`. Run as root
/// with `install <student ID> <login name>`.
pub const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/institute-printer-helper"));

pub use queue::INSTALL;

// Beside `queue`, not inside it: the helper includes `queue.rs` too.
#[cfg(test)]
#[path = "../tests/cups__queue.rs"]
mod tests;
