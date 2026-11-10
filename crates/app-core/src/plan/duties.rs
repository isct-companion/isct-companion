//! Deadline work besides school reads (`approach/core-deadline-plan`).
//!
//! A domain service keeps its own rules and state and answers one question,
//! [`Duty::next_due`]; the engine runs [`Duty::run`] once that time has come,
//! one run of a duty at a time, owned by the engine's tasks so shutdown
//! drains it. A service whose inputs changed brings its own `next_due`
//! forward and calls `PlanEngine::duties_changed`; none keeps a timer.
use super::BackgroundWork;
use crate::net::BoxFuture;
use chrono::{DateTime, Utc};
use std::sync::Arc;

pub(crate) trait Duty: Send + Sync + 'static {
    /// For logs.
    fn name(&self) -> &'static str;
    /// What an OS background opportunity must allow for it to run.
    fn work(&self) -> BackgroundWork;
    /// When it next wants to run; `None` while only an event can make it
    /// due. Called after every engine turn, so it reads memory only.
    fn next_due(&self) -> Option<DateTime<Utc>>;
    /// Runs what is due and notes the next time. Errors are its own to
    /// report.
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()>;
    /// The OS took the background time back while it runs: stop between
    /// steps, keeping what is left for its next run.
    fn interrupt(&self) {}
}

/// One duty as the engine holds it.
pub(super) struct Slot {
    pub(super) duty: Arc<dyn Duty>,
    pub(super) running: bool,
}

impl Slot {
    pub(super) fn new(duty: Arc<dyn Duty>) -> Self {
        Self {
            duty,
            running: false,
        }
    }
}
