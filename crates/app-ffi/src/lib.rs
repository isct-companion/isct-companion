//! Native application boundary. Business decisions remain in app-core.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]
uniffi::setup_scaffolding!();
#[cfg(target_os = "android")]
mod android_tls;
mod diagnostics;
mod error;
mod json;
pub mod types {
    include!(concat!(env!("OUT_DIR"), "/contracts.rs"));
}
pub use diagnostics::export_application_logs;
pub use error::SdkError;
pub use json::JsonValue;

mod events;
mod matrix;
mod operation;
pub mod ports;
mod reset;
mod session;
pub use events::{Event, StreamUpdate, Subscription};
pub use matrix::{
    MatrixCameraFrame, MatrixCameraPhase, MatrixCameraSession, MatrixCameraState, MatrixCell,
    MatrixPoint,
};
pub use operation::{Operation, OperationResult, OperationStatus, ScheduleSnapshot};
pub use session::{decode_sync_snapshot, Capabilities, Session, SessionConfig};
mod services;

#[cfg(test)]
#[path = "tests/crate.rs"]
mod tests;
