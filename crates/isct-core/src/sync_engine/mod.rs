//! One planner and one runner for every network read of school and public
//! data.
//!
//! Shells send [`Hint`]s: lifecycle facts and the user's intents. Core
//! decides which freshness keys are due (`sync_state::due` with each
//! dataset's TTL, missing first), why due work waits ([`Held`]), and leases
//! each key to one reader at a time. Readers run on per-host lanes; every
//! write a reader makes passes the commit gate, which refuses a lease of an
//! old generation, so nothing started before a reset writes after it. The
//! state of all of it is one [`SyncSnapshot`], sent as
//! `CoreEvent::SyncSnapshot` whenever it changes.
mod planner;
mod runner;
mod snapshot;

pub(crate) use planner::Depth;
pub(crate) use runner::{Gated, KeyEnd, Lease};

use crate::{lifecycle::SchoolSite, school_auth::SchoolAuthProfile};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Interactive beats Visible beats Background. A higher tier is leased
/// first but never interrupts a key that is already being read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    Background,
    Visible,
    Interactive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Network {
    Offline,
    Wifi,
    Cellular,
    Unknown,
}

/// Facts and intents from a shell. None of them decides anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Hint {
    /// Periodic shell wake (about once a second); the planner evaluates at
    /// most every 30 s unless another hint arrived.
    Wake,
    Foreground,
    Background,
    Suspend,
    Resume,
    Network {
        network: Network,
    },
    PowerSaver {
        on: bool,
    },
    SignedIn {
        profile: SchoolAuthProfile,
    },
    /// A page that shows `view` became visible; its keys use the open
    /// threshold (15 min) at Visible/Interactive priority.
    Open {
        view: View,
    },
    /// The refresh button of `scope`: ignores TTL and backoff, keeps
    /// single-flight; a repeat within 30 s joins the first.
    Refresh {
        scope: Scope,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum View {
    Today,
    Assignments,
    Timetable,
    /// One LMS course page (lms.course + lms.assign at Interactive).
    Course {
        lms_course_id: u64,
    },
    /// Course search for an academic year (catalog keys of that year).
    Finder {
        year: u16,
    },
    Grades,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Scope {
    Everything,
    Site {
        site: SchoolSite,
    },
    Course {
        lms_course_id: u64,
    },
    /// What the timetable grid is drawn from, the keys of `View::Timetable`.
    Timetable,
    References,
    Catalog {
        year: u16,
    },
}

/// i18n key plus arguments; shells translate, Core never builds sentences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub key: String,
    pub args: serde_json::Value,
}

/// Why due work is waiting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Held {
    Offline,
    /// Background work waits for Wi-Fi; Interactive still runs.
    Cellular,
    PowerSaver,
    /// Shell is in the background and the dataset needs the foreground
    /// (教務: cookie session only).
    Background,
    Backoff {
        until: DateTime<Utc>,
    },
    HostPaused {
        host: String,
        until: DateTime<Utc>,
    },
    SignInRequired {
        profile: SchoolAuthProfile,
    },
    Resetting,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncFailure {
    /// `sync_runs.error_kind` code (`timeout`, `http-503`, …).
    pub kind: String,
    pub keys_failed: u32,
    pub retry_at: Option<DateTime<Utc>>,
}

/// One dataset being worked on now (or held). Shells show `total` as a
/// determinate bar only when it is `Some`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub run: String,
    pub dataset: String, // sync_state::Dataset::name()
    pub label: Label,
    pub priority: Priority,
    pub done: u32,
    pub total: Option<u32>,
    pub current: Option<Label>, // e.g. the course being read
    /// Narrows a run to a page: `lms.course` key, offering id, year.
    pub scope: Option<String>,
    pub held: Option<Held>,
    pub failure: Option<SyncFailure>,
}

/// One source as the status line shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    /// `lms`, `kyomu`, `references`, `catalog`.
    pub source: String,
    pub last_updated: Option<DateTime<Utc>>,
    pub running: bool,
    pub held: Option<Held>,
    pub failure: Option<SyncFailure>,
    /// Sign-in state for school sources (`active`, `signed-out`,
    /// `maintenance`, …) — what SchoolStatus used to carry.
    pub session: Option<String>,
    pub maintenance: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncSnapshot {
    pub generation: u64,
    pub resetting: bool,
    pub runs: Vec<RunView>,
    pub sources: Vec<SourceView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ResetScope {
    SchoolCaches {
        sign_out_web: bool,
        delete_downloads: bool,
    },
    Everything,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResetReport {
    pub removed_rows: u64,
    pub freed_bytes: u64,
}

pub use runner::{AdmissionGuard, SyncEngine, SyncHints, KEY_DEADLINE};

#[cfg(test)]
#[path = "../tests/sync_engine.rs"]
mod tests;
