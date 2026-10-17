//! Shared scheduling for reference and school data pulls.
//!
//! The coordinator owns business-level admission, coalescing and cancellation.
//! School login and WebView navigation remain a separate executor state machine.

mod queue;
use queue::*;

use crate::{lifecycle::SchoolSite, runtime_tasks::RuntimeTasks};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{oneshot, watch, Mutex};
use url::Url;

/// Stable categories at the service boundary; desktop adapters retain Display text.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PullError {
    #[error("{0}")]
    InvalidTarget(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Authentication(String),
    #[error("{0}")]
    Maintenance(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    ServiceStopped(String),
    #[error("{0}")]
    SiteChanged(String),
    #[error("Pull cancelled.")]
    Cancelled,
    #[error("Pull coordinator stopped unexpectedly.")]
    CoordinatorStopped,
    #[error("The async runtime is unavailable.")]
    RuntimeUnavailable,
    #[error("Pull executor panicked.")]
    ExecutorPanicked,
    #[error("Pull executor was cancelled unexpectedly.")]
    ExecutorStopped,
    #[error("{0}")]
    Execution(String),
    #[error("Pull coordinator invariant failed: {0}")]
    Invariant(String),
}

type PullFuture = Pin<Box<dyn Future<Output = Result<Value, PullError>> + Send>>;
type PullWork = Arc<dyn Fn(PullContext) -> PullFuture + Send + Sync>;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PullTarget {
    Catalog {
        year: u16,
        query: String,
    },
    School {
        site: SchoolSite,
        course_id: Option<u64>,
    },
    Reference {
        kind: String,
        index_url: String,
        year: i32,
    },
}

impl PullTarget {
    pub fn normalized(self) -> Result<Self, PullError> {
        match self {
            Self::Catalog { year, query } => {
                if !(2020..=2200).contains(&year) || query.len() > 4096 {
                    return Err(PullError::InvalidTarget("Invalid course search.".into()));
                }
                Ok(Self::Catalog { year, query })
            }
            Self::Reference {
                kind,
                index_url,
                year,
            } => {
                let mut url = Url::parse(index_url.trim())
                    .map_err(|_| PullError::InvalidTarget("Pull index URL is invalid.".into()))?;
                if url.scheme() != "https"
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(PullError::InvalidTarget(
                        "Pull index URL must be HTTPS.".into(),
                    ));
                }
                if kind.trim().is_empty() || !(2000..=2200).contains(&year) {
                    return Err(PullError::InvalidTarget(
                        "Pull reference kind or year is invalid.".into(),
                    ));
                }
                url.set_fragment(None);
                Ok(Self::Reference {
                    kind: kind.trim().to_ascii_lowercase(),
                    index_url: url.to_string(),
                    year,
                })
            }
            Self::School { site, course_id } => {
                if !matches!(site, SchoolSite::ScienceLms | SchoolSite::Kyomu) {
                    return Err(PullError::InvalidTarget(
                        "Only LMS and 教務Web can be pulled.".into(),
                    ));
                }
                if course_id == Some(0) {
                    return Err(PullError::InvalidTarget(
                        "LMS course ID must be positive.".into(),
                    ));
                }
                if site == SchoolSite::Kyomu && course_id.is_some() {
                    return Err(PullError::InvalidTarget(
                        "教務Web does not accept an LMS course ID.".into(),
                    ));
                }
                Ok(Self::School { site, course_id })
            }
        }
    }

    fn lane(&self) -> PullLane {
        match self {
            Self::School { .. } | Self::Catalog { .. } => PullLane::School,
            Self::Reference { .. } => PullLane::Http,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PullPriority {
    Automatic,
    Manual,
}

impl PullPriority {
    fn encoded(self) -> u8 {
        match self {
            Self::Automatic => 0,
            Self::Manual => 1,
        }
    }

    fn from_encoded(value: u8) -> Self {
        if value == Self::Manual.encoded() {
            Self::Manual
        } else {
            Self::Automatic
        }
    }
}

#[derive(Clone, Debug)]
pub struct PullOptions {
    pub priority: PullPriority,
    pub delay: Duration,
    pub retry_limit: u32,
    /// School pulls sign in with saved credentials before extracting. A
    /// joining caller that asks for this upgrades the live job.
    pub login_first: bool,
}

impl PullOptions {
    pub fn manual() -> Self {
        Self {
            priority: PullPriority::Manual,
            delay: Duration::ZERO,
            retry_limit: 0,
            login_first: false,
        }
    }

    pub fn automatic() -> Self {
        Self {
            priority: PullPriority::Automatic,
            delay: Duration::ZERO,
            retry_limit: 0,
            login_first: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PullNetwork {
    Unknown,
    Offline,
    Wifi,
    Cellular,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullConditions {
    pub network: PullNetwork,
    pub foreground: bool,
    pub school_visible: bool,
    pub allow_cellular: bool,
}

impl Default for PullConditions {
    fn default() -> Self {
        Self {
            network: PullNetwork::Unknown,
            foreground: true,
            school_visible: false,
            allow_cellular: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PullState {
    Queued,
    Checking,
    Blocked,
    Running,
    Applying,
    RetryWaiting,
    Succeeded,
    Failed,
    Cancelling,
    Cancelled,
    Yielding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PullEvent {
    // Reducer contract only: the runtime creates a fresh job for each enqueue.
    #[cfg_attr(not(test), allow(dead_code))]
    Enqueue,
    Check,
    Block,
    Ready,
    Start,
    Progress,
    Apply,
    /// One commit finished; a multi-page run leaves its write gate.
    Applied,
    WaitRetry,
    Retry,
    Succeed,
    Fail,
    Cancel,
    Stop,
    // Reserved until browser preemption migrates from the school executor.
    #[cfg_attr(not(test), allow(dead_code))]
    Yield,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PullTransition {
    state: PullState,
    accepted: bool,
    diagnostic: &'static str,
}

fn accepted(state: PullState) -> PullTransition {
    PullTransition {
        state,
        accepted: true,
        diagnostic: "accepted",
    }
}

fn rejected(state: PullState) -> PullTransition {
    PullTransition {
        state,
        accepted: false,
        diagnostic: "event is not valid in the current pull state",
    }
}

/// Total finite control transition. Every state/event pair is explicit; runtime
/// metadata such as run IDs is checked before an event reaches this reducer.
fn reduce(state: PullState, event: PullEvent) -> PullTransition {
    use PullEvent::*;
    use PullState::*;
    match state {
        Queued => match event {
            Enqueue => accepted(Queued),
            Check => accepted(Checking),
            Block => accepted(Blocked),
            Ready => accepted(Queued),
            Start => accepted(Running),
            Cancel => accepted(Cancelled),
            Yield => accepted(Yielding),
            Progress | Apply | Applied | WaitRetry | Retry | Succeed | Fail | Stop => {
                rejected(Queued)
            }
        },
        Checking => match event {
            Block => accepted(Blocked),
            Ready => accepted(Queued),
            Cancel => accepted(Cancelled),
            Enqueue | Check | Start | Progress | Apply | Applied | WaitRetry | Retry | Succeed
            | Fail | Stop | Yield => rejected(Checking),
        },
        Blocked => match event {
            Enqueue => accepted(Blocked),
            Check => accepted(Checking),
            Ready => accepted(Queued),
            Cancel => accepted(Cancelled),
            Start | Progress | Apply | Applied | WaitRetry | Retry | Succeed | Fail | Stop
            | Yield | Block => rejected(Blocked),
        },
        Running => match event {
            Progress => accepted(Running),
            Apply => accepted(Applying),
            WaitRetry => accepted(RetryWaiting),
            Succeed => accepted(Succeeded),
            Fail => accepted(Failed),
            Cancel => accepted(Cancelling),
            Yield => accepted(Yielding),
            Enqueue | Check | Block | Ready | Start | Applied | Retry | Stop => rejected(Running),
        },
        Applying => match event {
            Progress => accepted(Applying),
            Apply => accepted(Applying),
            Applied => accepted(Running),
            WaitRetry => accepted(RetryWaiting),
            Succeed => accepted(Succeeded),
            Fail => accepted(Failed),
            Enqueue | Check | Block | Ready | Start | Retry | Cancel | Stop | Yield => {
                rejected(Applying)
            }
        },
        RetryWaiting => match event {
            Enqueue => accepted(RetryWaiting),
            Retry => accepted(Queued),
            Cancel => accepted(Cancelled),
            Check | Block | Ready | Start | Progress | Apply | Applied | WaitRetry | Succeed
            | Fail | Stop | Yield => rejected(RetryWaiting),
        },
        Succeeded => match event {
            Enqueue => accepted(Queued),
            Check | Block | Ready | Start | Progress | Apply | Applied | WaitRetry | Retry
            | Succeed | Fail | Cancel | Stop | Yield => rejected(Succeeded),
        },
        Failed => match event {
            Enqueue => accepted(Queued),
            Retry => accepted(Queued),
            Cancel => accepted(Cancelled),
            Check | Block | Ready | Start | Progress | Apply | Applied | WaitRetry | Succeed
            | Fail | Stop | Yield => rejected(Failed),
        },
        Cancelling => match event {
            Stop | Succeed | Fail => accepted(Cancelled),
            Cancel => accepted(Cancelling),
            Enqueue | Check | Block | Ready | Start | Progress | Apply | Applied | WaitRetry
            | Retry | Yield => rejected(Cancelling),
        },
        Cancelled => match event {
            Enqueue => accepted(Queued),
            Check | Block | Ready | Start | Progress | Apply | Applied | WaitRetry | Retry
            | Succeed | Fail | Cancel | Stop | Yield => rejected(Cancelled),
        },
        Yielding => match event {
            Ready => accepted(Queued),
            Cancel => accepted(Cancelled),
            Stop => accepted(Yielding),
            Enqueue | Check | Block | Start | Progress | Apply | Applied | WaitRetry | Retry
            | Succeed | Fail | Yield => rejected(Yielding),
        },
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullSnapshot {
    pub id: String,
    pub target: PullTarget,
    pub priority: PullPriority,
    pub state: PullState,
    pub phase: Option<String>,
    pub blocked_by: Option<String>,
    pub attempt: u32,
    pub waiter_count: usize,
    pub last_error: Option<String>,
}

#[derive(Clone)]
pub struct PullContext {
    coordinator: PullCoordinator,
    id: String,
    run_id: u64,
    cancelled: Arc<AtomicBool>,
    priority: Arc<AtomicU8>,
    login_first: Arc<AtomicBool>,
}

impl PullContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Current priority. A manual join can promote a job after it started,
    /// so executors read this again at every admission point.
    pub fn priority(&self) -> PullPriority {
        PullPriority::from_encoded(self.priority.load(Ordering::Acquire))
    }

    /// Whether any caller of this job asked to sign in first. Like the
    /// priority, a join can set this after the job started.
    pub fn login_first(&self) -> bool {
        self.login_first.load(Ordering::Acquire)
    }

    pub async fn phase(&self, phase: impl Into<String>) -> bool {
        self.coordinator
            .progress(&self.id, self.run_id, phase.into(), false)
            .await
    }

    pub async fn applying(&self) -> bool {
        self.coordinator
            .progress(&self.id, self.run_id, "applying".into(), true)
            .await
    }

    /// Leaves the write gate after one commit, so a multi-page executor can
    /// be cancelled again before its next page. Returns `false` when the run
    /// is no longer current or shutdown cancelled it during the commit.
    pub async fn applied(&self) -> bool {
        self.coordinator.applied(&self.id, self.run_id).await
    }

    /// Records a business-level partial failure without discarding a useful
    /// payload. The run settles its callers with `Ok`, while its snapshot ends
    /// in `Failed` so the UI and later automatic pulls can surface the issue.
    pub async fn outcome_error(&self, message: impl Into<String>) -> bool {
        self.coordinator
            .outcome_error(&self.id, self.run_id, message.into())
            .await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PullLane {
    School,
    Http,
}

struct PullJob {
    id: String,
    target: PullTarget,
    priority: PullPriority,
    priority_cell: Arc<AtomicU8>,
    login_first: Arc<AtomicBool>,
    state: PullState,
    phase: Option<String>,
    blocked_by: Option<String>,
    sequence: u64,
    run_id: u64,
    attempt: u32,
    retry_limit: u32,
    delay: Duration,
    ready: bool,
    last_error: Option<String>,
    cancel: Arc<AtomicBool>,
    work: Option<PullWork>,
    wait_for: Option<String>,
    waiters: Vec<oneshot::Sender<Result<Value, PullError>>>,
}

impl PullJob {
    fn snapshot(&self) -> PullSnapshot {
        PullSnapshot {
            id: self.id.clone(),
            target: self.target.clone(),
            priority: self.priority,
            state: self.state,
            phase: self.phase.clone(),
            blocked_by: self.blocked_by.clone(),
            attempt: self.attempt,
            waiter_count: self.waiters.len(),
            last_error: self.last_error.clone(),
        }
    }

    fn live(&self) -> bool {
        !matches!(
            self.state,
            PullState::Succeeded | PullState::Failed | PullState::Cancelled
        )
    }

    fn coalescible(&self) -> bool {
        self.live() && self.state != PullState::Cancelling
    }
}

struct CoordinatorState {
    jobs: HashMap<String, PullJob>,
    keys: HashMap<PullTarget, String>,
    queue: VecDeque<String>,
    next_sequence: u64,
    next_run: u64,
    school_running: bool,
    http_running: usize,
    conditions: PullConditions,
    driving: bool,
    closed: bool,
    suspended: bool,
}

struct PullShared {
    state: Mutex<CoordinatorState>,
    next_id: AtomicU64,
    snapshots: watch::Sender<Vec<PullSnapshot>>,
    tasks: RuntimeTasks,
}

#[derive(Clone)]
pub struct PullCoordinator(Arc<PullShared>);

impl Default for PullCoordinator {
    fn default() -> Self {
        let (snapshots, _) = watch::channel(Vec::new());
        Self(Arc::new(PullShared {
            state: Mutex::new(CoordinatorState {
                jobs: HashMap::new(),
                keys: HashMap::new(),
                queue: VecDeque::new(),
                next_sequence: 0,
                next_run: 0,
                school_running: false,
                http_running: 0,
                conditions: PullConditions::default(),
                driving: false,
                closed: false,
                suspended: false,
            }),
            next_id: AtomicU64::new(1),
            snapshots,
            tasks: RuntimeTasks::default(),
        }))
    }
}

impl PullCoordinator {
    /// OS suspension stops new execution in either lane. Foreground/focus is
    /// a separate school policy fact and does not suspend manual HTTP work.
    pub async fn suspend(&self) {
        let mut state = self.0.state.lock().await;
        state.suspended = true;
        publish_locked(&self.0, &state);
    }

    pub async fn resume(&self) {
        {
            let mut state = self.0.state.lock().await;
            state.suspended = false;
        }
        self.kick();
    }

    /// Stop admission and retries, cancel work before Applying, and drain the
    /// executors. Applying retains its write gate until the commit completes;
    /// a run that then leaves Applying for another page is cancelled.
    pub async fn shutdown(&self) {
        let mut waiters = Vec::new();
        {
            let mut state = self.0.state.lock().await;
            state.closed = true;
            for job in state.jobs.values_mut() {
                match job.state {
                    PullState::Applying => (),
                    PullState::Running | PullState::Cancelling => {
                        job.cancel.store(true, Ordering::Release);
                        job.state = PullState::Cancelling;
                    }
                    _ if job.live() => {
                        job.cancel.store(true, Ordering::Release);
                        job.state = reduce(job.state, PullEvent::Cancel).state;
                        job.work = None;
                        waiters.append(&mut job.waiters);
                    }
                    _ => (),
                }
            }
            state.queue.clear();
            state.keys.clear();
            publish_locked(&self.0, &state);
            self.0.tasks.close();
        }
        settle_waiters(waiters, Err(PullError::Cancelled));
        self.0.tasks.wait().await;
    }

    pub fn subscribe(&self) -> watch::Receiver<Vec<PullSnapshot>> {
        self.0.snapshots.subscribe()
    }

    pub async fn snapshots(&self) -> Vec<PullSnapshot> {
        let state = self.0.state.lock().await;
        snapshots_locked(&state)
    }

    pub async fn set_conditions(&self, conditions: PullConditions) {
        {
            let mut state = self.0.state.lock().await;
            if state.conditions == conditions {
                return;
            }
            state.conditions = conditions;
            publish_locked(&self.0, &state);
        }
        self.kick();
    }

    pub async fn run<F, Fut>(
        &self,
        target: PullTarget,
        options: PullOptions,
        work: F,
    ) -> Result<Value, PullError>
    where
        F: Fn(PullContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, PullError>> + Send + 'static,
    {
        tokio::runtime::Handle::try_current().map_err(|_| PullError::RuntimeUnavailable)?;
        let target = target.normalized()?;
        let work: PullWork = Arc::new(move |context| Box::pin(work(context)));
        let (sender, receiver) = oneshot::channel();
        let mut wake_after = None;
        {
            let mut state = self.0.state.lock().await;
            if state.closed {
                return Err(PullError::CoordinatorStopped);
            }
            if let Some(existing) = covering_job(&state, &target) {
                let requeue = {
                    let job = state
                        .jobs
                        .get_mut(&existing)
                        .ok_or_else(|| PullError::Invariant("live pull key".into()))?;
                    job.waiters.push(sender);
                    promote(job, &options);
                    if options.priority == PullPriority::Manual
                        && job.state == PullState::RetryWaiting
                    {
                        job.state = reduce(job.state, PullEvent::Retry).state;
                        job.ready = true;
                        true
                    } else {
                        if options.priority == PullPriority::Manual {
                            job.ready = true;
                        }
                        false
                    }
                };
                if requeue && !state.queue.contains(&existing) {
                    state.queue.push_back(existing);
                }
                publish_locked(&self.0, &state);
            } else {
                let id = format!("pull-{}", self.0.next_id.fetch_add(1, Ordering::Relaxed));
                let wait_for = state.keys.get(&target).and_then(|current| {
                    state
                        .jobs
                        .get(current)
                        .is_some_and(|job| job.state == PullState::Cancelling)
                        .then(|| current.clone())
                });
                state.next_sequence += 1;
                let sequence = state.next_sequence;
                let delayed = !options.delay.is_zero();
                state.queue.push_back(id.clone());
                state.keys.insert(target.clone(), id.clone());
                state.jobs.insert(
                    id.clone(),
                    PullJob {
                        id: id.clone(),
                        target,
                        priority: options.priority,
                        priority_cell: Arc::new(AtomicU8::new(options.priority.encoded())),
                        login_first: Arc::new(AtomicBool::new(options.login_first)),
                        state: PullState::Queued,
                        phase: None,
                        blocked_by: None,
                        sequence,
                        run_id: 0,
                        attempt: 0,
                        retry_limit: options.retry_limit,
                        delay: options.delay,
                        ready: !delayed,
                        last_error: None,
                        cancel: Arc::new(AtomicBool::new(false)),
                        work: Some(work),
                        wait_for,
                        waiters: vec![sender],
                    },
                );
                prune_terminal(&mut state);
                if delayed {
                    wake_after = Some((id, options.delay));
                }
                publish_locked(&self.0, &state);
            }
        }
        if let Some((id, delay)) = wake_after {
            self.wake_later(id, delay, false);
        }
        self.kick();
        receiver.await.unwrap_or(Err(PullError::CoordinatorStopped))
    }

    pub async fn cancel(&self, id: &str) -> bool {
        let mut settle = None;
        let changed = {
            let mut state = self.0.state.lock().await;
            let Some(job) = state.jobs.get_mut(id) else {
                return false;
            };
            let changed = match job.state {
                PullState::Queued
                | PullState::Checking
                | PullState::Blocked
                | PullState::RetryWaiting
                | PullState::Yielding => {
                    job.state = reduce(job.state, PullEvent::Cancel).state;
                    job.cancel.store(true, Ordering::Release);
                    settle = Some(std::mem::take(&mut job.waiters));
                    job.work = None;
                    let target = job.target.clone();
                    remove_key_if_current(&mut state, &target, id);
                    state.queue.retain(|queued| queued != id);
                    true
                }
                PullState::Running => {
                    job.state = reduce(job.state, PullEvent::Cancel).state;
                    job.cancel.store(true, Ordering::Release);
                    true
                }
                PullState::Applying => false,
                PullState::Cancelling => true,
                PullState::Succeeded | PullState::Failed | PullState::Cancelled => false,
            };
            prune_terminal(&mut state);
            publish_locked(&self.0, &state);
            changed
        };
        if let Some(waiters) = settle {
            settle_waiters(waiters, Err(PullError::Cancelled));
        }
        if changed {
            self.kick();
        }
        changed
    }

    fn kick(&self) {
        let coordinator = self.clone();
        self.0.tasks.spawn(async move { coordinator.drive().await });
    }

    fn wake_later(&self, id: String, delay: Duration, retry: bool) {
        let coordinator = self.clone();
        self.0.tasks.spawn(async move {
            tokio::select! {
                _ = coordinator.0.tasks.closed() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            {
                let mut state = coordinator.0.state.lock().await;
                if state.closed {
                    return;
                }
                let requeue = {
                    let Some(job) = state.jobs.get_mut(&id) else {
                        return;
                    };
                    if retry {
                        if job.state != PullState::RetryWaiting {
                            return;
                        }
                        job.state = reduce(job.state, PullEvent::Retry).state;
                    }
                    job.ready = true;
                    retry
                };
                if requeue {
                    state.queue.push_back(id.clone());
                }
                publish_locked(&coordinator.0, &state);
            }
            coordinator.kick();
        });
    }

    async fn drive(&self) {
        {
            let mut state = self.0.state.lock().await;
            if state.driving || state.closed {
                return;
            }
            state.driving = true;
        }
        loop {
            {
                let mut state = self.0.state.lock().await;
                let selected = select_next(&mut state);
                let Some(id) = selected else {
                    state.driving = false;
                    publish_locked(&self.0, &state);
                    return;
                };
                state.queue.retain(|queued| queued != &id);
                state.next_run += 1;
                let run_id = state.next_run;
                let (lane, work, context) = {
                    let Some(job) = state.jobs.get_mut(&id) else {
                        // A stale queue entry cannot own a lane or a caller.
                        continue;
                    };
                    job.state = reduce(job.state, PullEvent::Start).state;
                    job.blocked_by = None;
                    job.phase = Some("running".into());
                    job.run_id = run_id;
                    job.attempt += 1;
                    job.last_error = None;
                    job.cancel.store(false, Ordering::Release);
                    let lane = job.target.lane();
                    let Some(work) = job.work.clone() else {
                        continue;
                    };
                    let context = PullContext {
                        coordinator: self.clone(),
                        id: id.clone(),
                        run_id,
                        cancelled: job.cancel.clone(),
                        priority: job.priority_cell.clone(),
                        login_first: job.login_first.clone(),
                    };
                    (lane, work, context)
                };
                occupy_lane(&mut state, lane);
                publish_locked(&self.0, &state);
                // Register the supervisor while holding the admission lock:
                // shutdown cannot close task tracking between selection and spawn.
                let coordinator = self.clone();
                self.0.tasks.spawn(async move {
                    let worker = tokio::spawn(async move { work(context).await });
                    let result = worker.await.unwrap_or_else(|error| {
                        Err(if error.is_panic() {
                            PullError::ExecutorPanicked
                        } else {
                            PullError::ExecutorStopped
                        })
                    });
                    coordinator.finish(id, run_id, lane, result).await;
                });
            };
        }
    }

    async fn progress(&self, id: &str, run_id: u64, phase: String, applying: bool) -> bool {
        let changed = {
            let mut state = self.0.state.lock().await;
            let Some(job) = state.jobs.get_mut(id) else {
                return false;
            };
            if job.run_id != run_id
                || !matches!(job.state, PullState::Running | PullState::Applying)
            {
                return false;
            }
            let event = if applying {
                PullEvent::Apply
            } else {
                PullEvent::Progress
            };
            let transition = reduce(job.state, event);
            if !transition.accepted {
                return false;
            }
            job.state = transition.state;
            job.phase = Some(phase);
            prune_terminal(&mut state);
            publish_locked(&self.0, &state);
            true
        };
        changed
    }

    async fn applied(&self, id: &str, run_id: u64) -> bool {
        let mut state = self.0.state.lock().await;
        let closed = state.closed;
        let Some(job) = state.jobs.get_mut(id) else {
            return false;
        };
        if job.run_id != run_id {
            return false;
        }
        let transition = reduce(job.state, PullEvent::Applied);
        if !transition.accepted {
            return false;
        }
        job.state = transition.state;
        job.phase = Some("running".into());
        if closed {
            // Shutdown let this commit finish; the rest of the run is cancelled.
            job.cancel.store(true, Ordering::Release);
            job.state = reduce(job.state, PullEvent::Cancel).state;
        }
        publish_locked(&self.0, &state);
        !closed
    }

    async fn outcome_error(&self, id: &str, run_id: u64, message: String) -> bool {
        let mut state = self.0.state.lock().await;
        let Some(job) = state.jobs.get_mut(id) else {
            return false;
        };
        if job.run_id != run_id || !matches!(job.state, PullState::Running | PullState::Applying) {
            return false;
        }
        job.last_error = Some(message);
        publish_locked(&self.0, &state);
        true
    }

    async fn finish(
        &self,
        id: String,
        run_id: u64,
        lane: PullLane,
        result: Result<Value, PullError>,
    ) {
        let mut settle = None;
        let mut retry = None;
        {
            let mut state = self.0.state.lock().await;
            // The lane was occupied for this run when it started, whatever
            // happened to its job since; never leave it held.
            release_lane(&mut state, lane);
            if state.jobs.get(&id).is_none_or(|job| job.run_id != run_id) {
                publish_locked(&self.0, &state);
                self.kick();
                return;
            }
            let closed = state.closed;
            {
                let Some(job) = state.jobs.get_mut(&id) else {
                    // Run identity was checked under this same lock above.
                    return;
                };
                if job.state == PullState::Cancelling {
                    job.state = reduce(job.state, PullEvent::Stop).state;
                    let waiters = std::mem::take(&mut job.waiters);
                    settle = Some((waiters, Err(PullError::Cancelled)));
                    let target = job.target.clone();
                    remove_key_if_current(&mut state, &target, &id);
                } else {
                    match result {
                        Ok(value) => {
                            if job.last_error.is_some() {
                                job.state = reduce(job.state, PullEvent::Fail).state;
                                job.phase = Some("partial".into());
                            } else {
                                job.state = reduce(job.state, PullEvent::Succeed).state;
                                job.phase = Some("complete".into());
                            }
                            let waiters = std::mem::take(&mut job.waiters);
                            settle = Some((waiters, Ok(value)));
                            let target = job.target.clone();
                            remove_key_if_current(&mut state, &target, &id);
                        }
                        Err(error)
                            if !closed
                                && job.attempt <= job.retry_limit
                                && !matches!(error, PullError::Maintenance(_)) =>
                        {
                            let transition = reduce(job.state, PullEvent::WaitRetry);
                            job.state = transition.state;
                            job.phase = Some("retryWaiting".into());
                            job.last_error = Some(error.to_string());
                            job.ready = false;
                            retry = Some(job.delay.max(Duration::from_millis(250)));
                        }
                        Err(error) => {
                            job.state = reduce(job.state, PullEvent::Fail).state;
                            job.phase = Some("failed".into());
                            job.last_error = Some(error.to_string());
                            let waiters = std::mem::take(&mut job.waiters);
                            settle = Some((waiters, Err(error)));
                            let target = job.target.clone();
                            remove_key_if_current(&mut state, &target, &id);
                        }
                    }
                }
            }
            if let Some(job) = state.jobs.get_mut(&id) {
                if !job.live() {
                    job.work = None;
                }
            }
            prune_terminal(&mut state);
            publish_locked(&self.0, &state);
        }
        if let Some((waiters, result)) = settle {
            settle_waiters(waiters, result);
        }
        if let Some(delay) = retry {
            self.wake_later(id, delay, true);
        }
        self.kick();
    }
}

fn publish_locked(shared: &PullShared, state: &CoordinatorState) {
    let snapshots = snapshots_locked(state);
    {
        let previous = shared.snapshots.borrow();
        for next in &snapshots {
            let unchanged = previous
                .iter()
                .find(|old| old.id == next.id)
                .is_some_and(|old| {
                    old.state == next.state
                        && old.priority == next.priority
                        && old.phase == next.phase
                        && old.blocked_by == next.blocked_by
                        && old.attempt == next.attempt
                        && old.waiter_count == next.waiter_count
                });
            if !unchanged {
                log::info!(target: "isct_core::pull", "{} target={:?} state={:?} priority={:?} phase={:?} blocked={:?} attempt={} waiters={} school_lane={} http_lanes={} suspended={}",
                    next.id, next.target, next.state, next.priority, next.phase,
                    next.blocked_by, next.attempt, next.waiter_count,
                    state.school_running, state.http_running, state.suspended);
            }
        }
    }
    shared.snapshots.send_replace(snapshots);
}

fn settle_waiters(
    waiters: Vec<oneshot::Sender<Result<Value, PullError>>>,
    result: Result<Value, PullError>,
) {
    for waiter in waiters {
        let _ = waiter.send(result.clone());
    }
}

#[cfg(test)]
#[path = "tests/pull_coordinator.rs"]
mod tests;
