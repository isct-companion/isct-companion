//! Shared domain rules and state, independent of UI and platform frameworks.
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod contracts;
pub mod kyomu_guide;
pub mod lifecycle;
pub mod plans;
pub mod pull_coordinator;
