//! The one wall clock of Core and the shells.
//!
//! Every reading of the time goes through [`now`]; clippy (`clippy.toml`)
//! rejects any other `Utc::now`, so Core's stamps, its plan and the shells'
//! timers never disagree. A normal build reads the system clock. A `demo`
//! build starts at the demo scene's moment and runs on from there, so every
//! screen shows that day and every timer still fires as usual.

use chrono::{DateTime, TimeDelta, Utc};

/// The current instant.
#[cfg(not(feature = "demo"))]
#[allow(clippy::disallowed_methods)] // The one read of the system clock.
pub fn now() -> DateTime<Utc> {
    Utc::now()
}

/// The current instant on the demo scene's clock.
#[cfg(feature = "demo")]
#[allow(clippy::disallowed_methods)] // The one read of the system clock.
pub fn now() -> DateTime<Utc> {
    Utc::now() + offset()
}

/// How far [`now`] runs ahead of the system clock: zero outside a `demo`
/// build. The desktop demo injects it into its page as
/// `__ISCT_COMPANION_CLOCK_OFFSET_MS__`; iOS reads it through the FFI.
#[cfg(not(feature = "demo"))]
pub fn offset() -> TimeDelta {
    TimeDelta::zero()
}

/// Set on first use, so the scene starts at exactly its moment (10:20:00)
/// and runs on from there.
#[cfg(feature = "demo")]
#[allow(clippy::disallowed_methods)] // The instant the scene starts at.
pub fn offset() -> TimeDelta {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<TimeDelta> = OnceLock::new();
    *OFFSET.get_or_init(|| crate::demo::scene_start() - Utc::now())
}
