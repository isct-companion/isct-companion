//! Leases, lanes, the commit gate and the driver that ties them together.
//!
//! One driver task evaluates the plan when a hint, a fact or a deadline asks
//! for it, fills each lane up to its budget, starts the duties that are due
//! and publishes the snapshot and the [`WakeDirective`]. A worker reads one
//! leased key, records how it ended, and takes the next key of its lane, so
//! an LMS worker keeps its transport open across keys; a one-off read
//! queued for the lane takes the next free slot first.
use super::{
    duties::{Duty, Slot},
    planner::{
        self, Candidate, Credential, Credentials, Depth, Facts, Group, Inputs, Lane,
        ReferenceWatch, Waited,
    },
    school::{self, SchoolReaders},
    snapshot::{self, Shown},
    AccountWait, BackgroundRequest, BackgroundWork, Held, Hint, Network, OpportunityEnd, Priority,
    Scope, SyncSnapshot, View, WakeDirective,
};
use crate::{
    data::{DataError, DataStore, WorkspaceChangeSource},
    events::CoreEvent,
    lifecycle::SchoolSite,
    net::Host,
    ports::EventSink,
    runtime_tasks::{RuntimeTaskError, RuntimeTasks},
    school_auth::{SchoolAuthProfile, SchoolAuthService},
    school_session::SchoolSessionSnapshot,
    sync_state::{self, Dataset, Key, KeyState, RunOutcome, RunRecord},
};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use serde_json::Value;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch, Notify};
use tokio_util::sync::CancellationToken;

/// The longest one key may take, from its start, sign-in included. A key
/// over it fails as `timeout`; a reset waits at most this long for commits
/// in progress.
pub const KEY_DEADLINE: Duration = Duration::from_secs(3 * 60);
/// A whole-department 教務 search is one GET and one 3 MB postback.
const BULK_DEADLINE: Duration = Duration::from_secs(6 * 60);
/// A second press of a refresh button within this time joins the first.
const MANUAL_JOIN: Duration = Duration::from_secs(30);
/// How long an opened page or a refresh press keeps its keys wanted while
/// they wait (offline, paused host).
const INTENT_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// A failed run without a retry time stays visible this long.
const FAILURE_SHOWN: Duration = Duration::from_secs(30 * 60);

fn deadline(dataset: Dataset) -> Duration {
    if dataset == Dataset::CatalogOfficial {
        BULK_DEADLINE
    } else {
        KEY_DEADLINE
    }
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// -- Commit gate --------------------------------------------------------------

#[derive(Default)]
struct GateState {
    generation: u64,
    committing: usize,
    /// Moves when an LMS write starts on a key's resource; a lease taken
    /// before it never commits (`PlanEngine::bump_resource`).
    resources: HashMap<Key, u64>,
}

/// Every write of a lease passes here. Closing admission bumps the
/// generation under the same lock, so a commit either entered before the
/// bump (and `wait_idle` waits for it) or is refused; likewise a write that
/// started on the key's resource after the lease was taken.
pub(crate) struct Gate {
    state: Mutex<GateState>,
    /// The commits in progress, for `wait_idle`.
    committing: watch::Sender<usize>,
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
    fn generation(&self) -> u64 {
        guard(&self.state).generation
    }

    fn bump(&self) -> u64 {
        let mut state = guard(&self.state);
        state.generation = state.generation.saturating_add(1);
        state.generation
    }

    fn resource(&self, key: &Key) -> u64 {
        guard(&self.state).resources.get(key).copied().unwrap_or(0)
    }

    #[cfg_attr(not(feature = "agent"), allow(dead_code))]
    fn bump_resource(&self, key: &Key) -> u64 {
        let mut state = guard(&self.state);
        let revision = state.resources.entry(key.clone()).or_default();
        *revision = revision.saturating_add(1);
        *revision
    }

    fn enter(
        self: &Arc<Self>,
        generation: u64,
        resource: Option<&(Key, u64)>,
    ) -> Option<GateGuard> {
        let mut state = guard(&self.state);
        if state.generation != generation {
            return None;
        }
        if let Some((key, revision)) = resource {
            if state
                .resources
                .get(key)
                .is_some_and(|current| current > revision)
            {
                return None;
            }
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
    store: DataStore,
    gate: Arc<Gate>,
    generation: u64,
    /// The key's resource revision when it was leased.
    resource: Option<(Key, u64)>,
}

impl LeaseWriter {
    pub(crate) fn store(&self) -> &DataStore {
        &self.store
    }

    /// Holds the gate while the caller writes through store methods that
    /// open their own transaction. `None`: the lease is stale.
    pub(crate) fn enter(&self) -> Option<GateGuard> {
        self.gate.enter(self.generation, self.resource.as_ref())
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
    /// The key's deadline, counted from its start.
    pub(crate) deadline: tokio::time::Instant,
    pub(crate) cancel: CancellationToken,
    writer: LeaseWriter,
}

impl Lease {
    /// The key's resource revision the lease was taken at.
    fn revision(&self) -> u64 {
        self.writer
            .resource
            .as_ref()
            .map_or(0, |(_, revision)| *revision)
    }
}

impl Lease {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
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
            started: Utc::now(),
            deadline: tokio::time::Instant::now() + KEY_DEADLINE,
            cancel: CancellationToken::new(),
            writer: LeaseWriter {
                store: store.clone(),
                gate: Arc::new(Gate::default()),
                generation: 0,
                resource: None,
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
            let now = Utc::now();
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
            KeyEnd::HostPaused { .. } => {
                Some((RunOutcome::Failed, "host-paused".to_string(), false))
            }
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
    }
}

// -- State --------------------------------------------------------------------

struct LeaseEntry {
    cancel: CancellationToken,
}

/// Characters of a failure's reason kept for the run's row.
const MAX_FAILURE_REASON: usize = 200;

/// One failed key of a run.
pub(super) struct FailedKey {
    pub(super) kind: String,
    pub(super) retry: Option<DateTime<Utc>>,
    pub(super) reason: Option<String>,
}

/// One run in the snapshot: the keys of one group that became due
/// together, counted as they finish.
pub(super) struct Batch {
    pub(super) run: String,
    pub(super) group: Group,
    pub(super) priority: Priority,
    pub(super) pending: BTreeSet<Key>,
    pub(super) running: BTreeMap<Key, Option<String>>,
    pub(super) done: BTreeSet<Key>,
    pub(super) failed: BTreeMap<Key, FailedKey>,
    pub(super) held: Option<super::Held>,
    pub(super) finished_at: Option<Instant>,
}

impl Batch {
    fn open(&self) -> bool {
        self.finished_at.is_none()
    }

    fn idle(&self) -> bool {
        self.pending.is_empty() && self.running.is_empty()
    }
}

enum ManualTarget {
    Scope(Scope),
    /// The current-term courses of an LMS refresh, once its course list
    /// was read.
    LmsCurrentCourses,
}

/// Why a one-off read did not run. Only the desktop's LMS writes read
/// this way today (`approach/desktop-only-features-compiled-out`).
#[cfg_attr(not(feature = "agent"), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub(crate) enum ReadRefused {
    /// It cannot run now, for this reason; nothing was sent.
    #[error("The read waits ({0:?})")]
    Held(Held),
    /// A reset or the account's barrier ended it.
    #[error("The read was cancelled")]
    Cancelled,
    #[error("The read timed out")]
    Deadline,
    /// The read ran and failed (a refreshed key's error kind).
    #[error("The read failed ({0})")]
    Failed(String),
    #[error("Core has stopped")]
    Stopped,
}

/// A one-off read the engine admits like a key: the same holds, the lane's
/// slots, the engine's ownership and a reset's cancellation. Requests with
/// the same `key` join the one queued or running.
#[cfg_attr(not(feature = "agent"), allow(dead_code))]
pub(crate) struct ReadRequest {
    /// What the read is for and how fresh it must be; equal keys share one
    /// read.
    pub(crate) key: String,
    pub(crate) lane: Lane,
    /// The account and the credential path the read uses.
    pub(crate) account: Option<(SchoolAuthProfile, &'static [Credential])>,
    pub(crate) deadline: Duration,
}

type Output = Arc<dyn Any + Send + Sync>;
type Outcome = Result<Output, ReadRefused>;
type Work = Box<dyn FnOnce(CancellationToken) -> crate::net::BoxFuture<'static, Output> + Send>;

struct Pending {
    key: String,
    lane: Lane,
    deadline: Duration,
    work: Work,
}

/// Someone waiting for a key's read that started at `revision` or later.
struct Waiter {
    revision: u64,
    reply: oneshot::Sender<Result<(), ReadRefused>>,
}

struct State {
    facts: Facts,
    /// Each account as its session snapshot last said, for the status line.
    sessions: HashMap<SchoolAuthProfile, Shown>,
    /// The revision `sessions` came from: session events may arrive out of
    /// order, and the highest revision is the current one.
    sessions_revision: u64,
    inputs: Inputs,
    plan: Vec<Candidate>,
    leases: HashMap<Key, LeaseEntry>,
    busy: HashMap<Lane, usize>,
    batches: Vec<Batch>,
    opens: HashMap<View, Instant>,
    /// Keys a refresh button asked for, until they are read.
    manual: HashMap<Key, (Instant, Option<String>)>,
    /// When each key was last asked for by hand, for the 30 s join.
    last_manual: HashMap<Key, Instant>,
    requests: Vec<(ManualTarget, Instant)>,
    /// Keys a page showed since the last evaluation; touched for gc.
    shown: Vec<Key>,
    /// An LMS refresh waits for its course list before naming its courses.
    follow_lms: Option<Instant>,
    closed: usize,
    stopped: bool,
    evaluate: bool,
    last_evaluated: Option<Instant>,
    published: Option<SyncSnapshot>,
    /// When the plan next changes by time alone (`planner::next_due`).
    plan_next: Option<DateTime<Utc>>,
    duties: Vec<Slot>,
    /// One-off reads waiting for a slot of their lane.
    reads: VecDeque<Pending>,
    /// Every queued or running one-off read, by key, with its answer.
    answers: HashMap<String, watch::Sender<Option<Outcome>>>,
    running: HashMap<String, CancellationToken>,
    waiters: HashMap<Key, Vec<Waiter>>,
    /// OS background opportunities in progress, by id: whether each
    /// allows the network. Android runs a network and a local one at once.
    opportunities: HashMap<String, bool>,
}

struct Shared {
    store: DataStore,
    auth: Arc<SchoolAuthService>,
    events: OnceLock<Arc<dyn EventSink>>,
    school: Mutex<Option<SchoolReaders>>,
    state: Mutex<State>,
    gate: Arc<Gate>,
    wake: Notify,
    tasks: RuntimeTasks,
    runtime: OnceLock<tokio::runtime::Handle>,
    driving: AtomicBool,
    probes: Arc<crate::reference_refresh::IndexProbes>,
    /// Moves after every change a waiter could be waiting for
    /// (`wait_quiet`).
    version: watch::Sender<u64>,
    directive: watch::Sender<WakeDirective>,
}

/// The sync planner and runner. Cheap to clone.
#[derive(Clone)]
pub struct PlanEngine(Arc<Shared>);

/// A weak handle for components the engine itself owns (the school
/// browser), so hints never keep the engine alive.
#[derive(Clone)]
pub struct PlanHints(Weak<Shared>);

impl PlanHints {
    /// Hints sent to nothing; for components built without an engine.
    pub fn detached() -> Self {
        Self(Weak::new())
    }

    pub fn send(&self, hint: Hint) {
        if let Some(shared) = self.0.upgrade() {
            PlanEngine(shared).hint(hint);
        }
    }

    /// A duty brought its next time forward (`PlanEngine::duties_changed`).
    pub(crate) fn duties_changed(&self) {
        if let Some(shared) = self.0.upgrade() {
            PlanEngine(shared).duties_changed();
        }
    }

    /// The engine while something else still owns it.
    pub(crate) fn engine(&self) -> Option<PlanEngine> {
        self.0.upgrade().map(PlanEngine)
    }

    /// Folds a Core event (the session snapshot) into the snapshot.
    pub fn observe(&self, event: &CoreEvent) {
        if let Some(shared) = self.0.upgrade() {
            PlanEngine(shared).observe(event);
        }
    }
}

/// Admission stays closed while this lives; dropping it reopens.
pub struct AdmissionGuard {
    engine: PlanEngine,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        {
            let mut state = guard(&self.engine.0.state);
            state.closed = state.closed.saturating_sub(1);
            state.evaluate = true;
        }
        self.engine.0.wake.notify_one();
    }
}

/// The accounts as the status line shows them.
fn shown(snapshot: &SchoolSessionSnapshot) -> HashMap<SchoolAuthProfile, Shown> {
    [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech]
        .into_iter()
        .map(|profile| {
            let session = snapshot.profile(profile);
            (
                profile,
                Shown {
                    state: session.state,
                    maintenance: session
                        .maintenance
                        .as_ref()
                        .and_then(|maintenance| maintenance.notice.clone()),
                },
            )
        })
        .collect()
}

impl PlanEngine {
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>, network: Network) -> Self {
        let facts = Facts {
            network,
            ..Facts::default()
        };
        Self(Arc::new(Shared {
            store,
            auth,
            events: OnceLock::new(),
            school: Mutex::new(None),
            state: Mutex::new(State {
                facts,
                sessions: HashMap::new(),
                sessions_revision: 0,
                inputs: Inputs::default(),
                plan: Vec::new(),
                leases: HashMap::new(),
                busy: HashMap::new(),
                batches: Vec::new(),
                opens: HashMap::new(),
                manual: HashMap::new(),
                last_manual: HashMap::new(),
                requests: Vec::new(),
                shown: Vec::new(),
                follow_lms: None,
                closed: 0,
                stopped: false,
                evaluate: true,
                last_evaluated: None,
                published: None,
                plan_next: None,
                duties: Vec::new(),
                reads: VecDeque::new(),
                answers: HashMap::new(),
                running: HashMap::new(),
                waiters: HashMap::new(),
                opportunities: HashMap::new(),
            }),
            gate: Arc::new(Gate::default()),
            wake: Notify::new(),
            tasks: RuntimeTasks::default(),
            runtime: OnceLock::new(),
            driving: AtomicBool::new(false),
            probes: Arc::new(crate::reference_refresh::IndexProbes::default()),
            version: watch::channel(0).0,
            directive: watch::channel(WakeDirective::default()).0,
        }))
    }

    /// Adds a deadline duty; the engine runs it whenever it is due.
    pub(crate) fn register(&self, duty: Arc<dyn Duty>) {
        guard(&self.0.state).duties.push(Slot::new(duty));
        self.0.wake.notify_one();
    }

    /// A duty brought its next time forward: look again now.
    pub(crate) fn duties_changed(&self) {
        self.0.wake.notify_one();
    }

    /// The shell's timer for `revision` fired. A stale revision only costs
    /// one look; the plan is evaluated again only when its own time came.
    pub fn deadline_reached(&self, revision: u64) {
        self.ensure_started();
        {
            let mut state = guard(&self.0.state);
            if state
                .plan_next
                .is_some_and(|at| at <= Utc::now() + chrono::Duration::seconds(1))
            {
                state.evaluate = true;
            }
        }
        log::debug!(target: "app_core::plan", "deadline {revision} reached");
        self.0.wake.notify_one();
    }

    /// The directive as last published.
    pub fn wake_directive(&self) -> WakeDirective {
        self.0.directive.borrow().clone()
    }

    /// Every directive from now on, for a shell timer in this process.
    pub fn wake_directives(&self) -> watch::Receiver<WakeDirective> {
        self.0.directive.subscribe()
    }

    /// An LMS write starts on `key`'s resource: a read leased before now
    /// does not commit. Returns the revision a read after the write has.
    #[cfg_attr(not(feature = "agent"), allow(dead_code))]
    pub(crate) fn bump_resource(&self, key: &Key) -> u64 {
        self.0.gate.bump_resource(key)
    }

    /// Reads `key` in full as a refresh press does, past the 30 s join, and
    /// waits until a read leased at `revision` or later ended: `Ok` once
    /// it landed. A wait the key cannot end yet (offline, the account)
    /// lasts until it can; the caller bounds it.
    #[cfg_attr(not(feature = "agent"), allow(dead_code))]
    pub(crate) async fn refresh_key(&self, key: Key, revision: u64) -> Result<(), ReadRefused> {
        self.ensure_started();
        let credentials = self.credentials();
        let (reply, answer) = oneshot::channel();
        {
            let mut state = guard(&self.0.state);
            if state.stopped {
                return Err(ReadRefused::Stopped);
            }
            // Held by a fact now (offline, the account): refused at once, as
            // a one-off read is; the caller asks again later.
            let account = planner::profile_of(key.dataset)
                .map(|profile| (profile, planner::credential_path(&key, Depth::Full)));
            if let Some(held) = planner::request_hold(
                Lane::of(key.dataset),
                account,
                &state.facts,
                Some(&credentials),
                Utc::now(),
            ) {
                return Err(ReadRefused::Held(held));
            }
            let now = Instant::now();
            state.last_manual.insert(key.clone(), now);
            state.manual.insert(key.clone(), (now, None));
            // Revisions count from this run's start: after a restart every
            // read is later than a write recorded before it.
            let revision = revision.min(self.0.gate.resource(&key));
            state
                .waiters
                .entry(key)
                .or_default()
                .push(Waiter { revision, reply });
            state.evaluate = true;
        }
        self.0.wake.notify_one();
        answer.await.map_err(|_| ReadRefused::Stopped)?
    }

    /// Runs `work` as a one-off read on `request.lane`, once the lane has a
    /// slot, unless it is held now: a held read is refused at once with
    /// the reason, never queued behind a fact. The engine owns the read: a
    /// caller that stops waiting leaves it running, and a reset cancels it.
    #[cfg_attr(not(feature = "agent"), allow(dead_code))]
    pub(crate) async fn read<T, F>(&self, request: ReadRequest, work: F) -> Result<T, ReadRefused>
    where
        T: Clone + Send + Sync + 'static,
        F: FnOnce(CancellationToken) -> crate::net::BoxFuture<'static, T> + Send + 'static,
    {
        self.ensure_started();
        let school = self.school();
        let credentials = school.as_ref().map(school::credentials);
        let mut answer = {
            let mut state = guard(&self.0.state);
            if state.stopped {
                return Err(ReadRefused::Stopped);
            }
            if state.closed > 0 {
                return Err(ReadRefused::Held(Held::Resetting));
            }
            // Without sessions (an engine built alone) the read's own
            // transport decides what it can reach.
            let account = request.account.filter(|_| school.is_some());
            if let Some(held) = planner::request_hold(
                request.lane,
                account,
                &state.facts,
                credentials.as_ref(),
                Utc::now(),
            ) {
                return Err(ReadRefused::Held(held));
            }
            match state.answers.get(&request.key) {
                Some(sender) => sender.subscribe(),
                None => {
                    let (sender, answer) = watch::channel(None);
                    state.answers.insert(request.key.clone(), sender);
                    let work: Work = Box::new(move |cancel| {
                        let read = work(cancel);
                        Box::pin(async move { Arc::new(read.await) as Output })
                    });
                    state.reads.push_back(Pending {
                        key: request.key,
                        lane: request.lane,
                        deadline: request.deadline,
                        work,
                    });
                    answer
                }
            }
        };
        self.0.wake.notify_one();
        let outcome = answer
            .wait_for(Option::is_some)
            .await
            .map_err(|_| ReadRefused::Stopped)?
            .clone()
            .ok_or(ReadRefused::Stopped)?;
        outcome?
            .downcast_ref::<T>()
            .cloned()
            .ok_or(ReadRefused::Stopped)
    }

    /// An OS background opportunity begins: reads run again when it allows
    /// the network, and the duties its kind of work allows run.
    pub(crate) fn begin_opportunity(&self, id: String, network: bool) {
        self.ensure_started();
        {
            let mut state = guard(&self.0.state);
            if network {
                state.facts.suspended = false;
            }
            state.opportunities.insert(id, network);
            state.evaluate = true;
        }
        self.0.wake.notify_one();
    }

    /// The opportunity `id` ended. Once no opportunity that allows the
    /// network is left and the app is not in front, no new key is leased,
    /// the rebuildable reads stop (what they leave stays wanted for the
    /// next one) and the network duties are asked to stop between steps;
    /// local commits finish. Returns whether the app is out of front with
    /// no opportunity left, so Core suspends its sessions too.
    pub(crate) fn end_opportunity(&self, id: &str, reason: OpportunityEnd) -> bool {
        let (suspended, interrupt) = {
            let mut state = guard(&self.0.state);
            let Some(network) = state.opportunities.remove(id) else {
                return false;
            };
            log::info!(target: "app_core::plan", "background opportunity {id} ended ({reason:?})");
            let network_left = state.opportunities.values().any(|network| *network);
            let mut interrupt = Vec::new();
            if network && !network_left && !state.facts.foreground {
                state.facts.suspended = true;
                for lease in state.leases.values() {
                    lease.cancel.cancel();
                }
                for cancel in state.running.values() {
                    cancel.cancel();
                }
                interrupt = state
                    .duties
                    .iter()
                    .filter(|slot| slot.running && slot.duty.work() == BackgroundWork::Network)
                    .map(|slot| slot.duty.clone())
                    .collect();
            }
            state.evaluate = true;
            (
                state.opportunities.is_empty() && !state.facts.foreground,
                interrupt,
            )
        };
        for duty in interrupt {
            duty.interrupt();
        }
        self.0.wake.notify_one();
        suspended
    }

    pub fn hints(&self) -> PlanHints {
        PlanHints(Arc::downgrade(&self.0))
    }

    /// Where snapshots go. Set once, by the facade.
    pub(crate) fn set_events(&self, events: Arc<dyn EventSink>) {
        let _ = self.0.events.set(events);
    }

    /// The school sessions and clients LMS and 教務Web keys read with. The
    /// planner is told whenever their credential facts move.
    pub(crate) fn attach_school(&self, readers: SchoolReaders) {
        let snapshot = readers.sessions.snapshot();
        let sessions = shown(&snapshot);
        let hints = self.hints();
        readers
            .sessions
            .credential_signal()
            .listen(Arc::new(move || {
                if let Some(engine) = hints.engine() {
                    engine.reevaluate();
                }
            }));
        *guard(&self.0.school) = Some(readers);
        {
            let mut state = guard(&self.0.state);
            if snapshot.revision >= state.sessions_revision {
                state.sessions = sessions;
                state.sessions_revision = snapshot.revision;
            }
            state.evaluate = true;
        }
        self.0.wake.notify_one();
    }

    /// Stored facts changed outside a key (saved credentials, settings):
    /// evaluate again on the next driver turn.
    pub(crate) fn reevaluate(&self) {
        guard(&self.0.state).evaluate = true;
        self.0.wake.notify_one();
    }

    fn school(&self) -> Option<SchoolReaders> {
        guard(&self.0.school).clone()
    }

    /// The credential facts as the school sessions hold them now. Read
    /// before the state lock is taken: the sessions call back into the
    /// engine when they change, never the other way round under a lock.
    fn credentials(&self) -> Credentials {
        match self.school() {
            Some(readers) => school::credentials(&readers),
            None => Credentials::default(),
        }
    }

    /// Starts the driver on the shell's runtime. Later calls do nothing.
    pub fn start(&self, handle: &tokio::runtime::Handle) -> Result<(), RuntimeTaskError> {
        let _ = self.0.runtime.set(handle.clone());
        if self.0.driving.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let engine = self.clone();
        if !self.0.tasks.spawn_on(handle, engine.drive()) {
            self.0.driving.store(false, Ordering::Release);
            return Err(RuntimeTaskError::Stopped);
        }
        // The planner evaluates once at start.
        guard(&self.0.state).evaluate = true;
        self.0.wake.notify_one();
        Ok(())
    }

    /// A hint from a thread inside a runtime starts the driver too, so a
    /// shell that never called `start` still syncs once it wakes Core.
    fn ensure_started(&self) {
        if self.0.driving.load(Ordering::Acquire) {
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let _ = self.start(&handle);
        }
    }

    /// Takes in one shell fact or intent. Never blocks on I/O.
    pub fn hint(&self, hint: Hint) {
        self.ensure_started();
        let now = Instant::now();
        let evaluate = {
            let mut state = guard(&self.0.state);
            let evaluate = match hint {
                Hint::Foreground => {
                    state.facts.foreground = true;
                    true
                }
                Hint::Background => {
                    state.facts.foreground = false;
                    true
                }
                Hint::Suspend => {
                    state.facts.suspended = true;
                    true
                }
                Hint::Resume => {
                    state.facts.suspended = false;
                    true
                }
                Hint::Network { network } => report_network(&mut state, network),
                Hint::PowerSaver { on } => {
                    let changed = state.facts.power_saver != on;
                    state.facts.power_saver = on;
                    changed
                }
                Hint::Open { view } => {
                    if let View::Course { lms_course_id } = &view {
                        state.shown.extend(
                            [Dataset::LmsCourse, Dataset::LmsAssign]
                                .map(|dataset| Key::new(dataset, lms_course_id.to_string())),
                        );
                    }
                    state.opens.insert(view, now);
                    true
                }
                Hint::Refresh { scope } => {
                    state.requests.push((ManualTarget::Scope(scope), now));
                    true
                }
            };
            if evaluate {
                state.evaluate = true;
            }
            evaluate
        };
        if evaluate {
            self.0.wake.notify_one();
        }
    }

    /// Folds the session snapshot into the status line; the planner never
    /// reads it. The facade's event relay calls this for every event.
    pub fn observe(&self, event: &CoreEvent) {
        let CoreEvent::SchoolSession(snapshot) = event else {
            return;
        };
        let sessions = shown(snapshot);
        {
            let mut state = guard(&self.0.state);
            if snapshot.revision < state.sessions_revision {
                return;
            }
            state.sessions_revision = snapshot.revision;
            if state.sessions == sessions {
                return;
            }
            state.sessions = sessions;
        }
        self.0.wake.notify_one();
    }

    /// The current snapshot, as last sent or newer.
    pub fn snapshot(&self) -> SyncSnapshot {
        let state = guard(&self.0.state);
        let generation = self.0.gate.generation();
        let leased: HashSet<Key> = state.leases.keys().cloned().collect();
        snapshot::build(snapshot::Parts {
            generation,
            resetting: state.closed > 0,
            batches: &state.batches,
            leased: &leased,
            plan: &state.plan,
            sessions: &state.sessions,
            inputs: &state.inputs,
            now: Utc::now(),
        })
    }

    pub fn generation(&self) -> u64 {
        self.0.gate.generation()
    }

    /// Stops leasing, cancels in-flight reads, bumps the generation. Every
    /// commit checks its lease's generation inside the write gate, so a task
    /// of an old generation never writes. Dropping the guard reopens.
    pub async fn close_admission(&self) -> AdmissionGuard {
        {
            // Bumped under the state lock: no lease of the new generation
            // can start before admission is closed.
            let mut state = guard(&self.0.state);
            state.closed += 1;
            self.0.gate.bump();
            // A reset clears the sessions: a read finds out anew.
            state.facts.waited.clear();
            for lease in state.leases.values() {
                lease.cancel.cancel();
            }
            cancel_requests(&mut state);
            state.plan.clear();
            state.batches.clear();
            state.manual.clear();
            state.last_manual.clear();
            state.requests.clear();
            state.follow_lms = None;
            state.evaluate = true;
        }
        self.0.wake.notify_one();
        tokio::task::yield_now().await;
        AdmissionGuard {
            engine: self.clone(),
        }
    }

    /// Whether the shell last reported its app in front and running.
    pub fn in_front(&self) -> bool {
        let state = guard(&self.0.state);
        state.facts.foreground && !state.facts.suspended
    }

    /// Evaluates once more, then waits until no key is leased, none the
    /// facts allow is left to lease, no one-off read is queued or running
    /// and no duty the facts allow runs or is due, at most `limit`. An OS
    /// background opportunity holds the process open this long.
    pub async fn wait_quiet(&self, limit: Duration) -> bool {
        let since = Instant::now();
        let deadline = tokio::time::Instant::now() + limit;
        // Subscribed before the first look, so no change is missed.
        let mut changes = self.0.version.subscribe();
        self.reevaluate();
        loop {
            if self.quiet(since) {
                return true;
            }
            match tokio::time::timeout_at(deadline, changes.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return self.quiet(since),
            }
        }
    }

    fn quiet(&self, since: Instant) -> bool {
        let credentials = self.credentials();
        let duties = {
            let state = guard(&self.0.state);
            let now = Utc::now();
            let reads_quiet = !state.evaluate
                && state.last_evaluated.is_some_and(|at| at >= since)
                && state.leases.is_empty()
                && state.reads.is_empty()
                && state.running.is_empty()
                && !state.plan.iter().any(|candidate| {
                    candidate.held.is_none()
                        && planner::hold(
                            &candidate.key,
                            candidate.priority,
                            candidate.depth,
                            &state.inputs,
                            &state.facts,
                            &credentials,
                            now,
                        )
                        .is_none()
                });
            if !reads_quiet || state.duties.iter().any(|slot| slot.running) {
                return false;
            }
            let allowed = allowed_work(&state);
            state
                .duties
                .iter()
                .filter(|slot| allowed(slot.duty.work()))
                .map(|slot| slot.duty.clone())
                .collect::<Vec<_>>()
        };
        !duties
            .iter()
            .any(|duty| duty.next_due().is_some_and(|at| at <= Utc::now()))
    }

    /// Waits until no commit is in progress, at most `limit`.
    pub async fn wait_idle(&self, limit: Duration) -> bool {
        let mut committing = self.0.gate.committing.subscribe();
        tokio::time::timeout(limit, committing.wait_for(|count| *count == 0))
            .await
            .is_ok_and(|waited| waited.is_ok())
    }

    /// Cancels every read and waits for the driver and workers to stop.
    pub async fn shutdown(&self) {
        self.stop();
        self.0.tasks.wait().await;
    }

    /// Stops leasing, cancels reads and ends the driver without waiting, so
    /// a Core dropped without `shutdown` releases the engine and its ports.
    pub(crate) fn stop(&self) {
        {
            let mut state = guard(&self.0.state);
            state.stopped = true;
            for lease in state.leases.values() {
                lease.cancel.cancel();
            }
            cancel_requests(&mut state);
            for waiters in std::mem::take(&mut state.waiters).into_values() {
                for waiter in waiters {
                    let _ = waiter.reply.send(Err(ReadRefused::Stopped));
                }
            }
        }
        self.0.tasks.close();
    }

    // -- Driver ---------------------------------------------------------------

    async fn drive(self) {
        loop {
            tokio::select! {
                _ = self.0.wake.notified() => {}
                _ = self.0.tasks.closed() => break,
            }
            let evaluate = std::mem::take(&mut guard(&self.0.state).evaluate);
            if evaluate {
                self.evaluate().await;
            }
            self.pump();
            self.run_duties();
            self.publish();
        }
    }

    /// Starts every duty that is due and that the facts allow (a suspended
    /// app runs only what its background opportunity allows).
    fn run_duties(&self) {
        let Some(handle) = self.0.runtime.get().cloned() else {
            return;
        };
        let idle = {
            let state = guard(&self.0.state);
            if state.stopped {
                return;
            }
            let allowed = allowed_work(&state);
            state
                .duties
                .iter()
                .enumerate()
                .filter(|(_, slot)| !slot.running && allowed(slot.duty.work()))
                .map(|(index, slot)| (index, slot.duty.clone()))
                .collect::<Vec<_>>()
        };
        for (index, duty) in idle {
            if !duty.next_due().is_some_and(|at| at <= Utc::now()) {
                continue;
            }
            {
                let mut state = guard(&self.0.state);
                let Some(slot) = state.duties.get_mut(index) else {
                    continue;
                };
                if slot.running {
                    continue;
                }
                slot.running = true;
            }
            let engine = self.clone();
            let name = duty.name();
            let started = self.0.tasks.spawn_on(&handle, async move {
                log::debug!(target: "app_core::plan", "{name} runs");
                duty.run().await;
                if let Some(slot) = guard(&engine.0.state).duties.get_mut(index) {
                    slot.running = false;
                }
                engine.0.wake.notify_one();
            });
            if !started {
                if let Some(slot) = guard(&self.0.state).duties.get_mut(index) {
                    slot.running = false;
                }
            }
        }
    }

    async fn evaluate(&self) {
        let (store, auth) = (self.0.store.clone(), self.0.auth.clone());
        let shown = std::mem::take(&mut guard(&self.0.state).shown);
        let loaded = tokio::task::spawn_blocking(move || {
            if !shown.is_empty() {
                // Only gc reads `last_used`; a lost touch costs nothing now.
                if let Err(error) = store.write(None, |transaction| {
                    sync_state::touch(transaction, &shown, Utc::now())
                }) {
                    crate::diagnostics::report("sync", &error);
                }
            }
            load_inputs(&store, &auth, Utc::now())
        })
        .await;
        let inputs = match loaded {
            Ok(Ok(inputs)) => inputs,
            Ok(Err(error)) => {
                crate::diagnostics::report("sync", &error);
                return;
            }
            Err(error) => {
                crate::diagnostics::report("sync", &error);
                return;
            }
        };
        let credentials = self.credentials();
        let now = Utc::now();
        let instant = Instant::now();
        let mut state = guard(&self.0.state);
        state.last_evaluated = Some(instant);
        let fresh =
            |at: &Instant, lifetime: Duration| instant.saturating_duration_since(*at) < lifetime;
        state.opens.retain(|_, at| fresh(&*at, INTENT_LIFETIME));
        state
            .manual
            .retain(|_, (at, _)| fresh(&*at, INTENT_LIFETIME));
        state.last_manual.retain(|_, at| fresh(&*at, MANUAL_JOIN));
        let requests = std::mem::take(&mut state.requests);
        for (target, at) in requests {
            let keys = match &target {
                ManualTarget::Scope(scope) => planner::scope_keys(scope, &inputs, now),
                ManualTarget::LmsCurrentCourses => planner::current_course_keys(&inputs, now)
                    .into_iter()
                    .map(|key| (key, None))
                    .collect(),
            };
            for (key, scope) in keys {
                request_manual(&mut state, key, scope, at);
            }
            let reads_lms_list = matches!(
                target,
                ManualTarget::Scope(Scope::Everything)
                    | ManualTarget::Scope(Scope::Site {
                        site: SchoolSite::ScienceLms
                    })
            );
            if reads_lms_list {
                state.follow_lms = Some(at);
            }
        }
        state.facts.resetting = state.closed > 0;
        state.facts.paused.retain(|_, until| *until > now);
        // A wait seen under facts that moved since holds nothing. One a read
        // settled after these facts were read is newer, and stays.
        state
            .facts
            .waited
            .retain(|_, waited| waited.version >= credentials.version);
        let opens: Vec<View> = state.opens.keys().cloned().collect();
        let manual: HashMap<Key, Option<String>> = state
            .manual
            .iter()
            .map(|(key, (_, scope))| (key.clone(), scope.clone()))
            .collect();
        let leased: HashSet<Key> = state.leases.keys().cloned().collect();
        state.plan = planner::plan(
            &inputs,
            &state.facts,
            &credentials,
            &opens,
            &manual,
            &leased,
            now,
        );
        state.plan_next = planner::next_due(&inputs, &state.plan, &opens, &manual, now);
        state.inputs = inputs;
        update_batches(&mut state, instant, now);
    }

    /// Leases due keys into every lane with room.
    fn pump(&self) {
        let Some(handle) = self.0.runtime.get().cloned() else {
            return;
        };
        let mut started = Vec::new();
        let credentials = self.credentials();
        {
            let mut state = guard(&self.0.state);
            if state.closed > 0 || state.stopped {
                return;
            }
            for lane in Lane::ALL {
                while state.busy.get(&lane).copied().unwrap_or(0) < lane.capacity() {
                    let Some(index) = state.reads.iter().position(|request| request.lane == lane)
                    else {
                        break;
                    };
                    let Some(request) = state.reads.remove(index) else {
                        break;
                    };
                    // The facts may have moved while it queued (the app
                    // went out of front, the network dropped).
                    if let Some(held) =
                        planner::request_hold(lane, None, &state.facts, None, Utc::now())
                    {
                        if let Some(sender) = state.answers.remove(&request.key) {
                            sender.send_replace(Some(Err(ReadRefused::Held(held))));
                        }
                        continue;
                    }
                    let cancel = CancellationToken::new();
                    state.running.insert(request.key.clone(), cancel.clone());
                    *state.busy.entry(lane).or_default() += 1;
                    let key = request.key.clone();
                    if !self
                        .0
                        .tasks
                        .spawn_on(&handle, self.clone().run_request(request, cancel))
                    {
                        state.running.remove(&key);
                        if let Some(busy) = state.busy.get_mut(&lane) {
                            *busy = busy.saturating_sub(1);
                        }
                        if let Some(sender) = state.answers.remove(&key) {
                            sender.send_replace(Some(Err(ReadRefused::Stopped)));
                        }
                    }
                }
                while state.busy.get(&lane).copied().unwrap_or(0) < lane.capacity() {
                    let Some(lease) = self.lease_next(&mut state, lane, &credentials) else {
                        break;
                    };
                    *state.busy.entry(lane).or_default() += 1;
                    started.push((lane, lease));
                }
            }
        }
        for (lane, lease) in started {
            let key = lease.key.clone();
            if !self
                .0
                .tasks
                .spawn_on(&handle, self.clone().work(lane, lease))
            {
                let mut state = guard(&self.0.state);
                state.leases.remove(&key);
                if let Some(busy) = state.busy.get_mut(&lane) {
                    *busy = busy.saturating_sub(1);
                }
            }
        }
    }

    /// The first ready key of `lane` in plan order, leased. Each key's hold
    /// is asked again with `credentials` as they are now: the plan may be
    /// older than a read that just ended waiting.
    fn lease_next(
        &self,
        state: &mut State,
        lane: Lane,
        credentials: &Credentials,
    ) -> Option<Lease> {
        if state.closed > 0 || state.stopped {
            return None;
        }
        let now = Utc::now();
        let index = state.plan.iter().position(|candidate| {
            candidate.lane == lane
                && candidate.held.is_none()
                && !state.leases.contains_key(&candidate.key)
                && planner::hold(
                    &candidate.key,
                    candidate.priority,
                    candidate.depth,
                    &state.inputs,
                    &state.facts,
                    credentials,
                    now,
                )
                .is_none()
        })?;
        let candidate = state.plan.remove(index);
        let batch = state
            .batches
            .iter_mut()
            .find(|batch| batch.open() && batch.pending.contains(&candidate.key));
        let run = match batch {
            Some(batch) => {
                batch.pending.remove(&candidate.key);
                batch
                    .running
                    .insert(candidate.key.clone(), candidate.title.clone());
                batch.held = None;
                batch.run.clone()
            }
            None => uuid::Uuid::new_v4().to_string(),
        };
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + deadline(candidate.key.dataset);
        state.leases.insert(
            candidate.key.clone(),
            LeaseEntry {
                cancel: cancel.clone(),
            },
        );
        let key = candidate.key.clone();
        Some(Lease {
            key: candidate.key,
            run,
            locator: candidate.locator,
            depth: candidate.depth,
            started: now,
            deadline,
            cancel,
            writer: LeaseWriter {
                store: self.0.store.clone(),
                gate: self.0.gate.clone(),
                generation: self.0.gate.generation(),
                resource: Some((key.clone(), self.0.gate.resource(&key))),
            },
        })
    }

    /// One one-off read, from its slot to its answer.
    async fn run_request(self, request: Pending, cancel: CancellationToken) {
        let generation = self.0.gate.generation();
        let read = (request.work)(cancel.clone());
        let outcome = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ReadRefused::Cancelled),
            ended = tokio::time::timeout(request.deadline, read) => {
                ended.map_err(|_| ReadRefused::Deadline)
            }
        };
        // A reset during the read leaves its answer to nobody.
        let outcome = if self.0.gate.generation() == generation {
            outcome
        } else {
            Err(ReadRefused::Cancelled)
        };
        {
            let mut state = guard(&self.0.state);
            state.running.remove(&request.key);
            if let Some(busy) = state.busy.get_mut(&request.lane) {
                *busy = busy.saturating_sub(1);
            }
            if let Some(sender) = state.answers.remove(&request.key) {
                sender.send_replace(Some(outcome));
            }
        }
        self.0.wake.notify_one();
    }

    async fn work(self, lane: Lane, first: Lease) {
        if lane == Lane::Moodle {
            return self.moodle_worker(first).await;
        }
        let mut lease = first;
        loop {
            let end = bounded(&lease, self.execute(&lease)).await;
            match self.finish(lease, end).await {
                Some(next) => lease = next,
                None => return,
            }
        }
    }

    /// LMS keys share one open transport while the lane has work.
    async fn moodle_worker(self, first: Lease) {
        let mut lease = first;
        let Some(readers) = self.school() else {
            loop {
                // No sessions: the planner's admission holds these keys.
                let end = KeyEnd::Blocked {
                    profile: SchoolAuthProfile::Science,
                    wait: AccountWait::Person,
                    version: Credentials::default().version,
                    source: String::new(),
                };
                match self.finish(lease, end).await {
                    Some(next) => lease = next,
                    None => return,
                }
            }
        };
        let mut reader = None;
        loop {
            // Boxed with an explicit `Send` bound, as the LMS reads inside
            // it: rustc's higher-ranked auto-trait inference rejects the
            // spawned worker otherwise.
            let read: crate::net::BoxFuture<'_, KeyEnd> =
                Box::pin(school::read_lms_key(&readers, &mut reader, &lease));
            let end = bounded(&lease, read).await;
            let keep = match &end {
                KeyEnd::Done { .. } | KeyEnd::NotEnrolled { .. } => true,
                KeyEnd::Failed { kind, .. } => kind != "timeout",
                _ => false,
            };
            if !keep {
                reader = None;
            }
            match self.finish(lease, end).await {
                Some(next) => lease = next,
                None => return,
            }
        }
    }

    async fn execute(&self, lease: &Lease) -> KeyEnd {
        let unattached = || KeyEnd::Blocked {
            profile: SchoolAuthProfile::TokyoTech,
            wait: AccountWait::Person,
            version: Credentials::default().version,
            source: String::new(),
        };
        match lease.key.dataset {
            Dataset::KyomuPage => match self.school() {
                Some(readers) => {
                    school::read_kyomu_page(&readers, self.0.events.get(), lease).await
                }
                None => unattached(),
            },
            Dataset::CatalogOfficial => match self.school() {
                Some(readers) => crate::catalog::read_official(&readers.sessions, lease).await,
                None => unattached(),
            },
            Dataset::Reference => {
                crate::reference_refresh::read_reference(lease, &self.0.probes).await
            }
            _ => KeyEnd::failed("unsupported", ""),
        }
    }

    /// Records how a read ended and leases the lane's next key, if any.
    async fn finish(&self, lease: Lease, end: KeyEnd) -> Option<Lease> {
        let lane = Lane::of(lease.key.dataset);
        let ending = lease.ending(&end);
        let after = tokio::task::spawn_blocking(move || ending.write())
            .await
            .ok()
            .flatten();
        let credentials = self.credentials();
        let next = {
            let mut state = guard(&self.0.state);
            settle(&mut state, &lease, &end, after, &credentials);
            // A queued one-off read takes the slot first (`pump`).
            let next = if state.reads.iter().any(|request| request.lane == lane) {
                None
            } else {
                self.lease_next(&mut state, lane, &credentials)
            };
            if next.is_none() {
                if let Some(busy) = state.busy.get_mut(&lane) {
                    *busy = busy.saturating_sub(1);
                }
            }
            next
        };
        self.0.wake.notify_one();
        next
    }

    fn publish(&self) {
        let snapshot = self.snapshot();
        let changed = {
            let mut state = guard(&self.0.state);
            if state.published.as_ref() == Some(&snapshot) {
                false
            } else {
                state.published = Some(snapshot.clone());
                true
            }
        };
        if changed {
            if let Some(events) = self.0.events.get() {
                let _ = events.emit(CoreEvent::SyncSnapshot(snapshot));
            }
        }
        self.publish_directive();
        self.0
            .version
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    /// The earliest deadline of the plan and every duty, and what an OS
    /// background opportunity would find due, sent when it changed.
    fn publish_directive(&self) {
        let now = Utc::now();
        let (plan, waiting, duties, failures) = {
            let state = guard(&self.0.state);
            let instant = Instant::now();
            // A failed run without a retry time leaves the snapshot then.
            let failures = state
                .batches
                .iter()
                .filter(|batch| {
                    batch.idle()
                        && batch.failed.values().all(|failed| failed.retry.is_none())
                        && !batch.failed.is_empty()
                })
                .filter_map(|batch| batch.finished_at)
                .map(|at| {
                    now + chrono::Duration::from_std(
                        FAILURE_SHOWN.saturating_sub(instant.saturating_duration_since(at)),
                    )
                    .unwrap_or_default()
                })
                .min();
            let waiting = state.plan.iter().any(|candidate| candidate.held.is_none())
                || !state.reads.is_empty();
            let duties = state
                .duties
                .iter()
                .map(|slot| (slot.duty.clone(), slot.running))
                .collect::<Vec<_>>();
            (state.plan_next, waiting, duties, failures)
        };
        // Work waiting only for a lane slot needs no timer (a read ending
        // wakes the engine), but an OS opportunity would find it due.
        let mut network = plan;
        let mut local: Option<DateTime<Utc>> = None;
        for (duty, running) in duties {
            let Some(at) = (!running).then(|| duty.next_due()).flatten() else {
                continue;
            };
            let slot = match duty.work() {
                BackgroundWork::Network => &mut network,
                BackgroundWork::Local => &mut local,
            };
            *slot = Some(slot.map_or(at, |current| current.min(at)));
        }
        // Whole seconds, rounded up: every shell parses them, and a timer
        // never fires before the work is due.
        let second = |at: DateTime<Utc>| {
            DateTime::from_timestamp(
                at.timestamp() + i64::from(at.timestamp_subsec_nanos() > 0),
                0,
            )
            .unwrap_or(at)
        };
        let network = network.map(second);
        let local = local.map(second);
        let at = [network, local, failures.map(second)]
            .into_iter()
            .flatten()
            .min();
        let background = [
            (
                BackgroundWork::Network,
                network
                    .into_iter()
                    .chain(waiting.then(|| second(now)))
                    .min(),
            ),
            (BackgroundWork::Local, local),
        ]
        .into_iter()
        .filter_map(|(work, earliest)| {
            earliest.map(|earliest| BackgroundRequest {
                work,
                earliest: earliest.max(second(now)),
            })
        })
        .collect::<Vec<_>>();
        let mut published = None;
        self.0.directive.send_if_modified(|current| {
            // Only a move of the deadline or of what is asked of the OS is
            // news; a past deadline renewed to now is the same one.
            let same_at = match (current.at, at) {
                (Some(current), Some(at)) => current == at || (current <= now && at <= now),
                (current, at) => current == at,
            };
            let same_background = current.background.len() == background.len()
                && current.background.iter().zip(&background).all(|(a, b)| {
                    a.work == b.work
                        && (a.earliest == b.earliest || (a.earliest <= now && b.earliest <= now))
                });
            if same_at && same_background {
                return false;
            }
            *current = WakeDirective {
                revision: current.revision.wrapping_add(1),
                at,
                background,
            };
            published = Some(current.clone());
            true
        });
        if let (Some(directive), Some(events)) = (published, self.0.events.get()) {
            let _ = events.emit(CoreEvent::WakeDirective(directive));
        }
    }
}

/// Which duties the facts allow: all of them in front or running, and in
/// the background what the opportunity allows.
fn allowed_work(state: &State) -> impl Fn(BackgroundWork) -> bool {
    let all = !state.facts.suspended;
    let local = !state.opportunities.is_empty();
    let network = state.opportunities.values().any(|network| *network);
    move |work| {
        all || match work {
            BackgroundWork::Local => local,
            BackgroundWork::Network => network,
        }
    }
}

/// Ends every one-off read: the queued ones answered, the running ones
/// cancelled (they answer themselves).
fn cancel_requests(state: &mut State) {
    for request in std::mem::take(&mut state.reads) {
        if let Some(sender) = state.answers.remove(&request.key) {
            sender.send_replace(Some(Err(ReadRefused::Cancelled)));
        }
    }
    for cancel in state.running.values() {
        cancel.cancel();
    }
}

/// Races a read against its lease's cancellation and its deadline.
async fn bounded(lease: &Lease, work: impl Future<Output = KeyEnd>) -> KeyEnd {
    tokio::select! {
        biased;
        _ = lease.cancel.cancelled() => KeyEnd::Cancelled { source: String::new() },
        ended = tokio::time::timeout_at(lease.deadline, work) => {
            ended.unwrap_or_else(|_| KeyEnd::failed("timeout", ""))
        }
    }
}

/// A refresh press for one key: joins a lease in flight or a press of the
/// last 30 s, else wants the key by hand.
fn request_manual(state: &mut State, key: Key, scope: Option<String>, at: Instant) {
    if state.leases.contains_key(&key) {
        if let Some(batch) = state
            .batches
            .iter_mut()
            .find(|batch| batch.running.contains_key(&key))
        {
            batch.priority = Priority::Interactive;
        }
        state.last_manual.insert(key, at);
        return;
    }
    if state
        .last_manual
        .get(&key)
        .is_some_and(|last| at.saturating_duration_since(*last) < MANUAL_JOIN)
    {
        return;
    }
    state.last_manual.insert(key.clone(), at);
    state.manual.insert(key, (at, scope));
}

/// The shell's network.
fn report_network(state: &mut State, network: Network) -> bool {
    let changed = state.facts.network != network;
    state.facts.network = network;
    changed
}

/// Folds one finished key into the facts, the cache and its run.
/// `credentials` are the facts now: a wait decided under an older version
/// of them is not kept.
fn settle(
    state: &mut State,
    lease: &Lease,
    end: &KeyEnd,
    after: Option<KeyState>,
    credentials: &Credentials,
) {
    let key = &lease.key;
    state.leases.remove(key);
    // The key's new freshness or backoff moves the plan's own deadline:
    // once the reads in flight are done, plan again.
    if state.leases.is_empty() {
        state.evaluate = true;
    }
    match after {
        Some(after) => {
            state.inputs.states.insert(key.clone(), after);
        }
        None => {
            if !matches!(end, KeyEnd::Stale) {
                state.inputs.states.remove(key);
            }
        }
    }
    let host = Lane::of(key.dataset).host();
    match end {
        KeyEnd::Done { .. } => {
            state.facts.paused.remove(&host);
            // Keys held for this one (`planner::prerequisite`) are due now.
            if state
                .plan
                .iter()
                .any(|candidate| planner::prerequisite(&candidate.key).as_ref() == Some(key))
            {
                state.evaluate = true;
            }
        }
        // Decided under the facts in force now: this key at this depth is
        // not leased again until they move. Other keys wait only by the
        // facts themselves.
        KeyEnd::Blocked {
            profile,
            wait,
            version,
            ..
        } if *version == credentials.version => {
            log::info!(target: "app_core::sync", "{key} ended waiting for {profile:?} ({wait:?}) at credential version {version}");
            state.facts.waited.insert(
                (key.clone(), lease.depth),
                Waited {
                    version: *version,
                    held: Held::Account {
                        profile: *profile,
                        wait: wait.clone(),
                    },
                },
            );
            state.evaluate = true;
        }
        // The facts moved since the sessions decided (a Connect or Retry
        // landed meanwhile): the verdict is stale, only its run is kept.
        KeyEnd::Blocked {
            profile, version, ..
        } => {
            log::info!(target: "app_core::sync", "{key} ended waiting for {profile:?} at credential version {version}, now {}; not held", credentials.version);
            state.evaluate = true;
        }
        KeyEnd::Offline { .. } => state.evaluate = true,
        // The stored course list changed under the read: plan from it again.
        KeyEnd::NotEnrolled { .. } => state.evaluate = true,
        KeyEnd::HostPaused { host, until, .. } => {
            state.facts.paused.insert(*host, *until);
            state.evaluate = true;
        }
        _ => {}
    }
    // A read that ended for a reason of its own uses up the button press;
    // a paused host, a cancel or a missing course leaves the key wanted.
    if matches!(
        end,
        KeyEnd::Done { .. } | KeyEnd::Failed { .. } | KeyEnd::Refused { .. } | KeyEnd::Stale
    ) {
        state.manual.remove(key);
    }
    // Someone waits for a read of this key that started late enough.
    if let Some(waiters) = state.waiters.remove(key) {
        let answer = match end {
            KeyEnd::Done { .. } => Some(Ok(())),
            KeyEnd::Failed { kind, .. } | KeyEnd::Refused { kind } => {
                Some(Err(ReadRefused::Failed(kind.clone())))
            }
            _ => None,
        };
        let mut left = Vec::new();
        // A waiter that stopped waiting wants no read.
        for waiter in waiters
            .into_iter()
            .filter(|waiter| !waiter.reply.is_closed())
        {
            match &answer {
                Some(answer) if lease.revision() >= waiter.revision => {
                    let _ = waiter.reply.send(answer.clone());
                }
                _ => left.push(waiter),
            }
        }
        if !left.is_empty() {
            // Still wanted by hand, past the join, until a read late
            // enough ends.
            let now = Instant::now();
            state.manual.insert(key.clone(), (now, None));
            state.waiters.insert(key.clone(), left);
            state.evaluate = true;
        }
    }
    if key.dataset == Dataset::LmsEnrolment
        && !matches!(end, KeyEnd::Cancelled { .. } | KeyEnd::Stale)
    {
        if let Some(at) = state.follow_lms.take() {
            state.requests.push((ManualTarget::LmsCurrentCourses, at));
        }
        // New courses bring new keys.
        state.evaluate = true;
    }
    let retry_at = state
        .inputs
        .states
        .get(key)
        .and_then(|state| state.next_eligible);
    let instant = Instant::now();
    if let Some(batch) = state
        .batches
        .iter_mut()
        .find(|batch| batch.run == lease.run && batch.open())
    {
        batch.running.remove(key);
        match end {
            KeyEnd::Done { changed } => {
                log::debug!(target: "app_core::sync", "{key} committed, {changed} rows changed");
                batch.done.insert(key.clone());
            }
            KeyEnd::Failed { kind, reason, .. } => {
                if let Some(reason) = reason {
                    log::warn!(target: "app_core::sync", "{key} failed: {reason}");
                }
                batch.failed.insert(
                    key.clone(),
                    FailedKey {
                        kind: kind.clone(),
                        retry: retry_at,
                        reason: reason.clone(),
                    },
                );
            }
            KeyEnd::Refused { kind } => {
                batch.failed.insert(
                    key.clone(),
                    FailedKey {
                        kind: kind.clone(),
                        retry: retry_at,
                        reason: None,
                    },
                );
            }
            // Still due: the next plan holds or reads it again.
            KeyEnd::Blocked { .. }
            | KeyEnd::Offline { .. }
            | KeyEnd::HostPaused { .. }
            | KeyEnd::Cancelled { .. }
            | KeyEnd::NotEnrolled { .. }
            | KeyEnd::Stale => {}
        }
        if batch.idle() {
            batch.finished_at = Some(instant);
        }
    }
    state
        .batches
        .retain(|batch| !(batch.idle() && batch.finished_at.is_some() && batch.failed.is_empty()));
}

/// Matches the new plan against the runs: due keys join their group's open
/// run (or start one), keys no longer due leave it, finished runs go once
/// their failures are retried.
fn update_batches(state: &mut State, instant: Instant, now: DateTime<Utc>) {
    let planned: HashMap<&Key, &Candidate> = state
        .plan
        .iter()
        .map(|candidate| (&candidate.key, candidate))
        .collect();
    for batch in state.batches.iter_mut().filter(|batch| batch.open()) {
        batch.pending.retain(|key| planned.contains_key(key));
    }
    for candidate in &state.plan {
        let open = state
            .batches
            .iter()
            .position(|batch| batch.open() && batch.group == candidate.group);
        let index = match open {
            Some(index) => index,
            None => {
                state
                    .batches
                    .retain(|batch| batch.group != candidate.group || batch.open());
                state.batches.push(Batch {
                    run: uuid::Uuid::new_v4().to_string(),
                    group: candidate.group.clone(),
                    priority: candidate.priority,
                    pending: BTreeSet::new(),
                    running: BTreeMap::new(),
                    done: BTreeSet::new(),
                    failed: BTreeMap::new(),
                    held: None,
                    finished_at: None,
                });
                state.batches.len() - 1
            }
        };
        let batch = &mut state.batches[index];
        batch.done.remove(&candidate.key);
        batch.failed.remove(&candidate.key);
        batch.pending.insert(candidate.key.clone());
        batch.priority = batch.priority.max(candidate.priority);
    }
    for batch in state.batches.iter_mut().filter(|batch| batch.open()) {
        let ready = batch.pending.iter().any(|key| {
            planned
                .get(key)
                .is_some_and(|candidate| candidate.held.is_none())
        });
        batch.held = if !batch.running.is_empty() || ready {
            None
        } else {
            batch.pending.iter().find_map(|key| {
                planned
                    .get(key)
                    .and_then(|candidate| candidate.held.clone())
            })
        };
        if batch.idle() {
            batch.finished_at = Some(instant);
        }
    }
    state.batches.retain(|batch| {
        if !batch.idle() {
            return true;
        }
        if batch.failed.is_empty() {
            return false;
        }
        let retry = batch
            .failed
            .values()
            .filter_map(|failed| failed.retry)
            .min();
        match retry {
            Some(retry) => retry > now,
            None => batch
                .finished_at
                .is_some_and(|at| instant.saturating_duration_since(at) < FAILURE_SHOWN),
        }
    });
}

/// Every stored fact one evaluation needs, from one transaction.
fn load_inputs(
    store: &DataStore,
    auth: &SchoolAuthService,
    now: DateTime<Utc>,
) -> Result<Inputs, DataError> {
    let workspace = store.load_fields(&[])?;
    let allow_cellular = workspace
        .pointer("/settings/subscriptionRefresh/allowCellular")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let english = crate::schedule::content_language(&workspace) == "en";
    let year = crate::reference_refresh::academic_year(&workspace)
        .and_then(|year| u16::try_from(year).ok());
    let science_configured = auth.configured(SchoolAuthProfile::Science);
    let tokyo_configured = auth.configured(SchoolAuthProfile::TokyoTech);
    let followed = crate::reference_refresh::followed(&workspace);
    store.write(None, |transaction| {
        let states = sync_state::list(transaction, None)?
            .into_iter()
            .filter_map(|state| {
                let dataset = Dataset::parse(&state.dataset)?;
                Some((Key::new(dataset, state.key.clone()), state))
            })
            .collect();
        let courses = crate::lms_ingest::stored_courses(transaction)
            .map_err(|error| DataError::Storage(error.to_string()))?;
        let mut references = Vec::new();
        for (kind, scope, index_url) in followed {
            let key = crate::reference_refresh::reference_key(&kind, scope);
            let (due, stale_at) = crate::reference_refresh::reference_due(
                transaction,
                &key,
                &kind,
                scope,
                &index_url,
                now,
            )?;
            references.push(ReferenceWatch {
                key,
                index_url,
                due,
                stale_at,
            });
        }
        Ok(Inputs {
            states,
            courses,
            science_configured,
            tokyo_configured,
            allow_cellular,
            year,
            english,
            references,
        })
    })
}

/// Drives the engine by hand: no driver task runs, so nothing reads the
/// network.
#[cfg(test)]
impl PlanEngine {
    /// Runs one evaluation now and returns the plan.
    pub(crate) async fn evaluate_now(&self) -> Vec<Candidate> {
        self.evaluate().await;
        guard(&self.0.state).plan.clone()
    }

    /// The commit gate an executor leased at `generation` writes through.
    pub(crate) fn writer_for_test(&self, generation: u64) -> LeaseWriter {
        LeaseWriter {
            store: self.0.store.clone(),
            gate: self.0.gate.clone(),
            generation,
            resource: None,
        }
    }

    /// A lease as `pump` would make it, without running it.
    pub(crate) fn lease_for_test(&self, lane: Lane) -> Option<Lease> {
        let credentials = self.credentials();
        let mut state = guard(&self.0.state);
        self.lease_next(&mut state, lane, &credentials)
    }

    /// Records and settles a read's ending as `finish` does, without
    /// leasing the lane's next key.
    pub(crate) async fn settle_for_test(&self, lease: Lease, end: KeyEnd) {
        let ending = lease.ending(&end);
        let after = tokio::task::spawn_blocking(move || ending.write())
            .await
            .ok()
            .flatten();
        let credentials = self.credentials();
        settle(&mut guard(&self.0.state), &lease, &end, after, &credentials);
    }

    /// `Hint::Refresh` without starting the driver.
    pub(crate) fn refresh_for_test(&self, scope: Scope) {
        guard(&self.0.state)
            .requests
            .push((ManualTarget::Scope(scope), Instant::now()));
    }
}
