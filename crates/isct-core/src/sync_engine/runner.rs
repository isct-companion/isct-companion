//! Leases, lanes, the commit gate and the driver that ties them together.
//!
//! One driver task evaluates the plan when a hint asks for it (at most
//! every 30 s on `Wake`) and fills each lane up to its budget. A worker
//! reads one leased key, records how it ended, and takes the next key of its
//! lane, so an LMS worker keeps its transport open across keys.
use super::{
    planner::{self, Candidate, Facts, Group, Inputs, Lane, PackWatch, ReferenceWatch},
    snapshot, Hint, Network, Priority, Scope, SyncSnapshot, View,
};
use crate::{
    data::{DataError, DataStore, WorkspaceChangeSource},
    events::CoreEvent,
    lifecycle::SchoolSite,
    net::Host,
    ports::EventSink,
    runtime_tasks::{RuntimeTaskError, RuntimeTasks},
    school_auth::{SchoolAuthProfile, SchoolAuthService},
    school_browser::{SchoolBrowserEngine, SchoolSessionState},
    school_service::IngestState,
    sync_state::{self, Dataset, Key, KeyState, RunOutcome, RunRecord},
};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// The longest one key may take, sign-in included. A key over it fails as
/// `timeout`; a reset waits at most this long for commits in progress.
pub const KEY_DEADLINE: Duration = Duration::from_secs(3 * 60);
/// A whole-department 教務 search is one GET and one 3 MB postback; a
/// catalog pack is a download and one import of a year's offerings.
const BULK_DEADLINE: Duration = Duration::from_secs(6 * 60);
const EVALUATE_EVERY: Duration = Duration::from_secs(30);
/// A second press of a refresh button within this time joins the first.
const MANUAL_JOIN: Duration = Duration::from_secs(30);
/// How long an opened page or a refresh press keeps its keys wanted while
/// they wait (offline, paused host).
const INTENT_LIFETIME: Duration = Duration::from_secs(10 * 60);
/// A failed run without a retry time stays visible this long.
const FAILURE_SHOWN: Duration = Duration::from_secs(30 * 60);

fn deadline(dataset: Dataset) -> Duration {
    if matches!(dataset, Dataset::CatalogOfficial | Dataset::CatalogSyllabus) {
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
}

/// Every write of a lease passes here. Closing admission bumps the
/// generation under the same lock, so a commit either entered before the
/// bump (and `wait_idle` waits for it) or is refused.
#[derive(Default)]
pub(crate) struct Gate(Mutex<GateState>);

impl Gate {
    fn generation(&self) -> u64 {
        guard(&self.0).generation
    }

    fn bump(&self) -> u64 {
        let mut state = guard(&self.0);
        state.generation = state.generation.saturating_add(1);
        state.generation
    }

    fn committing(&self) -> usize {
        guard(&self.0).committing
    }

    fn enter(self: &Arc<Self>, generation: u64) -> Option<GateGuard> {
        let mut state = guard(&self.0);
        if state.generation != generation {
            return None;
        }
        state.committing += 1;
        Some(GateGuard(self.clone()))
    }
}

/// Holds the gate for one commit; leaving it may let a reset proceed.
pub(crate) struct GateGuard(Arc<Gate>);

impl Drop for GateGuard {
    fn drop(&mut self) {
        let mut state = guard(&self.0 .0);
        state.committing = state.committing.saturating_sub(1);
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
}

impl LeaseWriter {
    pub(crate) fn store(&self) -> &DataStore {
        &self.store
    }

    /// Holds the gate while the caller writes through store methods that
    /// open their own transaction. `None`: the lease is stale.
    pub(crate) fn enter(&self) -> Option<GateGuard> {
        self.gate.enter(self.generation)
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
}

// -- Leases -------------------------------------------------------------------

/// The right to read one key, with what its reader needs.
pub(crate) struct Lease {
    pub(crate) key: Key,
    /// The account the key's content belongs to (`sync_state::account`).
    pub(crate) account: String,
    /// Correlates the `sync_runs` rows of one run in the snapshot.
    pub(crate) run: String,
    /// The Isct-Data index of a reference key.
    pub(crate) locator: Option<String>,
    pub(crate) started: DateTime<Utc>,
    pub(crate) cancel: CancellationToken,
    writer: LeaseWriter,
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
            account: &self.account,
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
    pub(crate) fn detached(store: &DataStore, key: Key, account: &str) -> Self {
        Self {
            key,
            account: account.into(),
            run: uuid::Uuid::new_v4().to_string(),
            locator: None,
            started: Utc::now(),
            cancel: CancellationToken::new(),
            writer: LeaseWriter {
                store: store.clone(),
                gate: Arc::new(Gate::default()),
                generation: 0,
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
    },
    /// Already recorded by the executor with its own backoff (a public file
    /// held back after shrinking).
    Refused {
        kind: String,
    },
    /// The session is gone; the profile's automatic keys wait for sign-in.
    SignInRequired {
        profile: SchoolAuthProfile,
        source: String,
    },
    /// The host's breaker paused it: every key of the host is held, and this
    /// key's own failure count stays.
    HostPaused {
        host: Host,
        until: DateTime<Utc>,
        notice: Option<String>,
        source: String,
    },
    Cancelled {
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
                log::warn!(target: "isct_core::sync", "commit failed: {error}");
                Self::failed(kind, source)
            }
        }
    }
}

/// What the runner records for a read that wrote no rows: the key's
/// backoff (unless the cause was not the key: sign-in, a paused host, a
/// cancel) and its `sync_runs` row, inside the lease's gate.
pub(crate) struct Ending {
    writer: LeaseWriter,
    key: Key,
    account: String,
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
                    sync_state::fail(transaction, &self.key, &self.account, kind, now)?;
                }
                sync_state::record_run(
                    transaction,
                    &RunRecord {
                        run: &self.run,
                        key: &self.key,
                        account: &self.account,
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
            | KeyEnd::SignInRequired { source, .. }
            | KeyEnd::HostPaused { source, .. }
            | KeyEnd::Cancelled { source } => source.clone(),
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
            // The reader already tried the saved sign-in once; backing off
            // keeps a session the school refuses from being retried on every
            // evaluation. A manual refresh still ignores the backoff.
            KeyEnd::SignInRequired { .. } => {
                Some((RunOutcome::SignInRequired, "sign-in".to_string(), true))
            }
            KeyEnd::HostPaused { .. } => {
                Some((RunOutcome::Failed, "host-paused".to_string(), false))
            }
            KeyEnd::Cancelled { .. } => {
                Some((RunOutcome::Cancelled, "cancelled".to_string(), false))
            }
        };
        Ending {
            writer: self.writer(),
            key: self.key.clone(),
            account: self.account.clone(),
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
        Dataset::KyomuPage => crate::school_parser::kyomu::page_url(&key.key)
            .and_then(|url| url::Url::parse(&url).ok())
            .map(|url| sync_state::source_of_url(&url))
            .unwrap_or_default(),
        Dataset::CatalogOfficial => format!(
            "{}{}",
            crate::school_parser::kyomu::HOST,
            crate::catalog::KYOMU_SEARCH_PATH
        ),
        Dataset::CatalogDetail => "syllabus".into(),
        Dataset::Reference => locator.unwrap_or_default().into(),
        Dataset::CatalogSyllabus => locator
            .unwrap_or(crate::reference_refresh::PUBLIC_INDEX)
            .into(),
    }
}

// -- State --------------------------------------------------------------------

struct LeaseEntry {
    cancel: CancellationToken,
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
    /// Failed keys: error kind and retry time.
    pub(super) failed: BTreeMap<Key, (String, Option<DateTime<Utc>>)>,
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

struct State {
    facts: Facts,
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
    sessions: HashMap<SchoolAuthProfile, String>,
    kyomu_notice: Option<String>,
    published: Option<SyncSnapshot>,
}

struct Shared {
    store: DataStore,
    auth: Arc<SchoolAuthService>,
    events: OnceLock<Arc<dyn EventSink>>,
    school: Mutex<Option<Arc<SchoolBrowserEngine>>>,
    state: Mutex<State>,
    gate: Arc<Gate>,
    wake: Notify,
    tasks: RuntimeTasks,
    runtime: OnceLock<tokio::runtime::Handle>,
    driving: AtomicBool,
    probes: Arc<crate::reference_refresh::IndexProbes>,
}

/// The sync planner and runner. Cheap to clone.
#[derive(Clone)]
pub struct SyncEngine(Arc<Shared>);

/// A weak handle for components the engine itself owns (the school
/// browser), so hints never keep the engine alive.
#[derive(Clone)]
pub struct SyncHints(Weak<Shared>);

impl SyncHints {
    /// Hints sent to nothing; for components built without an engine.
    pub fn detached() -> Self {
        Self(Weak::new())
    }

    pub fn send(&self, hint: Hint) {
        if let Some(shared) = self.0.upgrade() {
            SyncEngine(shared).hint(hint);
        }
    }

    /// The engine while something else still owns it.
    pub(crate) fn engine(&self) -> Option<SyncEngine> {
        self.0.upgrade().map(SyncEngine)
    }

    /// Folds a Core event (sign-in, maintenance) into the snapshot.
    pub fn observe(&self, event: &CoreEvent) {
        if let Some(shared) = self.0.upgrade() {
            SyncEngine(shared).observe(event);
        }
    }

    /// Whether the shell's app is in front; true without an engine.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub(crate) fn in_front(&self) -> bool {
        self.engine().is_none_or(|engine| engine.in_front())
    }
}

/// Admission stays closed while this lives; dropping it reopens.
pub struct AdmissionGuard {
    engine: SyncEngine,
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

fn session_name(state: SchoolSessionState) -> &'static str {
    match state {
        SchoolSessionState::Pending => "pending",
        SchoolSessionState::Active => "active",
        SchoolSessionState::Maintenance => "maintenance",
        SchoolSessionState::Failed => "failed",
        SchoolSessionState::NotConfigured => "signed-out",
    }
}

impl SyncEngine {
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
                sessions: HashMap::new(),
                kyomu_notice: None,
                published: None,
            }),
            gate: Arc::new(Gate::default()),
            wake: Notify::new(),
            tasks: RuntimeTasks::default(),
            runtime: OnceLock::new(),
            driving: AtomicBool::new(false),
            probes: Arc::new(crate::reference_refresh::IndexProbes::default()),
        }))
    }

    pub fn hints(&self) -> SyncHints {
        SyncHints(Arc::downgrade(&self.0))
    }

    /// Where snapshots go. Set once, by the facade.
    pub(crate) fn set_events(&self, events: Arc<dyn EventSink>) {
        let _ = self.0.events.set(events);
    }

    /// The school browser whose HTTP transport reads LMS and 教務Web.
    pub(crate) fn attach_school(&self, engine: Arc<SchoolBrowserEngine>) {
        *guard(&self.0.school) = Some(engine);
        guard(&self.0.state).evaluate = true;
        self.0.wake.notify_one();
    }

    /// Stored facts changed outside a key (saved credentials, settings):
    /// evaluate again on the next driver turn.
    pub(crate) fn reevaluate(&self) {
        guard(&self.0.state).evaluate = true;
        self.0.wake.notify_one();
    }

    fn school(&self) -> Option<Arc<SchoolBrowserEngine>> {
        guard(&self.0.school).clone()
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
                Hint::Wake => state
                    .last_evaluated
                    .is_none_or(|at| now.saturating_duration_since(at) >= EVALUATE_EVERY),
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
                Hint::Network { network } => {
                    let changed = state.facts.network != network;
                    state.facts.network = network;
                    changed
                }
                Hint::PowerSaver { on } => {
                    let changed = state.facts.power_saver != on;
                    state.facts.power_saver = on;
                    changed
                }
                Hint::SignedIn { profile } => {
                    state.facts.signed_in.insert(profile);
                    state.sessions.insert(profile, "active".into());
                    true
                }
                Hint::Open { view } => {
                    match &view {
                        View::Course { lms_course_id } => state.shown.extend(
                            [Dataset::LmsCourse, Dataset::LmsAssign]
                                .map(|dataset| Key::new(dataset, lms_course_id.to_string())),
                        ),
                        View::Offering { offering_id } => state
                            .shown
                            .push(Key::new(Dataset::CatalogDetail, offering_id.clone())),
                        _ => {}
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

    /// Folds the sign-in and maintenance facts of Core events into the
    /// sources; the facade's event relay calls this for every event.
    pub fn observe(&self, event: &CoreEvent) {
        let changed = {
            let mut state = guard(&self.0.state);
            match event {
                CoreEvent::SchoolSession(session) => {
                    state
                        .sessions
                        .insert(session.profile, session_name(session.state).into());
                    match session.state {
                        SchoolSessionState::Active => {
                            state.facts.signed_in.insert(session.profile);
                            state.evaluate = true;
                        }
                        SchoolSessionState::Failed | SchoolSessionState::NotConfigured => {
                            state.facts.signed_in.remove(&session.profile);
                            state.evaluate = true;
                        }
                        SchoolSessionState::Pending | SchoolSessionState::Maintenance => {}
                    }
                    true
                }
                CoreEvent::SchoolIngest(outcome)
                    if outcome.site == SchoolSite::Kyomu
                        && outcome.state == IngestState::Maintenance =>
                {
                    state.kyomu_notice = Some(outcome.message.clone());
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.0.wake.notify_one();
        }
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
            facts: &state.facts,
            inputs: &state.inputs,
            sessions: &state.sessions,
            kyomu_notice: state.kyomu_notice.as_deref(),
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
            for lease in state.leases.values() {
                lease.cancel.cancel();
            }
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

    /// Evaluates once more, then waits until no key is leased and none the
    /// facts allow is left to lease, at most `limit`. An OS background wake
    /// holds the process open this long.
    pub async fn wait_quiet(&self, limit: Duration) -> bool {
        let since = Instant::now();
        let deadline = since + limit;
        self.reevaluate();
        loop {
            if self.quiet(since) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(250))).await;
        }
    }

    fn quiet(&self, since: Instant) -> bool {
        let state = guard(&self.0.state);
        let now = Utc::now();
        !state.evaluate
            && state.last_evaluated.is_some_and(|at| at >= since)
            && state.leases.is_empty()
            && !state.plan.iter().any(|candidate| {
                candidate.held.is_none()
                    && planner::hold(
                        candidate.key.dataset,
                        candidate.priority,
                        &state.inputs,
                        &state.facts,
                        now,
                    )
                    .is_none()
            })
    }

    /// Waits until no commit is in progress, at most `limit`.
    pub async fn wait_idle(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if self.0.gate.committing() == 0 {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(20))).await;
        }
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
            self.publish();
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
        let school_attached = self.school().is_some();
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
        state.facts.school_attached = school_attached;
        state.facts.paused.retain(|_, until| *until > now);
        let opens: Vec<View> = state.opens.keys().cloned().collect();
        let manual: HashMap<Key, Option<String>> = state
            .manual
            .iter()
            .map(|(key, (_, scope))| (key.clone(), scope.clone()))
            .collect();
        let leased: HashSet<Key> = state.leases.keys().cloned().collect();
        state.plan = planner::plan(&inputs, &state.facts, &opens, &manual, &leased, now);
        state.inputs = inputs;
        update_batches(&mut state, instant, now);
    }

    /// Leases due keys into every lane with room.
    fn pump(&self) {
        let Some(handle) = self.0.runtime.get().cloned() else {
            return;
        };
        let mut started = Vec::new();
        {
            let mut state = guard(&self.0.state);
            if state.closed > 0 || state.stopped {
                return;
            }
            for lane in Lane::ALL {
                while state.busy.get(&lane).copied().unwrap_or(0) < lane.capacity() {
                    let Some(lease) = self.lease_next(&mut state, lane) else {
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

    /// The first ready key of `lane` in plan order, leased.
    fn lease_next(&self, state: &mut State, lane: Lane) -> Option<Lease> {
        if state.closed > 0 || state.stopped {
            return None;
        }
        let now = Utc::now();
        let index = state.plan.iter().position(|candidate| {
            candidate.lane == lane
                && candidate.held.is_none()
                && !state.leases.contains_key(&candidate.key)
                && planner::hold(
                    candidate.key.dataset,
                    candidate.priority,
                    &state.inputs,
                    &state.facts,
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
        state.leases.insert(
            candidate.key.clone(),
            LeaseEntry {
                cancel: cancel.clone(),
            },
        );
        Some(Lease {
            key: candidate.key,
            account: candidate.account,
            run,
            locator: candidate.locator,
            started: now,
            cancel,
            writer: LeaseWriter {
                store: self.0.store.clone(),
                gate: self.0.gate.clone(),
                generation: self.0.gate.generation(),
            },
        })
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
        let Some(engine) = self.school() else {
            loop {
                let end = KeyEnd::SignInRequired {
                    profile: SchoolAuthProfile::Science,
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
                Box::pin(engine.read_lms_key(&mut reader, &lease));
            let end = bounded(&lease, read).await;
            let keep = match &end {
                KeyEnd::Done { .. } => true,
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
        match lease.key.dataset {
            Dataset::KyomuPage => match self.school() {
                Some(engine) => engine.read_kyomu_page(lease).await,
                None => KeyEnd::SignInRequired {
                    profile: SchoolAuthProfile::TokyoTech,
                    source: String::new(),
                },
            },
            Dataset::CatalogOfficial => {
                match self.school().and_then(|engine| engine.kyomu_client().ok()) {
                    Some(client) => crate::catalog::read_official(&client, lease).await,
                    None => KeyEnd::SignInRequired {
                        profile: SchoolAuthProfile::TokyoTech,
                        source: String::new(),
                    },
                }
            }
            Dataset::CatalogDetail => crate::catalog::read_detail(lease).await,
            Dataset::Reference => {
                crate::reference_refresh::read_reference(lease, &self.0.probes).await
            }
            Dataset::CatalogSyllabus => crate::catalog_pack::read_pack(lease).await,
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
        let next = {
            let mut state = guard(&self.0.state);
            settle(&mut state, &lease, &end, after);
            let next = self.lease_next(&mut state, lane);
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
        {
            let mut state = guard(&self.0.state);
            if state.published.as_ref() == Some(&snapshot) {
                return;
            }
            state.published = Some(snapshot.clone());
        }
        if let Some(events) = self.0.events.get() {
            let _ = events.emit(CoreEvent::SyncSnapshot(snapshot));
        }
    }
}

/// Races a read against its lease's cancellation and its deadline.
async fn bounded(lease: &Lease, work: impl Future<Output = KeyEnd>) -> KeyEnd {
    tokio::select! {
        biased;
        _ = lease.cancel.cancelled() => KeyEnd::Cancelled { source: String::new() },
        ended = tokio::time::timeout(deadline(lease.key.dataset), work) => {
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

/// Folds one finished key into the facts, the cache and its run.
fn settle(state: &mut State, lease: &Lease, end: &KeyEnd, after: Option<KeyState>) {
    let key = &lease.key;
    state.leases.remove(key);
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
            // Its assignment states wait for the outline (planner rule).
            if key.dataset == Dataset::LmsCourse {
                state.evaluate = true;
            }
            if host == Host::Kyomu {
                state.kyomu_notice = None;
            }
        }
        KeyEnd::SignInRequired { profile, .. } => {
            // Every other key of the profile would fail the same way.
            state.facts.signed_in.remove(profile);
            state
                .plan
                .retain(|candidate| planner::profile_of(candidate.key.dataset) != Some(*profile));
            state.evaluate = true;
        }
        KeyEnd::HostPaused {
            host,
            until,
            notice,
            ..
        } => {
            state.facts.paused.insert(*host, *until);
            if *host == Host::Kyomu {
                if let Some(notice) = notice {
                    state.kyomu_notice = Some(notice.clone());
                }
            }
            state.evaluate = true;
        }
        _ => {}
    }
    // A read that ended for a reason of its own uses up the button press;
    // a paused host or a cancel leaves the key wanted.
    if matches!(
        end,
        KeyEnd::Done { .. }
            | KeyEnd::Failed { .. }
            | KeyEnd::Refused { .. }
            | KeyEnd::SignInRequired { .. }
            | KeyEnd::Stale
    ) {
        state.manual.remove(key);
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
                log::debug!(target: "isct_core::sync", "{key} committed, {changed} rows changed");
                batch.done.insert(key.clone());
            }
            KeyEnd::Failed { kind, .. } | KeyEnd::Refused { kind } => {
                batch.failed.insert(key.clone(), (kind.clone(), retry_at));
            }
            KeyEnd::SignInRequired { .. } => {
                batch.failed.insert(key.clone(), ("sign-in".into(), None));
            }
            // Still due: the next plan holds or reads it again.
            KeyEnd::HostPaused { .. } | KeyEnd::Cancelled { .. } | KeyEnd::Stale => {}
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
        let retry = batch.failed.values().filter_map(|(_, at)| *at).min();
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
    let workspace = store.load_workspace()?;
    let allow_cellular = workspace
        .pointer("/settings/subscriptionRefresh/allowCellular")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let year = crate::reference_refresh::subscription_year(&workspace)
        .and_then(|year| u16::try_from(year).ok());
    let account = |profile: SchoolAuthProfile| match auth.sync_account(profile) {
        Ok(account) => Some(account),
        Err(error) => {
            crate::diagnostics::report("sync", &error);
            None
        }
    };
    let science = account(SchoolAuthProfile::Science);
    let tokyo = account(SchoolAuthProfile::TokyoTech);
    // `sync_state::account` of an empty username ends with the colon.
    let configured = |account: &Option<String>| {
        account
            .as_deref()
            .is_some_and(|account| !account.ends_with(':'))
    };
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
        let mut statement = transaction.prepare(
            "SELECT lms_id, json_extract(value,'$.offeringId')
             FROM lms_courses WHERE missing_since IS NULL",
        )?;
        let offerings = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(id, offering)| Some((u64::try_from(id?).ok()?, offering?)))
            .collect();
        let mut references = Vec::new();
        for (kind, year, index_url) in followed {
            let key = crate::reference_refresh::reference_key(&kind, year);
            let due = crate::reference_refresh::reference_due(
                transaction,
                &key,
                &kind,
                year,
                &index_url,
                now,
            )?;
            references.push(ReferenceWatch {
                key,
                index_url,
                due,
            });
        }
        let mut years = crate::catalog_pack::watched_years(now);
        if let Some(viewed) = year.filter(|viewed| !years.contains(viewed)) {
            years.push(viewed);
        }
        let packs = years
            .into_iter()
            .map(|year| {
                let key = crate::catalog_pack::pack_key(year);
                let due = crate::catalog_pack::pack_due(transaction, &key, now)?;
                Ok(PackWatch { key, due })
            })
            .collect::<Result<Vec<_>, DataError>>()?;
        Ok(Inputs {
            states,
            courses,
            offerings,
            science_configured: configured(&science),
            tokyo_configured: configured(&tokyo),
            science,
            tokyo,
            allow_cellular,
            year,
            references,
            packs,
        })
    })
}

/// Drives the engine by hand: no driver task runs, so nothing reads the
/// network.
#[cfg(test)]
impl SyncEngine {
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
        }
    }

    /// A lease as `pump` would make it, without running it.
    pub(crate) fn lease_for_test(&self, lane: Lane) -> Option<Lease> {
        let mut state = guard(&self.0.state);
        self.lease_next(&mut state, lane)
    }

    /// Records and settles a read's ending as `finish` does, without
    /// leasing the lane's next key.
    pub(crate) async fn settle_for_test(&self, lease: Lease, end: KeyEnd) {
        let ending = lease.ending(&end);
        let after = tokio::task::spawn_blocking(move || ending.write())
            .await
            .ok()
            .flatten();
        settle(&mut guard(&self.0.state), &lease, &end, after);
    }

    /// `Hint::Refresh` without starting the driver.
    pub(crate) fn refresh_for_test(&self, scope: Scope) {
        guard(&self.0.state)
            .requests
            .push((ManualTarget::Scope(scope), Instant::now()));
    }

    pub(crate) fn set_signed_in(&self, profile: SchoolAuthProfile) {
        guard(&self.0.state).facts.signed_in.insert(profile);
    }
}
