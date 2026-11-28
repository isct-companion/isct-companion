#[cfg(test)]
use super::KEY_DEADLINE;
use super::{guard, AccountWait, Dataset, Depth, Key, KeyState};
use crate::{
    data::{DataError, DataStore, WorkspaceChangeSource},
    net::Host,
    school_auth::SchoolAuthProfile,
    sync_state::{self, RunOutcome, RunRecord},
};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{watch, Notify};
use tokio_util::sync::CancellationToken;

// -- Commit gate --------------------------------------------------------------

#[derive(Default)]
struct GateState {
    generation: u64,
    committing: usize,
    /// Moves when an LMS write starts on a key's resource, or when the person
    /// leaves the in-app 教務 page; a lease taken before it never commits.
    resources: HashMap<Key, u64>,
}

/// Every write of a lease passes here. Closing admission bumps the
/// generation under the same lock, so a commit either entered before the
/// bump (and `wait_idle` waits for it) or is refused; likewise a write that
/// started on the key's resource after the lease was taken.
pub(super) struct Gate {
    state: Mutex<GateState>,
    /// The commits in progress, for `wait_idle`.
    pub(super) committing: watch::Sender<usize>,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            committing: watch::channel(0).0,
        }
    }
}

impl Gate {
    pub(super) fn generation(&self) -> u64 {
        guard(&self.state).generation
    }

    pub(super) fn bump(&self) -> u64 {
        let mut state = guard(&self.state);
        state.generation = state.generation.saturating_add(1);
        state.generation
    }

    pub(super) fn resource(&self, key: &Key) -> u64 {
        guard(&self.state).resources.get(key).copied().unwrap_or(0)
    }

    /// Moves every key's resource under one lock, so no commit lands
    /// between them; returns their new revisions in order.
    pub(super) fn bump_resources(&self, keys: &[Key]) -> Vec<u64> {
        let mut state = guard(&self.state);
        keys.iter()
            .map(|key| {
                let revision = state.resources.entry(key.clone()).or_default();
                *revision = revision.saturating_add(1);
                *revision
            })
            .collect()
    }

    fn enter(self: &Arc<Self>, generation: u64, resources: &[(Key, u64)]) -> Option<GateGuard> {
        let mut state = guard(&self.state);
        if state.generation != generation {
            return None;
        }
        if resources.iter().any(|(key, revision)| {
            state
                .resources
                .get(key)
                .is_some_and(|current| current > revision)
        }) {
            return None;
        }
        state.committing += 1;
        self.committing.send_replace(state.committing);
        Some(GateGuard(self.clone()))
    }
}

/// Holds the gate for one commit; leaving it may let a reset proceed.
pub(crate) struct GateGuard(Arc<Gate>);

impl Drop for GateGuard {
    fn drop(&mut self) {
        let mut state = guard(&self.0.state);
        state.committing = state.committing.saturating_sub(1);
        self.0.committing.send_replace(state.committing);
    }
}

/// Why a gated write did not happen.
#[derive(Debug)]
pub(crate) enum Gated<E> {
    /// The lease belongs to an earlier generation (a reset ran).
    Stale,
    Failed(E),
}

/// Writes for one lease. Cloneable into blocking workers.
#[derive(Clone)]
pub(crate) struct LeaseWriter {
    pub(super) store: DataStore,
    pub(super) gate: Arc<Gate>,
    pub(super) generation: u64,
    /// The resource revision, when it was leased, of each key the read
    /// writes: its own and the one it reads beside it
    /// (`planner::read_beside`).
    pub(super) resources: Vec<(Key, u64)>,
}

impl LeaseWriter {
    pub(crate) fn store(&self) -> &DataStore {
        &self.store
    }

    /// Holds the gate while the caller writes through store methods that
    /// open their own transaction. `None`: the lease is stale.
    pub(crate) fn enter(&self) -> Option<GateGuard> {
        self.gate.enter(self.generation, &self.resources)
    }

    /// One transaction inside the gate.
    pub(crate) fn write<T, E: From<DataError>>(
        &self,
        source: Option<WorkspaceChangeSource>,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, Gated<E>> {
        let Some(_guard) = self.enter() else {
            return Err(Gated::Stale);
        };
        self.store.write(source, work).map_err(Gated::Failed)
    }

    /// One transaction to `year`'s catalog inside the gate
    /// (`DataStore::catalog_write`).
    pub(crate) fn catalog_write<T, E: From<DataError>>(
        &self,
        year: u16,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, Gated<E>> {
        let Some(_guard) = self.enter() else {
            return Err(Gated::Stale);
        };
        self.store.catalog_write(year, work).map_err(Gated::Failed)
    }
}

// -- Leases -------------------------------------------------------------------

/// The right to read one key, with what its reader needs.
pub(crate) struct Lease {
    pub(crate) key: Key,
    /// Correlates the `sync_runs` rows of one run in the snapshot.
    pub(crate) run: String,
    /// The Isct-Data index of a reference key.
    pub(crate) locator: Option<String>,
    pub(crate) depth: Depth,
    pub(crate) started: DateTime<Utc>,
    pub(crate) cancel: CancellationToken,
    pub(super) writer: LeaseWriter,
    pub(super) control: Arc<LeaseControl>,
}

/// What the engine and a running read share beyond cancellation: a
/// deadline that a pause does not use up, and the request to give the lane
/// to a read that needs it more (`d/3mjnxv`).
pub(crate) struct LeaseControl {
    clock: Mutex<Clock>,
    /// The clock paused or resumed.
    moved: Notify,
    /// The engine wants the lane back at the next safe point.
    pub(super) asked: AtomicBool,
    /// The engine gave the lane back.
    pub(super) resumed: Notify,
    /// Who to tell the read paused.
    pub(super) engine: super::PlanHints,
    /// The key read beside the lease's own committed with it.
    beside: AtomicBool,
}

/// A read's time budget: running towards a deadline, or stopped with what
/// was left while the read is paused.
#[derive(Clone, Copy)]
enum Clock {
    Running(tokio::time::Instant),
    Paused(std::time::Duration),
}

impl LeaseControl {
    pub(super) fn new(deadline: tokio::time::Instant, engine: super::PlanHints) -> Self {
        Self {
            clock: Mutex::new(Clock::Running(deadline)),
            moved: Notify::new(),
            asked: AtomicBool::new(false),
            resumed: Notify::new(),
            engine,
            beside: AtomicBool::new(false),
        }
    }

    /// When the read must have ended if it runs on from now.
    pub(super) fn deadline(&self) -> tokio::time::Instant {
        match *guard(&self.clock) {
            Clock::Running(deadline) => deadline,
            Clock::Paused(left) => tokio::time::Instant::now() + left,
        }
    }

    /// Waits until the running clock passes the deadline; a paused clock
    /// never expires.
    pub(super) async fn expired(&self) {
        loop {
            let moved = self.moved.notified();
            tokio::pin!(moved);
            moved.as_mut().enable();
            let clock = *guard(&self.clock);
            match clock {
                Clock::Running(deadline) => tokio::select! {
                    () = tokio::time::sleep_until(deadline) => {
                        if matches!(*guard(&self.clock), Clock::Running(at) if at <= tokio::time::Instant::now()) {
                            return;
                        }
                    }
                    () = moved => {}
                },
                Clock::Paused(_) => moved.await,
            }
        }
    }

    fn pause(&self) {
        {
            let mut clock = guard(&self.clock);
            if let Clock::Running(deadline) = *clock {
                *clock =
                    Clock::Paused(deadline.saturating_duration_since(tokio::time::Instant::now()));
            }
        }
        self.moved.notify_waiters();
    }

    fn resume(&self) {
        {
            let mut clock = guard(&self.clock);
            if let Clock::Paused(left) = *clock {
                *clock = Clock::Running(tokio::time::Instant::now() + left);
            }
        }
        self.moved.notify_waiters();
    }
}

impl Lease {
    /// `key`'s resource revision the lease was taken at.
    pub(super) fn revision(&self, key: &Key) -> u64 {
        self.writer
            .resources
            .iter()
            .find(|(written, _)| written == key)
            .map_or(0, |(_, revision)| *revision)
    }

    /// The read committed the key it reads beside its own
    /// (`planner::read_beside`) in its transaction.
    pub(crate) fn landed_beside(&self) {
        self.control.beside.store(true, Ordering::Release);
    }

    pub(super) fn beside_landed(&self) -> bool {
        self.control.beside.load(Ordering::Acquire)
    }
}

impl Lease {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// When the read must have ended, its pauses left out.
    pub(crate) fn deadline(&self) -> tokio::time::Instant {
        self.control.deadline()
    }

    /// A safe point between two complete exchanges with the school: when a
    /// read that needs the lane more is waiting, this read gives its slot
    /// and session up until the engine hands them back, and the pause does
    /// not count against its deadline (`d/3mjnxv`). A cancel ends the pause.
    pub(crate) async fn checkpoint(&self) {
        let control = &self.control;
        if !control.asked.swap(false, Ordering::AcqRel) {
            return;
        }
        let Some(engine) = control.engine.engine() else {
            return;
        };
        let resumed = control.resumed.notified();
        tokio::pin!(resumed);
        resumed.as_mut().enable();
        control.pause();
        engine.lease_paused(&self.key);
        tokio::select! {
            () = resumed => {}
            () = self.cancel.cancelled() => {}
        }
        control.resume();
    }

    pub(crate) fn writer(&self) -> LeaseWriter {
        self.writer.clone()
    }

    /// The `sync_runs` row of a read that ends now.
    pub(crate) fn run_record<'a>(
        &'a self,
        outcome: RunOutcome,
        changed: usize,
        source: &'a str,
        error_kind: Option<&'a str>,
        finished_at: DateTime<Utc>,
    ) -> RunRecord<'a> {
        RunRecord {
            run: &self.run,
            key: &self.key,
            started_at: self.started,
            finished_at,
            outcome,
            changed,
            requests: None,
            bytes: None,
            source,
            error_kind,
        }
    }

    /// A lease outside any engine, for executor tests.
    #[cfg(test)]
    pub(crate) fn detached(store: &DataStore, key: Key) -> Self {
        Self {
            key,
            run: uuid::Uuid::new_v4().to_string(),
            locator: None,
            depth: Depth::Full,
            started: crate::clock::now(),
            cancel: CancellationToken::new(),
            control: Arc::new(LeaseControl::new(
                tokio::time::Instant::now() + KEY_DEADLINE,
                super::PlanHints::detached(),
            )),
            writer: LeaseWriter {
                store: store.clone(),
                gate: Arc::new(Gate::default()),
                generation: 0,
                resources: Vec::new(),
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn with_locator(mut self, locator: &str) -> Self {
        self.locator = Some(locator.into());
        self
    }
}

/// How one key read ended. Executors commit complete reads themselves (rows,
/// freshness and the run row in one transaction); every other ending is
/// recorded by the runner.
#[derive(Debug)]
pub(crate) enum KeyEnd {
    Done {
        changed: usize,
    },
    /// Nothing landed; the key backs off.
    Failed {
        kind: String,
        source: String,
        /// What the reader or the projection said, for the run's row.
        reason: Option<String>,
    },
    /// Already recorded by the executor with its own backoff (a public file
    /// held back after shrinking).
    Refused {
        kind: String,
    },
    /// The read could not reach the account's school: no credential on its
    /// path served it and none could be got. Not a failure, no backoff: the
    /// planner holds this key (at this depth) while the credential facts
    /// stay at `version`, the one the sessions decided under
    /// (`Facts::waited`), and derives anew from the facts what else waits.
    Blocked {
        profile: SchoolAuthProfile,
        wait: AccountWait,
        version: u64,
        source: String,
    },
    /// A sign-in the read needed found the shell offline and sent nothing.
    /// Not a failure, no backoff, and the refresh button's press stays: the
    /// planner holds every key while the shell reports no network.
    Offline {
        source: String,
    },
    /// The host's breaker paused it: every key of the host is held, and this
    /// key's own failure count stays.
    HostPaused {
        host: Host,
        until: DateTime<Utc>,
        source: String,
    },
    Cancelled {
        source: String,
    },
    /// The enrolment list no longer (or not yet) holds the course: nothing
    /// landed and nothing backs off. The planner holds the key until a list
    /// that holds the course lands, then reads it at once.
    NotEnrolled {
        source: String,
    },
    /// The lease's generation is gone; nothing was written.
    Stale,
}

impl KeyEnd {
    pub(crate) fn failed(kind: impl Into<String>, source: impl Into<String>) -> Self {
        Self::Failed {
            kind: kind.into(),
            source: source.into(),
            reason: None,
        }
    }

    /// A failure with the reason shown on the run's row. Bounded, since a
    /// storage or client error's text is not ours.
    pub(crate) fn failed_because(
        kind: impl Into<String>,
        source: impl Into<String>,
        reason: &str,
    ) -> Self {
        Self::Failed {
            kind: kind.into(),
            source: source.into(),
            reason: Some(reason.chars().take(MAX_FAILURE_REASON).collect()),
        }
    }

    /// The ending of a gated write that did not happen.
    pub(crate) fn from_gated<E: std::fmt::Display>(
        gated: Gated<E>,
        kind: &str,
        source: &str,
    ) -> Self {
        match gated {
            Gated::Stale => Self::Stale,
            Gated::Failed(error) => {
                log::warn!(target: "app_core::sync", "commit failed: {error}");
                Self::failed(kind, source)
            }
        }
    }
}

/// What the runner records for a read that wrote no rows: the key's
/// backoff (unless the cause was not the key: the account, a paused host, a
/// cancel) and its `sync_runs` row, inside the lease's gate.
pub(crate) struct Ending {
    writer: LeaseWriter,
    key: Key,
    run: String,
    started: DateTime<Utc>,
    source: String,
    record: Option<(RunOutcome, String, bool)>,
}

impl Ending {
    /// Blocking: writes the record and returns the key's state after it.
    pub(crate) fn write(self) -> Option<KeyState> {
        if let Some((outcome, kind, backoff)) = &self.record {
            let now = crate::clock::now();
            let written = self.writer.write(None, |transaction| {
                if *backoff {
                    sync_state::fail(transaction, &self.key, kind, now)?;
                }
                sync_state::record_run(
                    transaction,
                    &RunRecord {
                        run: &self.run,
                        key: &self.key,
                        started_at: self.started,
                        finished_at: now,
                        outcome: *outcome,
                        changed: 0,
                        requests: None,
                        bytes: None,
                        source: &self.source,
                        error_kind: Some(kind),
                    },
                )
            });
            if let Err(Gated::Failed(error)) = written {
                crate::diagnostics::report("sync", &error);
            }
        }
        self.writer
            .store()
            .catalog_key_states(std::slice::from_ref(&self.key))
            .ok()
            .and_then(|mut states| states.pop().flatten())
    }
}

impl Lease {
    pub(crate) fn ending(&self, end: &KeyEnd) -> Ending {
        let source = match end {
            KeyEnd::Failed { source, .. }
            | KeyEnd::Blocked { source, .. }
            | KeyEnd::Offline { source }
            | KeyEnd::HostPaused { source, .. }
            | KeyEnd::Cancelled { source }
            | KeyEnd::NotEnrolled { source } => source.clone(),
            _ => String::new(),
        };
        let source = if source.is_empty() {
            default_source(&self.key, self.locator.as_deref())
        } else {
            source
        };
        let record = match end {
            KeyEnd::Done { .. } | KeyEnd::Refused { .. } | KeyEnd::Stale => None,
            KeyEnd::Failed { kind, .. } => Some((RunOutcome::Failed, kind.clone(), true)),
            // Waiting for the account is not the key's failure: the planner
            // holds the key until its credential facts move.
            KeyEnd::Blocked { .. } => {
                Some((RunOutcome::SignInRequired, "sign-in".to_string(), false))
            }
            KeyEnd::Offline { .. } => Some((RunOutcome::Waiting, "offline".to_string(), false)),
            // A paused host is not this key's failure and leaves no run
            // (`d/kt8ts2`): the planner holds every key of the host.
            KeyEnd::HostPaused { .. } => None,
            KeyEnd::Cancelled { .. } => {
                Some((RunOutcome::Cancelled, "cancelled".to_string(), false))
            }
            KeyEnd::NotEnrolled { .. } => {
                Some((RunOutcome::Waiting, "not-enrolled".to_string(), false))
            }
        };
        Ending {
            writer: self.writer(),
            key: self.key.clone(),
            run: self.run.clone(),
            started: self.started,
            source,
            record,
        }
    }
}

/// Canonical `sync_runs.source` of a key whose reader did not name one.
fn default_source(key: &Key, locator: Option<&str>) -> String {
    match key.dataset {
        Dataset::LmsEnrolment
        | Dataset::LmsDeadlines
        | Dataset::LmsCourse
        | Dataset::LmsAssign
        | Dataset::LmsHistory => crate::net::moodle::source(key.dataset).into(),
        Dataset::KyomuPage => adpt_isct::kyomu::page_url(&key.key)
            .and_then(|url| url::Url::parse(&url).ok())
            .map(|url| sync_state::source_of_url(&url))
            .unwrap_or_default(),
        Dataset::CatalogOfficial => format!(
            "{}{}",
            adpt_isct::kyomu::HOST,
            crate::catalog::KYOMU_SEARCH_PATH
        ),
        Dataset::Reference => locator.unwrap_or_default().into(),
        Dataset::Announcements => crate::announcements::URL.into(),
    }
}

/// Characters of a failure's reason kept for the run's row.
const MAX_FAILURE_REASON: usize = 200;
