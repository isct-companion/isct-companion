//! The campus printer queue's installer, run once as root by the app's
//! administrator prompt with `<student ID> <login name>`: it installs the
//! embedded `titech-lpd` backend and creates or updates the
//! `InstitutePrinter` queue for that login (`adpt_cups::install`).
// The shared modules carry items only the backend or the library uses.
#![allow(dead_code)]

#[path = "../../adpt-printer/src/endpoint.rs"]
mod endpoint;
#[path = "../src/install.rs"]
mod install;
#[path = "../src/queue.rs"]
mod queue;
#[path = "../../adpt-printer/src/student_id.rs"]
mod student_id;

/// The backend for this target, built just before this program.
const BACKEND: &[u8] = include_bytes!(env!("TITECH_LPD"));

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match install::run(&args, BACKEND) {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
    }
}
