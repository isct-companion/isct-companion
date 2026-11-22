//! The one wall clock of Core and the shells.
//!
//! Every reading of the time goes through [`now`]; clippy (`clippy.toml`)
//! rejects any other `Utc::now`, so Core's stamps, its plan and the shells'
//! timers never disagree. A normal build reads the system clock. A `demo`
//! build starts at the demo scene's moment and runs on from there, so every
//! screen shows that day and every timer still fires as usual.

use chrono::{DateTime, TimeDelta, Utc};

/// The current instant.
#[allow(clippy::disallowed_methods)] // The one read of the system clock.
pub fn now() -> DateTime<Utc> {
    Utc::now() + offset()
}

/// How far [`now`] runs ahead of the system clock: zero outside a `demo`
/// build, and always whole minutes, so minute boundaries stay on the system
/// clock's. The desktop injects it into its page as
/// `__ISCT_COMPANION_CLOCK_OFFSET_MS__`.
#[cfg(not(feature = "demo"))]
pub fn offset() -> TimeDelta {
    TimeDelta::zero()
}

#[cfg(feature = "demo")]
#[allow(clippy::disallowed_methods)] // The system minute the scene starts at.
pub fn offset() -> TimeDelta {
    use chrono::DurationRound;
    use std::sync::OnceLock;
    static OFFSET: OnceLock<TimeDelta> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let started = Utc::now();
        let minute = started
            .duration_trunc(TimeDelta::minutes(1))
            .unwrap_or(started);
        crate::demo::scene_start() - minute
    })
}
