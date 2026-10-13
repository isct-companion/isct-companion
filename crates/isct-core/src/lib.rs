//! Shared domain rules and state, independent of UI and platform frameworks.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod contracts;
pub mod data;
pub mod kyomu_guide;
pub mod kyomu_ingest;
pub mod lifecycle;
pub mod lms_ingest;
pub mod plans;
pub mod ports;
pub mod pull_coordinator;
pub mod school_auth;
pub mod school_parser;
pub mod subscriptions;
pub mod syllabus;
