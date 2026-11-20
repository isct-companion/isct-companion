//! The `PlanEngine`: one owner of when Core's background work runs
//! (`approach/core-deadline-plan`).
//!
//! It plans every network read of school and public data: shells send
//! [`Hint`]s (lifecycle facts and the user's intents), and Core decides which
//! freshness keys are due (`sync_state::due` with each dataset's TTL,
//! missing first), why due work waits ([`Held`]), and leases each key to one
//! reader at a time. Readers run on per-host lanes; every write a reader
//! makes passes the commit gate, which refuses a lease of an old generation
//! or of a resource a write moved since, so nothing started before a reset
//! or an LMS write lands after it. One-off reads (an LMS submission's
//! preparation, preflight and verification) are admitted by the same rules
//! and lanes (`ReadRequest`). The other deadline work, the session pauses,
//! reminders, the person's tasks, cloud sync, maintenance, the agent
//! listener and the LMS writes' verification, are [`Duty`]s: each says when
//! it is next due and the engine runs it then.
//!
//! No service keeps a clock of its own. The engine publishes the earliest
//! deadline of all of it as a [`WakeDirective`]; a shell keeps one timer for
//! it and calls `Core::deadline_reached`, and asks its OS for a background
//! opportunity from `WakeDirective::background`. The state of the reads is
//! one [`SyncSnapshot`], sent as `CoreEvent::SyncSnapshot` whenever it
//! changes.
mod duties;
mod planner;
mod runner;
mod school;
mod snapshot;

pub(crate) use duties::Duty;
#[cfg(feature = "agent")]
pub(crate) use planner::{Credential, Lane};
#[cfg(feature = "agent")]
pub(crate) use runner::{ReadRefused, ReadRequest};

pub(crate) use planner::Depth;
pub(crate) use runner::{Gated, KeyEnd, Lease};
pub(crate) use school::{denied_read, net_end, session_end, SchoolReaders};

use crate::school_session::SignInPresentation;
use crate::{
    lifecycle::SchoolSite, school_auth::SchoolAuthProfile, school_session::SchoolSessionState,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Interactive beats Visible beats Background. A higher tier is leased
/// first but never interrupts a key that is already being read.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncPriority"))]
#[serde(rename_all = "kebab-case")]
pub enum Priority {
    Background,
    Visible,
    Interactive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncNetwork"))]
#[serde(rename_all = "kebab-case")]
pub enum Network {
    Offline,
    Wifi,
    Cellular,
    Unknown,
}

/// Facts and intents from a shell. None of them decides anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncHint"))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Hint {
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncView"))]
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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        lms_course_id: u64,
    },
    /// Course search for an academic year (catalog keys of that year).
    Finder {
        year: u16,
    },
    Grades,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncScope"))]
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
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        lms_course_id: u64,
    },
    /// What the timetable grid is drawn from, the keys of `View::Timetable`.
    Timetable,
    References,
    Catalog {
        year: u16,
    },
    LmsHistory {
        year: u16,
        month: u8,
    },
}

/// Refresh scopes that cover the failed keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RetryPlan {
    pub scopes: Vec<Scope>,
}

/// i18n key plus arguments; shells translate, Core never builds sentences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncLabel"))]
pub struct Label {
    pub key: String,
    #[cfg_attr(feature = "ts", ts(type = "Record<string, unknown> | null"))]
    pub args: serde_json::Value,
}

/// Why due work is waiting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename = "SyncHeld"))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Held {
    Offline,
    /// Background work waits for Wi-Fi; Interactive still runs.
    Cellular,
    PowerSaver,
    /// The shell suspended the app.
    Background,
    Backoff {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        until: DateTime<Utc>,
    },
    HostPaused {
        host: String,
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        until: DateTime<Utc>,
    },
    /// No credential on the key's path can serve it and none can be got
    /// now (`planner::hold`): it waits for what the account's admission or
    /// sign-in says lets it go on. Keys whose path still has a usable token
    /// or session go on. Not a failure: no backoff.
    Account {
        profile: SchoolAuthProfile,
        wait: AccountWait,
    },
    Resetting,
    /// Reads once the data it lands on is stored (`planner::prerequisite`):
    /// a course after the enrolment list, its assignments after the course.
    /// Not a failure.
    Prerequisite,
}

/// What lets an account's waiting reads go on. Waiting for the network is
/// `Held::Offline`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AccountWait {
    /// The person: no credentials, the school refused them or wants its own
    /// page, or a sign-out is pending. Until the credentials change, or the
    /// person's Retry or Connect.
    Person,
    /// A pause after a sign-in without a verdict, until `until`.
    Paused {
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        until: DateTime<Utc>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SyncFailure {
    /// `sync_runs.error_kind` code (`timeout`, `http-503`, …).
    pub kind: String,
    pub reason: Label,
    pub keys_failed: u32,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub retry_at: Option<DateTime<Utc>>,
    /// Which pages of a run failed and why, one `page: reason` line each.
    /// Diagnostic text a person can quote in a report, not translated; only
    /// the runs of this launch hold it, `kind` is what is stored.
    pub detail: Option<String>,
}

/// One dataset being worked on now (or held). Shells show `total` as a
/// determinate bar only when it is `Some`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    pub retry: Option<RetryPlan>,
}

/// One source as the status line shows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    /// `lms`, `kyomu`, `references`, `catalog`.
    pub source: String,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub last_updated: Option<DateTime<Utc>>,
    pub running: bool,
    pub held: Option<Held>,
    pub failure: Option<SyncFailure>,
    pub retry: Option<RetryPlan>,
    /// The account's session state for school sources.
    pub session: Option<SchoolSessionState>,
    /// The school's maintenance notice for the source's site.
    pub maintenance: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SyncWork {
    Resetting,
    Active { run: String },
    Held { run: String },
    Problem,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum SyncAttentionProblem {
    Account {
        profile: SchoolAuthProfile,
        presentation: Option<SignInPresentation>,
        #[cfg_attr(feature = "ts", ts(type = "string | null"))]
        retry_at: Option<DateTime<Utc>>,
    },
    RunFailed {
        dataset: String,
        failure: SyncFailure,
        retry: Option<RetryPlan>,
    },
    SourceFailed {
        source: String,
        failure: SyncFailure,
        retry: Option<RetryPlan>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SyncAttention {
    pub primary: Option<SyncWork>,
    pub problem: Option<SyncAttentionProblem>,
    pub busy: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SyncSnapshot {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub generation: u64,
    pub resetting: bool,
    pub attention: SyncAttention,
    pub runs: Vec<RunView>,
    pub sources: Vec<SourceView>,
}

#[cfg(feature = "ts")]
pub fn declarations(cfg: &ts_rs::Config) -> Vec<String> {
    use ts_rs::TS;
    vec![
        SchoolSite::decl(cfg),
        Priority::decl(cfg),
        Network::decl(cfg),
        Hint::decl(cfg),
        View::decl(cfg),
        Scope::decl(cfg),
        Label::decl(cfg),
        Held::decl(cfg),
        RetryPlan::decl(cfg),
        AccountWait::decl(cfg),
        SyncFailure::decl(cfg),
        RunView::decl(cfg),
        SourceView::decl(cfg),
        SyncWork::decl(cfg),
        SyncAttentionProblem::decl(cfg),
        SyncAttention::decl(cfg),
        SyncSnapshot::decl(cfg),
    ]
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

/// When Core next has work: a shell keeps one timer for `at` and calls
/// `Core::deadline_reached` with this revision when it fires, and asks its
/// OS for a background opportunity from `background`. Read
/// `Core::wake_directive` on start and after a missed `wake-directive`
/// event; a revision lower than the one held is older.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WakeDirective {
    pub revision: u64,
    /// The earliest deadline, possibly already past. `None`: nothing waits
    /// on time; an event or the person moves Core next.
    pub at: Option<DateTime<Utc>>,
    /// What an OS background opportunity would find due, by kind.
    pub background: Vec<BackgroundRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundRequest {
    pub work: BackgroundWork,
    /// The earliest the work is due. An OS window before it finds nothing;
    /// the OS decides how late it comes.
    pub earliest: DateTime<Utc>,
}

/// What a background opportunity needs from the OS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackgroundWork {
    /// School and reference reads and cloud sync.
    Network,
    /// Reminders, the person's tasks and maintenance.
    Local,
}

/// Why a background opportunity ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpportunityEnd {
    /// Core went quiet or its budget ran out.
    Finished,
    /// The OS took the time back early.
    Expired,
}

pub use runner::{AdmissionGuard, PlanEngine, PlanHints, KEY_DEADLINE};

#[cfg(test)]
#[path = "../tests/plan.rs"]
mod tests;
