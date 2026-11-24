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
// The demo scene runs only in the demo SDK: Core's `demo` feature must come
// through this crate's own, which the iOS demo build checks against its
// configuration.
const _: () = assert!(
    app_core::DEMO == cfg!(feature = "demo"),
    "app-core's `demo` feature must match app-ffi's"
);
#[cfg(target_os = "android")]
mod android_tls;
mod campus_print;
mod clock;
mod diagnostics;
mod error;
mod exam_share;
mod hand_in;
mod json;
pub mod types {
    include!(concat!(env!("OUT_DIR"), "/contracts.rs"));
}
pub use campus_print::{
    campus_printer_capabilities, CampusPrintColor, CampusPrintJob, CampusPrintMedia,
    CampusPrintMessage, CampusPrintReport, CampusPrintSides, CampusPrinterCapabilities,
    CampusPrinterState,
};
pub use clock::clock_offset_ms;
pub use diagnostics::{export_application_logs, feedback_form_url};
pub use error::SdkError;
pub use json::JsonValue;

mod events;
mod matrix;
mod operation;
pub mod ports;
mod reminders;
pub use reminders::NativeReminderPort;
mod reset;
mod session;
pub use events::{Event, StreamUpdate, Subscription};
pub use matrix::{
    MatrixCameraFrame, MatrixCameraPhase, MatrixCameraSession, MatrixCameraState, MatrixCell,
    MatrixConfidence, MatrixPoint,
};
pub use operation::{Operation, OperationResult, OperationStatus, ScheduleSnapshot};
pub use session::{
    decode_school_session, decode_school_web_page, decode_sync_snapshot, Capabilities, Session,
    SessionConfig,
};
mod services;

#[cfg(test)]
#[path = "tests/crate.rs"]
mod tests;

mod widgets;
pub use widgets::{NativeWidgets, WidgetRoute};
