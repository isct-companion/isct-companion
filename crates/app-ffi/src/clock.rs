//! Core's one wall clock (`app_core::clock`), for the shells' own readings.

/// How far Core's clock runs ahead of the system clock, in milliseconds: zero
/// outside the demo app, whose clock starts at the demo scene's moment. The
/// shells add it to every reading of the time they compare with Core's or
/// show.
#[uniffi::export]
pub fn clock_offset_ms() -> i64 {
    app_core::clock::offset().num_milliseconds()
}
