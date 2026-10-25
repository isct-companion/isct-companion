//! What a committed write changed, in the terms shells refresh by.
//!
//! Every commit reports the tables it touched (`data::CommitObserver`). This
//! module is the one table that turns them into the change names a
//! `workspace-changed` event carries, and each name into the snapshot fields
//! a shell re-reads (`Core::workspace_records`). It also coalesces bursts: a
//! sync commits many keys within seconds, and a shell should hear them as a
//! few events, not dozens.
use crate::data::WorkspaceChangeSource;
use crate::workspace::WorkspaceSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// A group of snapshot records that changed together. Shells re-read the
/// records and re-run the views that depend on a name; names are stable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceChange {
    /// Timetable courses and memberships, the LMS course list, and the
    /// person's own courses and course edits.
    Courses,
    /// LMS course contents.
    Materials,
    /// Assignments and the person's marks on them.
    Assignments,
    /// 教務 registrations and their states.
    Registrations,
    /// Grades, GPA and graduation-requirement progress.
    Grades,
    /// The course catalog the finder searches, with departments and
    /// recommended courses.
    Finder,
    /// The academic calendar, important dates, class schedules and the
    /// person's calendar exceptions, schedule overrides and intensive courses.
    Calendar,
    Exams,
    /// Plans and plan items.
    Plans,
    /// Personal events.
    Events,
    Subscriptions,
    ScheduledTasks,
    /// Settings rows. Language, identity and profile settings shape every
    /// projected field, so this name re-reads the whole workspace.
    Settings,
    /// When each sync key was last read, and 教務 page availability.
    Freshness,
}

use WorkspaceChange::*;

impl WorkspaceChange {
    pub const ALL: [Self; 14] = [
        Courses,
        Materials,
        Assignments,
        Registrations,
        Grades,
        Finder,
        Calendar,
        Exams,
        Plans,
        Events,
        Subscriptions,
        ScheduledTasks,
        Settings,
        Freshness,
    ];

    /// The stable name shells receive, e.g. `scheduled-tasks`.
    pub fn name(self) -> &'static str {
        match self {
            Courses => "courses",
            Materials => "materials",
            Assignments => "assignments",
            Registrations => "registrations",
            Grades => "grades",
            Finder => "finder",
            Calendar => "calendar",
            Exams => "exams",
            Plans => "plans",
            Events => "events",
            Subscriptions => "subscriptions",
            ScheduledTasks => "scheduled-tasks",
            Settings => "settings",
            Freshness => "freshness",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|change| change.name() == name)
    }

    /// The workspace fields and school-record fields this change can alter,
    /// or `None` for every workspace field.
    fn fields(self) -> Option<Fields> {
        fn fields(
            projected: bool,
            workspace: &'static [&'static str],
            records: &'static [&'static str],
        ) -> Option<Fields> {
            Some(Fields {
                projected,
                workspace,
                records,
            })
        }
        match self {
            Courses => fields(true, &["lmsCourses"], &["lmsCourses"]),
            Registrations => fields(
                true,
                &["registrations"],
                &["registrations", "registrationStates"],
            ),
            Finder => fields(true, &["departments", "recommendedCourses"], &[]),
            // No calendar row feeds the course projection; quarter-start
            // days decide the current term.
            Calendar => fields(
                false,
                &[
                    "currentTerm",
                    "academicCalendar",
                    "importantDates",
                    "exceptions",
                    "classSchedules",
                    "scheduleOverrides",
                    "intensiveSchedules",
                ],
                &[],
            ),
            Materials => fields(false, &["materials"], &[]),
            Assignments => fields(false, &["assignments"], &[]),
            Grades => fields(
                false,
                &["gpaSummary", "grades"],
                &["grades", "gpaByTerm", "requirements"],
            ),
            Exams => fields(false, &["exams"], &[]),
            Plans => fields(false, &["plans", "planItems"], &[]),
            Events => fields(false, &["events"], &[]),
            Subscriptions => fields(false, &["subscriptions"], &[]),
            ScheduledTasks => fields(false, &["scheduledTasks"], &[]),
            Freshness => fields(false, &[], &["freshness", "pageAvailability"]),
            Settings => None,
        }
    }
}

struct Fields {
    /// The course projection merges LMS, 教務, catalog and calendar rows into
    /// courses and rewrites course ids in the rows that point at them, so a
    /// change that feeds it re-reads every projected field.
    projected: bool,
    workspace: &'static [&'static str],
    records: &'static [&'static str],
}

const PROJECTED: &[&str] = &[
    "courses",
    "memberships",
    "assignments",
    "materials",
    "planItems",
    "terms",
    "currentTerm",
];

/// Which database file a table lives in. Only user-owned rows reach cloud
/// sync, so only their commits bring a capture forward.
#[derive(Clone, Copy, PartialEq)]
enum Owner {
    User,
    Cache,
    /// sync.db: the sync engine's bookkeeping about cache rows.
    Sync,
}

/// Every table a commit can report, with the changes it feeds. A table not
/// listed here is reported as every change and as user-owned, so a new
/// table is over-refreshed rather than missed; the test that lists every
/// database file fails until it is added.
const TABLES: &[(&str, &[WorkspaceChange], Owner)] = &[
    ("timetable_courses", &[Courses], Owner::Cache),
    ("memberships", &[Courses], Owner::Cache),
    ("lms_courses", &[Courses], Owner::Cache),
    ("manual_courses", &[Courses], Owner::User),
    ("manual_memberships", &[Courses], Owner::User),
    ("course_overrides", &[Courses], Owner::User),
    ("course_identity_map", &[Courses], Owner::User),
    ("lms_items", &[Materials], Owner::Cache),
    ("assignments", &[Assignments], Owner::Cache),
    ("assignment_marks", &[Assignments], Owner::User),
    ("registrations", &[Registrations], Owner::Cache),
    ("registration_states", &[Registrations], Owner::Cache),
    ("grades", &[Grades], Owner::Cache),
    ("gpa_by_term", &[Grades], Owner::Cache),
    ("gpa_summary", &[Grades], Owner::Cache),
    ("requirement_progress", &[Grades], Owner::Cache),
    ("offerings", &[Finder], Owner::Cache),
    ("offerings_fts", &[Finder], Owner::Cache),
    ("departments", &[Finder], Owner::Cache),
    ("recommended_courses", &[Finder], Owner::Cache),
    ("academic_calendar", &[Calendar], Owner::Cache),
    ("important_dates", &[Calendar], Owner::Cache),
    ("class_schedules", &[Calendar], Owner::Cache),
    ("calendar_exceptions", &[Calendar], Owner::User),
    ("schedule_overrides", &[Calendar], Owner::User),
    ("intensive_schedules", &[Calendar], Owner::User),
    ("exams", &[Exams], Owner::Cache),
    ("plans", &[Plans], Owner::User),
    ("plan_items", &[Plans], Owner::User),
    ("personal_events", &[Events], Owner::User),
    ("subscriptions", &[Subscriptions], Owner::User),
    ("scheduled_tasks", &[ScheduledTasks], Owner::User),
    ("settings", &[Settings], Owner::User),
    ("dataset_state", &[Freshness], Owner::Sync),
    // Run records are diagnostics; no view shows them.
    ("sync_runs", &[], Owner::Sync),
];

/// FTS5 keeps an index in shadow tables (`offerings_fts_data`, `_idx`,
/// `_content`, `_docsize`, `_config`); the update hook reports those.
fn entry(table: &str) -> Option<&'static (&'static str, &'static [WorkspaceChange], Owner)> {
    TABLES
        .iter()
        .find(|(name, _, _)| *name == table)
        .or_else(|| {
            let base = ["_data", "_idx", "_content", "_docsize", "_config"]
                .iter()
                .find_map(|suffix| table.strip_suffix(suffix))?;
            TABLES
                .iter()
                .find(|(name, _, _)| *name == base && name.ends_with("_fts"))
        })
}

/// The change names a commit to `tables` produces, sorted.
pub fn changes_for(tables: &BTreeSet<String>) -> BTreeSet<WorkspaceChange> {
    let mut changes = BTreeSet::new();
    for table in tables {
        match entry(table) {
            Some((_, feeds, _)) => changes.extend(feeds.iter().copied()),
            None => changes.extend(WorkspaceChange::ALL),
        }
    }
    changes
}

/// Whether a commit to `tables` changed rows cloud sync captures.
pub(crate) fn touches_user_data(tables: &BTreeSet<String>) -> bool {
    tables
        .iter()
        .any(|table| entry(table).is_none_or(|(_, _, owner)| *owner == Owner::User))
}

/// The snapshot fields that `changes` feed, from one coherent snapshot. A
/// shell merges `workspace` and `schoolRecords` over its copy, field by field.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePatch {
    /// As in `WorkspaceSnapshot`, for the whole snapshot this was read from.
    pub revision: String,
    pub content_revision: String,
    /// Only the workspace fields the requested changes feed.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub workspace: Value,
    /// Only the school-record fields the requested changes feed.
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub school_records: Value,
}

pub(crate) fn patch(snapshot: WorkspaceSnapshot, changes: &[WorkspaceChange]) -> WorkspacePatch {
    let mut workspace_fields = BTreeSet::new();
    let mut record_fields = BTreeSet::new();
    let mut whole_workspace = false;
    for change in changes {
        match change.fields() {
            Some(fields) => {
                if fields.projected {
                    workspace_fields.extend(PROJECTED.iter().copied());
                }
                workspace_fields.extend(fields.workspace.iter().copied());
                record_fields.extend(fields.records.iter().copied());
            }
            None => whole_workspace = true,
        }
    }
    let pick = |value: &Value, all: bool, fields: &BTreeSet<&str>| -> Value {
        match value {
            Value::Object(map) if !all => Value::Object(
                map.iter()
                    .filter(|(key, _)| fields.contains(key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Map<_, _>>(),
            ),
            value => value.clone(),
        }
    };
    WorkspacePatch {
        workspace: pick(&snapshot.workspace, whole_workspace, &workspace_fields),
        school_records: pick(&snapshot.school_records, false, &record_fields),
        revision: snapshot.revision,
        content_revision: snapshot.content_revision,
    }
}

/// How long a shell waits at most to hear a background change, and how
/// often it hears one at most.
pub(crate) const COALESCE_WINDOW: Duration = Duration::from_millis(250);

type Deliver<K, V> = dyn Fn(K, Vec<V>) + Send + Sync;

/// Delivers background changes at most once per window and source. The
/// first change after a quiet window goes out at once; later ones in the
/// window are merged and go out when it ends. User intents do not pass
/// here: the shell that made one waits on it. Catalog changes pass a
/// coalescer of their own, keyed by year.
pub(crate) struct Coalescer<K = WorkspaceChangeSource, V = WorkspaceChange> {
    window: Duration,
    state: Mutex<Window<K, V>>,
    deliver: Box<Deliver<K, V>>,
}

struct Window<K, V> {
    /// Changes before this instant wait for the trailing delivery.
    open_until: Option<Instant>,
    /// Merged per source, in the order sources first appeared.
    pending: Vec<(K, BTreeSet<V>)>,
    /// A thread owns the trailing delivery.
    flushing: bool,
}

impl<K, V> Default for Window<K, V> {
    fn default() -> Self {
        Self {
            open_until: None,
            pending: Vec::new(),
            flushing: false,
        }
    }
}

impl<K, V> Coalescer<K, V>
where
    K: Eq + Send + 'static,
    V: Ord + Send + 'static,
{
    pub(crate) fn new(
        window: Duration,
        deliver: impl Fn(K, Vec<V>) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            window,
            state: Mutex::default(),
            deliver: Box::new(deliver),
        })
    }

    pub(crate) fn push(self: &Arc<Self>, source: K, changes: BTreeSet<V>) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.open_until.is_none_or(|until| until <= now) {
            state.open_until = Some(now + self.window);
            drop(state);
            (self.deliver)(source, changes.into_iter().collect());
            return;
        }
        match state
            .pending
            .iter_mut()
            .find(|(pending, _)| *pending == source)
        {
            Some((_, merged)) => merged.extend(changes),
            None => state.pending.push((source, changes)),
        }
        if state.flushing {
            return;
        }
        state.flushing = true;
        drop(state);
        // Commits arrive on whatever thread wrote, with or without an async
        // runtime, so the trailing delivery gets its own thread. It ends one
        // quiet window after its last delivery; after `Core::shutdown` its
        // deliveries reach a cleared sink and are dropped.
        let this = self.clone();
        let spawned = std::thread::Builder::new()
            .name("coalesced-changes".into())
            .spawn(move || this.flush());
        if spawned.is_err() {
            // No thread to wait with: deliver now rather than never.
            self.flush_now();
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .flushing = false;
        }
    }

    fn flush(&self) {
        loop {
            let until = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .open_until;
            if let Some(until) = until {
                std::thread::sleep(until.saturating_duration_since(Instant::now()));
            }
            if !self.flush_now() {
                return;
            }
        }
    }

    /// Delivers what is pending and opens a new window; false when nothing
    /// was pending, which ends the trailing thread.
    fn flush_now(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = std::mem::take(&mut state.pending);
        if pending.is_empty() {
            state.flushing = false;
            return false;
        }
        state.open_until = Some(Instant::now() + self.window);
        drop(state);
        for (source, changes) in pending {
            (self.deliver)(source, changes.into_iter().collect());
        }
        true
    }
}

#[cfg(test)]
#[path = "tests/workspace_changes.rs"]
mod tests;
