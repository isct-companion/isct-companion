//! `titech-lpd`, the campus printer queue's CUPS backend. CUPS runs it as
//! root: with no arguments to discover it, and with a job's 5 or 6
//! arguments to print (`adpt_cups::backend`).
// The shared modules carry items only the installer or the library uses.
#![allow(dead_code)]

#[path = "../src/backend.rs"]
mod backend;
#[path = "../../adpt-printer/src/endpoint.rs"]
mod endpoint;
#[path = "../src/queue.rs"]
mod queue;
#[path = "../../adpt-printer/src/student_id.rs"]
mod student_id;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match backend::run(&args) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("ERROR: titech-lpd: {error}");
            std::process::exit(1);
        }
    }
}
