//! Shared domain rules and state, independent of UI and platform frameworks.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod backup;
pub mod cloud_sync;
pub mod contracts;
pub mod course_materials;
pub mod data;
pub mod downloads;
pub mod events;
pub mod kyomu_guide;
pub mod kyomu_ingest;
pub mod lifecycle;
pub mod lms_ingest;
pub mod lms_session;
pub mod notes;
pub mod plans;
pub mod ports;
pub mod pull_coordinator;
pub mod reference_refresh;
pub mod runtime;
pub mod runtime_tasks;
pub mod scheduler;
pub mod school_auth;
pub mod school_parser;
pub mod school_refresh;
pub mod school_service;
pub mod subscriptions;
pub mod syllabus;
