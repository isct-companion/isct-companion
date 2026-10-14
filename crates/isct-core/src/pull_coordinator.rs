//! Shared scheduling for reference and school data pulls.
//!
//! The coordinator owns business-level admission, coalescing and cancellation.
//! School login and WebView navigation remain a separate executor state machine.

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
            Self::School { .. } => PullLane::School,
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
}

impl PullOptions {
    pub fn manual() -> Self {
        Self {
            priority: PullPriority::Manual,
            delay: Duration::ZERO,
            retry_limit: 0,
        }
    }

    pub fn automatic() -> Self {
        Self {
            priority: PullPriority::Automatic,
            delay: Duration::ZERO,
            retry_limit: 0,
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
            Progress | Apply | WaitRetry | Retry | Succeed | Fail | Stop => rejected(Queued),
        },
        Checking => match event {
            Block => accepted(Blocked),
            Ready => accepted(Queued),
            Cancel => accepted(Cancelled),
            Enqueue | Check | Start | Progress | Apply | WaitRetry | Retry | Succeed | Fail
            | Stop | Yield => rejected(Checking),
        },
        Blocked => match event {
            Enqueue => accepted(Blocked),
            Check => accepted(Checking),
            Ready => accepted(Queued),
            Cancel => accepted(Cancelled),
            Start | Progress | Apply | WaitRetry | Retry | Succeed | Fail | Stop | Yield
            | Block => rejected(Blocked),
        },
        Running => match event {
            Progress => accepted(Running),
            Apply => accepted(Applying),
            WaitRetry => accepted(RetryWaiting),
            Succeed => accepted(Succeeded),
            Fail => accepted(Failed),
            Cancel => accepted(Cancelling),
            Yield => accepted(Yielding),
            Enqueue | Check | Block | Ready | Start | Retry | Stop => rejected(Running),
        },
        Applying => match event {
            Progress => accepted(Applying),
            Apply => accepted(Applying),
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
            Check | Block | Ready | Start | Progress | Apply | WaitRetry | Succeed | Fail
            | Stop | Yield => rejected(RetryWaiting),
        },
        Succeeded => match event {
            Enqueue => accepted(Queued),
            Check | Block | Ready | Start | Progress | Apply | WaitRetry | Retry | Succeed
            | Fail | Cancel | Stop | Yield => rejected(Succeeded),
        },
        Failed => match event {
            Enqueue => accepted(Queued),
            Retry => accepted(Queued),
            Cancel => accepted(Cancelled),
            Check | Block | Ready | Start | Progress | Apply | WaitRetry | Succeed | Fail
            | Stop | Yield => rejected(Failed),
        },
        Cancelling => match event {
            Stop | Succeed | Fail => accepted(Cancelled),
            Cancel => accepted(Cancelling),
            Enqueue | Check | Block | Ready | Start | Progress | Apply | WaitRetry | Retry
            | Yield => rejected(Cancelling),
        },
        Cancelled => match event {
            Enqueue => accepted(Queued),
            Check | Block | Ready | Start | Progress | Apply | WaitRetry | Retry | Succeed
            | Fail | Cancel | Stop | Yield => rejected(Cancelled),
        },
        Yielding => match event {
            Ready => accepted(Queued),
            Cancel => accepted(Cancelled),
            Stop => accepted(Yielding),
            Enqueue | Check | Block | Start | Progress | Apply | WaitRetry | Retry | Succeed
            | Fail | Yield => rejected(Yielding),
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
}

impl PullContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn priority(&self) -> PullPriority {
        PullPriority::from_encoded(self.priority.load(Ordering::Acquire))
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
    /// executors. Applying retains its write gate until the commit completes.
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
                        Err(error) if !closed && job.attempt <= job.retry_limit => {
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

fn promote(job: &mut PullJob, options: &PullOptions) {
    if options.priority == PullPriority::Manual {
        job.priority = PullPriority::Manual;
        job.priority_cell
            .store(PullPriority::Manual.encoded(), Ordering::Release);
    }
    job.retry_limit = job.retry_limit.max(options.retry_limit);
}

fn covering_job(state: &CoordinatorState, target: &PullTarget) -> Option<String> {
    if let Some(id) = state.keys.get(target) {
        if state.jobs.get(id).is_some_and(PullJob::coalescible) {
            return Some(id.clone());
        }
    }
    None
}

fn blocker(job: &PullJob, conditions: PullConditions) -> Option<&'static str> {
    if job.priority == PullPriority::Manual || matches!(job.target, PullTarget::Reference { .. }) {
        return None;
    }
    if conditions.network == PullNetwork::Offline {
        Some("offline")
    } else if conditions.network == PullNetwork::Cellular && !conditions.allow_cellular {
        Some("cellular")
    } else if !conditions.foreground {
        Some("background")
    } else if conditions.school_visible {
        Some("schoolVisible")
    } else {
        None
    }
}

fn select_next(state: &mut CoordinatorState) -> Option<String> {
    if state.closed {
        return None;
    }
    let candidates = state.queue.iter().cloned().collect::<Vec<_>>();
    let mut selected: Option<(u8, u64, String)> = None;
    for id in candidates {
        let wait_for = state
            .jobs
            .get(&id)
            .and_then(|job| job.wait_for.as_ref())
            .cloned();
        if wait_for
            .as_ref()
            .and_then(|predecessor| state.jobs.get(predecessor))
            .is_some_and(PullJob::live)
        {
            continue;
        }
        let Some(job) = state.jobs.get_mut(&id) else {
            continue;
        };
        job.wait_for = None;
        if !job.ready || !matches!(job.state, PullState::Queued | PullState::Blocked) {
            continue;
        }
        job.state = reduce(job.state, PullEvent::Check).state;
        if let Some(reason) = if state.suspended {
            Some("suspended")
        } else {
            blocker(job, state.conditions)
        } {
            job.state = reduce(job.state, PullEvent::Block).state;
            job.blocked_by = Some(reason.into());
            continue;
        }
        job.state = reduce(job.state, PullEvent::Ready).state;
        job.blocked_by = None;
        let available = match job.target.lane() {
            PullLane::School => !state.school_running,
            PullLane::Http => state.http_running < 2,
        };
        if !available {
            continue;
        }
        let rank = if job.priority == PullPriority::Manual {
            0
        } else {
            1
        };
        let candidate = (rank, job.sequence, id);
        if selected.as_ref().is_none_or(|current| candidate < *current) {
            selected = Some(candidate);
        }
    }
    selected.map(|(_, _, id)| id)
}

fn occupy_lane(state: &mut CoordinatorState, lane: PullLane) {
    match lane {
        PullLane::School => state.school_running = true,
        PullLane::Http => state.http_running += 1,
    }
}

fn release_lane(state: &mut CoordinatorState, lane: PullLane) {
    match lane {
        PullLane::School => state.school_running = false,
        PullLane::Http => state.http_running = state.http_running.saturating_sub(1),
    }
}

fn remove_key_if_current(state: &mut CoordinatorState, target: &PullTarget, id: &str) {
    if state.keys.get(target).is_some_and(|current| current == id) {
        state.keys.remove(target);
    }
}

fn snapshots_locked(state: &CoordinatorState) -> Vec<PullSnapshot> {
    let mut jobs = state.jobs.values().collect::<Vec<_>>();
    jobs.sort_by_key(|job| job.sequence);
    jobs.into_iter().map(PullJob::snapshot).collect()
}

const TERMINAL_HISTORY_LIMIT: usize = 128;

fn prune_terminal(state: &mut CoordinatorState) {
    let mut terminal = state
        .jobs
        .values()
        .filter(|job| !job.live())
        .map(|job| (job.sequence, job.id.clone()))
        .collect::<Vec<_>>();
    if terminal.len() <= TERMINAL_HISTORY_LIMIT {
        return;
    }
    terminal.sort_by_key(|(sequence, _)| *sequence);
    let remove = terminal.len() - TERMINAL_HISTORY_LIMIT;
    for (_, id) in terminal.into_iter().take(remove) {
        state.jobs.remove(&id);
        state.queue.retain(|queued| queued != &id);
        state.keys.retain(|_, current| current != &id);
    }
}

fn publish_locked(shared: &PullShared, state: &CoordinatorState) {
    shared.snapshots.send_replace(snapshots_locked(state));
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
mod tests {
    use super::*;
    use futures_util::task::noop_waker_ref;
    use serde_json::json;
    use std::{
        future::Future,
        sync::atomic::{AtomicUsize, Ordering},
        task::{Context, Poll},
    };

    fn reference(kind: &str) -> PullTarget {
        PullTarget::Reference {
            kind: kind.into(),
            index_url: "https://example.com/index.json".into(),
            year: 2026,
        }
    }

    fn lms(course_id: Option<u64>) -> PullTarget {
        PullTarget::School {
            site: SchoolSite::ScienceLms,
            course_id,
        }
    }

    #[test]
    fn reducer_is_total_for_every_state_and_event() {
        let states = [
            PullState::Queued,
            PullState::Checking,
            PullState::Blocked,
            PullState::Running,
            PullState::Applying,
            PullState::RetryWaiting,
            PullState::Succeeded,
            PullState::Failed,
            PullState::Cancelling,
            PullState::Cancelled,
            PullState::Yielding,
        ];
        let events = [
            PullEvent::Enqueue,
            PullEvent::Check,
            PullEvent::Block,
            PullEvent::Ready,
            PullEvent::Start,
            PullEvent::Progress,
            PullEvent::Apply,
            PullEvent::WaitRetry,
            PullEvent::Retry,
            PullEvent::Succeed,
            PullEvent::Fail,
            PullEvent::Cancel,
            PullEvent::Stop,
            PullEvent::Yield,
        ];
        for state in states {
            for event in events.clone() {
                let transition = reduce(state, event);
                assert!(states.contains(&transition.state));
                assert!(!transition.diagnostic.is_empty());
            }
        }
    }

    #[test]
    fn run_without_an_async_runtime_fails_before_admission() {
        let coordinator = PullCoordinator::default();
        let mut request = Box::pin(coordinator.run(
            reference("no-runtime"),
            PullOptions::manual(),
            |_| async { Ok(json!(true)) },
        ));
        let mut context = Context::from_waker(noop_waker_ref());
        assert_eq!(
            Future::poll(request.as_mut(), &mut context),
            Poll::Ready(Err(PullError::RuntimeUnavailable))
        );
        drop(request);
        let state = coordinator.0.state.try_lock().unwrap();
        assert!(state.jobs.is_empty());
        assert!(state.queue.is_empty());
    }

    #[test]
    fn targets_are_validated_and_serialized_for_the_frontend() {
        assert!(PullTarget::School {
            site: SchoolSite::ScienceLms,
            course_id: Some(0),
        }
        .normalized()
        .is_err());
        assert!(PullTarget::School {
            site: SchoolSite::SciencePortal,
            course_id: None,
        }
        .normalized()
        .is_err());
        assert!(PullTarget::Reference {
            kind: "calendar".into(),
            index_url: "https://user:secret@example.com/index.json".into(),
            year: 2026,
        }
        .normalized()
        .is_err());
        assert!(PullTarget::Reference {
            kind: "calendar".into(),
            index_url: "http://example.com/index.json".into(),
            year: 2026,
        }
        .normalized()
        .is_err());
        assert!(PullTarget::Reference {
            kind: "calendar".into(),
            index_url: "https://example.com/index.json".into(),
            year: 1999,
        }
        .normalized()
        .is_err());

        let value = serde_json::to_value(lms(Some(42))).unwrap();
        assert_eq!(value["type"], "school");
        assert_eq!(value["courseId"], 42);
        assert!(value.get("course_id").is_none());
    }

    #[tokio::test]
    async fn same_key_runs_once_and_keeps_all_waiters() {
        let coordinator = PullCoordinator::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let run = |coordinator: PullCoordinator| {
            let calls = calls.clone();
            let gate = gate.clone();
            tokio::spawn(async move {
                coordinator
                    .run(reference("calendar"), PullOptions::automatic(), move |_| {
                        let calls = calls.clone();
                        let gate = gate.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            gate.notified().await;
                            Ok(json!({"ok":true}))
                        }
                    })
                    .await
            })
        };
        let first = run(coordinator.clone());
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let second = run(coordinator.clone());
        loop {
            let snapshots = coordinator.snapshots().await;
            if snapshots[0].waiter_count == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        gate.notify_waiters();
        assert_eq!(first.await.unwrap().unwrap(), json!({"ok":true}));
        assert_eq!(second.await.unwrap().unwrap(), json!({"ok":true}));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn successor_waits_for_cancelling_run_and_keeps_its_key() {
        let coordinator = PullCoordinator::default();
        let old_started = Arc::new(tokio::sync::Notify::new());
        let old_finish = Arc::new(tokio::sync::Notify::new());
        let old = {
            let coordinator = coordinator.clone();
            let old_started = old_started.clone();
            let old_finish = old_finish.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("successor"),
                        PullOptions::automatic(),
                        move |_| {
                            let old_started = old_started.clone();
                            let old_finish = old_finish.clone();
                            async move {
                                old_started.notify_one();
                                old_finish.notified().await;
                                Ok(json!("old"))
                            }
                        },
                    )
                    .await
            })
        };
        old_started.notified().await;
        let old_id = coordinator.snapshots().await[0].id.clone();
        assert!(coordinator.cancel(&old_id).await);

        let successor_started = Arc::new(AtomicBool::new(false));
        let successor_finish = Arc::new(tokio::sync::Notify::new());
        let successor = {
            let coordinator = coordinator.clone();
            let successor_started = successor_started.clone();
            let successor_finish = successor_finish.clone();
            tokio::spawn(async move {
                coordinator
                    .run(reference("successor"), PullOptions::manual(), move |_| {
                        let successor_started = successor_started.clone();
                        let successor_finish = successor_finish.clone();
                        async move {
                            successor_started.store(true, Ordering::SeqCst);
                            successor_finish.notified().await;
                            Ok(json!("new"))
                        }
                    })
                    .await
            })
        };
        loop {
            if coordinator.snapshots().await.len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        tokio::task::yield_now().await;
        assert!(!successor_started.load(Ordering::SeqCst));
        old_finish.notify_one();
        assert_eq!(old.await.unwrap().unwrap_err(), PullError::Cancelled);
        while !successor_started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }

        let joined = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(reference("successor"), PullOptions::manual(), |_| async {
                        panic!("successor key was lost")
                    })
                    .await
            })
        };
        loop {
            if coordinator
                .snapshots()
                .await
                .iter()
                .any(|snapshot| snapshot.state == PullState::Running && snapshot.waiter_count == 2)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        successor_finish.notify_one();
        assert_eq!(successor.await.unwrap().unwrap(), json!("new"));
        assert_eq!(joined.await.unwrap().unwrap(), json!("new"));
    }

    #[tokio::test]
    async fn queued_manual_join_promotes_executor_context_priority() {
        let coordinator = PullCoordinator::default();
        coordinator
            .set_conditions(PullConditions {
                foreground: false,
                ..PullConditions::default()
            })
            .await;
        let observed = Arc::new(Mutex::new(None));
        let automatic = {
            let coordinator = coordinator.clone();
            let observed = observed.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(Some(99)), PullOptions::automatic(), move |context| {
                        let observed = observed.clone();
                        async move {
                            *observed.lock().await = Some(context.priority());
                            Ok(json!("promoted"))
                        }
                    })
                    .await
            })
        };
        loop {
            if coordinator
                .snapshots()
                .await
                .iter()
                .any(|snapshot| snapshot.state == PullState::Blocked)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let manual = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(Some(99)), PullOptions::manual(), |_| async {
                        panic!("manual join must reuse the queued executor")
                    })
                    .await
            })
        };
        assert_eq!(automatic.await.unwrap().unwrap(), json!("promoted"));
        assert_eq!(manual.await.unwrap().unwrap(), json!("promoted"));
        assert_eq!(*observed.lock().await, Some(PullPriority::Manual));
    }

    #[tokio::test]
    async fn unchanged_conditions_do_not_publish_duplicate_snapshots() {
        let coordinator = PullCoordinator::default();
        let receiver = coordinator.subscribe();
        coordinator.set_conditions(PullConditions::default()).await;
        assert!(!receiver.has_changed().unwrap());
        coordinator
            .set_conditions(PullConditions {
                network: PullNetwork::Offline,
                ..PullConditions::default()
            })
            .await;
        assert!(receiver.has_changed().unwrap());
    }

    #[tokio::test]
    async fn automatic_school_admission_matches_the_previous_condition_matrix() {
        fn previously_allowed(conditions: PullConditions) -> bool {
            conditions.foreground
                && conditions.network != PullNetwork::Offline
                && (conditions.network != PullNetwork::Cellular || conditions.allow_cellular)
                && !conditions.school_visible
        }

        let coordinator = PullCoordinator::default();
        let mut course_id = 1u64;
        for network in [
            PullNetwork::Unknown,
            PullNetwork::Offline,
            PullNetwork::Wifi,
            PullNetwork::Cellular,
        ] {
            for foreground in [false, true] {
                for school_visible in [false, true] {
                    for allow_cellular in [false, true] {
                        let conditions = PullConditions {
                            network,
                            foreground,
                            school_visible,
                            allow_cellular,
                        };
                        coordinator.set_conditions(conditions).await;
                        let calls = Arc::new(AtomicUsize::new(0));
                        let task = {
                            let coordinator = coordinator.clone();
                            let calls = calls.clone();
                            let target = lms(Some(course_id));
                            tokio::spawn(async move {
                                coordinator
                                    .run(target, PullOptions::automatic(), move |_| {
                                        let calls = calls.clone();
                                        async move {
                                            calls.fetch_add(1, Ordering::SeqCst);
                                            Ok(json!("ran"))
                                        }
                                    })
                                    .await
                            })
                        };
                        let id = loop {
                            if let Some(snapshot) = coordinator
                                .snapshots()
                                .await
                                .into_iter()
                                .find(|snapshot| snapshot.target == lms(Some(course_id)))
                            {
                                if previously_allowed(conditions)
                                    || snapshot.state == PullState::Blocked
                                {
                                    break snapshot.id;
                                }
                            }
                            tokio::task::yield_now().await;
                        };
                        if previously_allowed(conditions) {
                            assert_eq!(task.await.unwrap().unwrap(), json!("ran"));
                            assert_eq!(calls.load(Ordering::SeqCst), 1);
                        } else {
                            assert_eq!(calls.load(Ordering::SeqCst), 0);
                            assert!(coordinator.cancel(&id).await);
                            assert_eq!(task.await.unwrap().unwrap_err(), PullError::Cancelled);
                        }
                        course_id += 1;
                    }
                }
            }
        }

        coordinator
            .set_conditions(PullConditions {
                network: PullNetwork::Offline,
                foreground: false,
                school_visible: true,
                allow_cellular: false,
            })
            .await;
        let manual = coordinator
            .run(lms(Some(course_id)), PullOptions::manual(), |_| async {
                Ok(json!("manual"))
            })
            .await;
        assert_eq!(manual.unwrap(), json!("manual"));
    }

    #[tokio::test]
    async fn stale_context_cannot_update_a_retried_run() {
        let coordinator = PullCoordinator::default();
        let attempts = Arc::new(AtomicUsize::new(0));
        let contexts = Arc::new(Mutex::new(Vec::<PullContext>::new()));
        let second_started = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let task = {
            let coordinator = coordinator.clone();
            let attempts = attempts.clone();
            let contexts = contexts.clone();
            let second_started = second_started.clone();
            let finish = finish.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("stale-context"),
                        PullOptions {
                            retry_limit: 1,
                            ..PullOptions::automatic()
                        },
                        move |context| {
                            let attempts = attempts.clone();
                            let contexts = contexts.clone();
                            let second_started = second_started.clone();
                            let finish = finish.clone();
                            async move {
                                contexts.lock().await.push(context);
                                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                                    Err(PullError::Execution("retry".into()))
                                } else {
                                    second_started.notify_one();
                                    finish.notified().await;
                                    Ok(json!("done"))
                                }
                            }
                        },
                    )
                    .await
            })
        };
        second_started.notified().await;
        let saved = contexts.lock().await.clone();
        assert_eq!(saved.len(), 2);
        assert!(!saved[0].phase("stale").await);
        assert!(saved[1].phase("current").await);
        finish.notify_one();
        assert_eq!(task.await.unwrap().unwrap(), json!("done"));
    }

    #[tokio::test]
    async fn full_lms_and_course_detail_keep_distinct_work() {
        let coordinator = PullCoordinator::default();
        coordinator
            .set_conditions(PullConditions {
                foreground: false,
                ..PullConditions::default()
            })
            .await;
        let detail_calls = Arc::new(AtomicUsize::new(0));
        let detail = {
            let coordinator = coordinator.clone();
            let calls = detail_calls.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(Some(42)), PullOptions::automatic(), move |_| {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            Ok(json!("detail"))
                        }
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        let full = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(None), PullOptions::manual(), |_| async {
                        Ok(json!("full"))
                    })
                    .await
            })
        };
        assert_eq!(full.await.unwrap().unwrap(), json!("full"));
        assert!(!detail.is_finished());
        assert_eq!(detail_calls.load(Ordering::SeqCst), 0);
        coordinator.set_conditions(PullConditions::default()).await;
        assert_eq!(detail.await.unwrap().unwrap(), json!("detail"));
        assert_eq!(detail_calls.load(Ordering::SeqCst), 1);

        let coordinator = PullCoordinator::default();
        let started = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let detail_calls = Arc::new(AtomicUsize::new(0));
        let full = {
            let coordinator = coordinator.clone();
            let started = started.clone();
            let finish = finish.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(None), PullOptions::manual(), move |_| {
                        let started = started.clone();
                        let finish = finish.clone();
                        async move {
                            started.notify_one();
                            finish.notified().await;
                            Ok(json!("running-full"))
                        }
                    })
                    .await
            })
        };
        started.notified().await;
        let detail = {
            let coordinator = coordinator.clone();
            let detail_calls = detail_calls.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(Some(7)), PullOptions::manual(), move |_| {
                        let detail_calls = detail_calls.clone();
                        async move {
                            detail_calls.fetch_add(1, Ordering::SeqCst);
                            Ok(json!("running-detail"))
                        }
                    })
                    .await
            })
        };
        loop {
            if coordinator
                .snapshots()
                .await
                .iter()
                .any(|snapshot| snapshot.target == lms(Some(7)))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!detail.is_finished());
        assert_eq!(detail_calls.load(Ordering::SeqCst), 0);
        finish.notify_one();
        assert_eq!(full.await.unwrap().unwrap(), json!("running-full"));
        assert_eq!(detail.await.unwrap().unwrap(), json!("running-detail"));
        assert_eq!(detail_calls.load(Ordering::SeqCst), 1);

        let coordinator = PullCoordinator::default();
        let finish = Arc::new(tokio::sync::Notify::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let enqueue_full = |coordinator: PullCoordinator| {
            let finish = finish.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(None), PullOptions::manual(), move |_| {
                        let finish = finish.clone();
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            finish.notified().await;
                            Ok(json!("deduplicated-full"))
                        }
                    })
                    .await
            })
        };
        let first = enqueue_full(coordinator.clone());
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let second = enqueue_full(coordinator.clone());
        loop {
            if coordinator.snapshots().await[0].waiter_count == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
        finish.notify_one();
        assert_eq!(first.await.unwrap().unwrap(), json!("deduplicated-full"));
        assert_eq!(second.await.unwrap().unwrap(), json!("deduplicated-full"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn school_lane_prefers_manual_then_preserves_fifo() {
        let coordinator = PullCoordinator::default();
        let gate = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(tokio::sync::Notify::new());
        let order = Arc::new(Mutex::new(Vec::new()));
        let first = {
            let coordinator = coordinator.clone();
            let gate = gate.clone();
            let started = started.clone();
            tokio::spawn(async move {
                coordinator
                    .run(lms(Some(1)), PullOptions::manual(), move |_| {
                        let gate = gate.clone();
                        let started = started.clone();
                        async move {
                            started.notify_one();
                            gate.notified().await;
                            Ok(json!("first"))
                        }
                    })
                    .await
            })
        };
        started.notified().await;

        let enqueue = |target, priority, label: &'static str| {
            let coordinator = coordinator.clone();
            let order = order.clone();
            tokio::spawn(async move {
                coordinator
                    .run(target, priority, move |_| {
                        let order = order.clone();
                        async move {
                            order.lock().await.push(label);
                            Ok(json!(label))
                        }
                    })
                    .await
            })
        };
        let automatic_a = enqueue(
            PullTarget::School {
                site: SchoolSite::Kyomu,
                course_id: None,
            },
            PullOptions::automatic(),
            "automatic-a",
        );
        let automatic_b = enqueue(lms(Some(2)), PullOptions::automatic(), "automatic-b");
        let manual = enqueue(lms(Some(3)), PullOptions::manual(), "manual");
        loop {
            if coordinator
                .snapshots()
                .await
                .iter()
                .filter(|snapshot| snapshot.state == PullState::Queued)
                .count()
                == 3
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        gate.notify_one();
        first.await.unwrap().unwrap();
        manual.await.unwrap().unwrap();
        automatic_a.await.unwrap().unwrap();
        automatic_b.await.unwrap().unwrap();
        assert_eq!(
            *order.lock().await,
            ["manual", "automatic-a", "automatic-b"]
        );
    }

    #[tokio::test]
    async fn retry_uses_the_factory_and_queued_cancel_settles() {
        let coordinator = PullCoordinator::default();
        let attempts = Arc::new(AtomicUsize::new(0));
        let result = coordinator
            .run(
                reference("retry"),
                PullOptions {
                    retry_limit: 1,
                    ..PullOptions::automatic()
                },
                {
                    let attempts = attempts.clone();
                    move |_| {
                        let attempts = attempts.clone();
                        async move {
                            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                                Err(PullError::Execution("try again".into()))
                            } else {
                                Ok(json!("recovered"))
                            }
                        }
                    }
                },
            )
            .await;
        assert_eq!(result.unwrap(), json!("recovered"));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);

        let delayed = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("cancel"),
                        PullOptions {
                            delay: Duration::from_secs(60),
                            ..PullOptions::automatic()
                        },
                        |_| async { Ok(json!("too late")) },
                    )
                    .await
            })
        };
        let id = loop {
            if let Some(snapshot) = coordinator
                .snapshots()
                .await
                .into_iter()
                .find(|snapshot| matches!(&snapshot.target, PullTarget::Reference { kind, .. } if kind == "cancel"))
            {
                break snapshot.id;
            }
            tokio::task::yield_now().await;
        };
        assert!(coordinator.cancel(&id).await);
        assert_eq!(delayed.await.unwrap().unwrap_err(), PullError::Cancelled);
    }

    #[tokio::test]
    async fn http_lane_runs_two_jobs_and_executor_panic_settles_waiter() {
        let coordinator = PullCoordinator::default();
        let gate = Arc::new(tokio::sync::Notify::new());
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut jobs = Vec::new();
        for kind in ["calendar", "exams"] {
            let coordinator = coordinator.clone();
            let gate = gate.clone();
            let running = running.clone();
            let peak = peak.clone();
            jobs.push(tokio::spawn(async move {
                coordinator
                    .run(reference(kind), PullOptions::automatic(), move |_| {
                        let gate = gate.clone();
                        let running = running.clone();
                        let peak = peak.clone();
                        async move {
                            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            gate.notified().await;
                            running.fetch_sub(1, Ordering::SeqCst);
                            Ok(json!(kind))
                        }
                    })
                    .await
            }));
        }
        while running.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        gate.notify_waiters();
        for job in jobs {
            job.await.unwrap().unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);

        let error = coordinator
            .run(reference("panic"), PullOptions::manual(), |_| async {
                panic!("executor bug")
            })
            .await
            .unwrap_err();
        assert_eq!(error, PullError::ExecutorPanicked);
    }

    #[tokio::test]
    async fn running_cancel_waits_for_executor_and_late_progress_is_rejected() {
        let coordinator = PullCoordinator::default();
        let started = Arc::new(tokio::sync::Notify::new());
        let stop = Arc::new(tokio::sync::Notify::new());
        let saved_context = Arc::new(Mutex::new(None));
        let task = {
            let coordinator = coordinator.clone();
            let started = started.clone();
            let stop = stop.clone();
            let saved_context = saved_context.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("calendar"),
                        PullOptions::manual(),
                        move |context| {
                            let started = started.clone();
                            let stop = stop.clone();
                            let saved_context = saved_context.clone();
                            async move {
                                *saved_context.lock().await = Some(context.clone());
                                started.notify_one();
                                stop.notified().await;
                                Ok(json!("late success"))
                            }
                        },
                    )
                    .await
            })
        };
        started.notified().await;
        let snapshot = coordinator.snapshots().await;
        let id = snapshot[0].id.clone();
        assert!(coordinator.cancel(&id).await);
        assert_eq!(
            coordinator.snapshots().await[0].state,
            PullState::Cancelling
        );
        stop.notify_one();
        assert_eq!(task.await.unwrap().unwrap_err(), PullError::Cancelled);
        let context = saved_context.lock().await.clone().unwrap();
        assert!(!context.phase("late").await);
        assert_eq!(coordinator.snapshots().await[0].state, PullState::Cancelled);
    }

    #[tokio::test]
    async fn shutdown_drains_applying_cancels_running_and_releases_captures() {
        let coordinator = PullCoordinator::default();
        let capture = Arc::new(());
        let weak = Arc::downgrade(&capture);
        let entered = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let applying = {
            let coordinator = coordinator.clone();
            let entered = entered.clone();
            let finish = finish.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("committing"),
                        PullOptions::manual(),
                        move |context| {
                            let capture = capture.clone();
                            let entered = entered.clone();
                            let finish = finish.clone();
                            async move {
                                assert!(context.applying().await);
                                entered.notify_one();
                                finish.notified().await;
                                assert!(!context.is_cancelled());
                                drop(capture);
                                Ok(json!("committed"))
                            }
                        },
                    )
                    .await
            })
        };
        entered.notified().await;
        let (sent, received) = oneshot::channel();
        let sent = Arc::new(Mutex::new(Some(sent)));
        let running = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("fetching"),
                        PullOptions::manual(),
                        move |context| {
                            let sent = sent.clone();
                            async move {
                                sent.lock().await.take().unwrap().send(context.clone()).ok();
                                while !context.is_cancelled() {
                                    tokio::task::yield_now().await;
                                }
                                assert!(!context.applying().await);
                                Ok(json!("discarded"))
                            }
                        },
                    )
                    .await
            })
        };
        let late = received.await.unwrap();
        let shutdown = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.shutdown().await })
        };
        assert_eq!(running.await.unwrap(), Err(PullError::Cancelled));
        assert!(!shutdown.is_finished());
        assert_eq!(
            coordinator
                .run(reference("late"), PullOptions::manual(), |_| async {
                    Ok(Value::Null)
                })
                .await,
            Err(PullError::CoordinatorStopped)
        );
        finish.notify_one();
        assert_eq!(applying.await.unwrap().unwrap(), json!("committed"));
        shutdown.await.unwrap();
        assert!(!late.phase("late").await);
        assert!(
            weak.upgrade().is_none(),
            "history must not retain executor captures"
        );
        assert!(coordinator.0.tasks.is_closed());
        assert_eq!(coordinator.0.state.lock().await.http_running, 0);
    }

    #[tokio::test]
    async fn suspend_resume_and_shutdown_of_delayed_work_are_finite() {
        let coordinator = PullCoordinator::default();
        coordinator.suspend().await;
        let mut states = coordinator.subscribe();
        let task = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                coordinator
                    .run(reference("paused"), PullOptions::manual(), |_| async {
                        Ok(json!(42))
                    })
                    .await
            })
        };
        loop {
            states.changed().await.unwrap();
            if states
                .borrow()
                .iter()
                .any(|row| row.blocked_by.as_deref() == Some("suspended"))
            {
                break;
            }
        }
        assert!(!task.is_finished());
        coordinator.resume().await;
        assert_eq!(task.await.unwrap().unwrap(), json!(42));
        let task = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                let mut options = PullOptions::manual();
                options.delay = Duration::from_secs(3600);
                coordinator
                    .run(reference("delayed"), options, |_| async {
                        panic!("cancelled delay executed")
                    })
                    .await
            })
        };
        loop {
            states.changed().await.unwrap();
            if states
                .borrow()
                .iter()
                .any(|row| row.state == PullState::Queued)
            {
                break;
            }
        }
        tokio::time::timeout(Duration::from_secs(1), coordinator.shutdown())
            .await
            .unwrap();
        assert_eq!(task.await.unwrap(), Err(PullError::Cancelled));
        coordinator.resume().await;
        coordinator.shutdown().await;
    }

    #[tokio::test]
    async fn applying_rejects_cancel_and_terminal_history_is_bounded() {
        let coordinator = PullCoordinator::default();
        let applying = Arc::new(tokio::sync::Notify::new());
        let finish = Arc::new(tokio::sync::Notify::new());
        let task = {
            let coordinator = coordinator.clone();
            let applying = applying.clone();
            let finish = finish.clone();
            tokio::spawn(async move {
                coordinator
                    .run(
                        reference("applying"),
                        PullOptions::manual(),
                        move |context| {
                            let applying = applying.clone();
                            let finish = finish.clone();
                            async move {
                                assert!(context.applying().await);
                                assert!(context.applying().await);
                                applying.notify_one();
                                finish.notified().await;
                                Ok(json!("applied"))
                            }
                        },
                    )
                    .await
            })
        };
        applying.notified().await;
        let id = coordinator.snapshots().await[0].id.clone();
        assert!(!coordinator.cancel(&id).await);
        finish.notify_one();
        assert_eq!(task.await.unwrap().unwrap(), json!("applied"));

        for index in 0..=TERMINAL_HISTORY_LIMIT {
            coordinator
                .run(
                    reference(&format!("history-{index}")),
                    PullOptions::manual(),
                    |_| async { Ok(json!(true)) },
                )
                .await
                .unwrap();
        }
        let snapshots = coordinator.snapshots().await;
        assert_eq!(snapshots.len(), TERMINAL_HISTORY_LIMIT);
        assert!(snapshots.iter().all(|snapshot| matches!(
            snapshot.state,
            PullState::Succeeded | PullState::Failed | PullState::Cancelled
        )));
    }

    #[tokio::test]
    async fn finishing_an_unknown_run_still_releases_its_lane() {
        let coordinator = PullCoordinator::default();
        occupy_lane(&mut *coordinator.0.state.lock().await, PullLane::School);
        coordinator
            .finish("pull-missing".into(), 7, PullLane::School, Ok(Value::Null))
            .await;
        assert!(!coordinator.0.state.lock().await.school_running);
        assert_eq!(
            coordinator
                .run(lms(None), PullOptions::manual(), |_| async {
                    Ok(json!("ran"))
                })
                .await
                .unwrap(),
            json!("ran")
        );
    }

    #[tokio::test]
    async fn partial_failure_keeps_payload_and_marks_snapshot_failed() {
        let coordinator = PullCoordinator::default();
        let value = coordinator
            .run(
                reference("partial"),
                PullOptions::manual(),
                |context| async move {
                    assert!(context.outcome_error("one course could not be read").await);
                    Ok(json!({"successful": 3, "failed": 1}))
                },
            )
            .await
            .unwrap();
        assert_eq!(value, json!({"successful": 3, "failed": 1}));
        let snapshot = coordinator.snapshots().await.pop().unwrap();
        assert_eq!(snapshot.state, PullState::Failed);
        assert_eq!(
            snapshot.last_error.as_deref(),
            Some("one course could not be read")
        );
    }
}
