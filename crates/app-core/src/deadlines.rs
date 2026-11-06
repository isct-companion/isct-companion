//! Core's deadline work, each a [`Duty`] of the `PlanEngine`
//! (`approach/core-deadline-plan`). The services keep their rules; these
//! adapters tell the engine when each is next due and run it then.
use crate::{
    data::DataStore,
    net::BoxFuture,
    plan::{BackgroundWork, Duty},
    runtime::CoreRuntime,
    schedule::ReminderService,
    school_session::SchoolSessions,
};
use chrono::{DateTime, Duration, Utc};
use std::sync::{Arc, Mutex};

/// Maintenance runs this often, and first this long after start, clear of
/// startup work.
const MAINTENANCE_EVERY: Duration = Duration::days(1);
const MAINTENANCE_START: Duration = Duration::minutes(1);
const MAINTENANCE_RETRY: Duration = Duration::hours(1);
const MAINTENANCE_KEY: &str = "daily";
/// Under `local/`: the latest copy of the user tables, in the format Settings
/// imports.
pub(crate) const USER_SNAPSHOT_DIR: &str = "user-snapshot";

/// A sign-in's pause without a verdict ends: the account may sign in again
/// and the reads waiting on it go on.
pub(crate) struct SessionPauses(pub(crate) SchoolSessions);

impl Duty for SessionPauses {
    fn name(&self) -> &'static str {
        "session-pauses"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Network
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.0.next_pause_end()
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move { self.0.lift_pauses() })
    }
}

/// Reminders through the shell's notification port.
pub(crate) struct Reminders(pub(crate) ReminderService);

impl Duty for Reminders {
    fn name(&self) -> &'static str {
        "reminders"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Local
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.0.next_due()
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let Some(sink) = self.0.sink() else {
                return;
            };
            if let Err(error) = self.0.deliver_due(sink, Utc::now().fixed_offset()).await {
                crate::diagnostics::report("reminders", &error);
            }
        })
    }
}

/// The person's scheduled tasks, once a shell attached the export port.
pub(crate) struct Scheduled {
    pub(crate) runtime: Arc<CoreRuntime>,
    pub(crate) store: DataStore,
}

impl Duty for Scheduled {
    fn name(&self) -> &'static str {
        "scheduled-tasks"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Local
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.runtime.scheduler_services(&self.store)?;
        self.runtime.scheduler().next_due()
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let Some(services) = self.runtime.scheduler_services(&self.store) else {
                return;
            };
            if let Err(error) = self.runtime.scheduler().run_due(&services).await {
                crate::diagnostics::report("scheduler", &error);
            }
        })
    }
}

/// Automatic cloud sync: the journal's queue and captures of local changes.
pub(crate) struct Cloud(pub(crate) Arc<CoreRuntime>);

impl Duty for Cloud {
    fn name(&self) -> &'static str {
        "cloud-sync"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Network
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.0.cloud()?.next_due()
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let Some(cloud) = self.0.cloud() else {
                return;
            };
            if let Err(error) = cloud.run_due().await {
                crate::diagnostics::report("cloud-sync", &error);
            }
        })
    }
    fn interrupt(&self) {
        if let Some(cloud) = self.0.cloud() {
            cloud.interrupt();
        }
    }
}

/// Low-frequency maintenance, the only place stored things are collected:
/// unused school content (`sync_gc`), retired store generations and, on the
/// desktop, the LMS write ledger's expired records and unreferenced
/// snapshots. It also keeps a copy of the user tables outside the store: they
/// share one file with everything else, and cloud sync is optional, so a
/// damaged file would otherwise take them with it. Its schedule survives
/// restarts in `store-index.db`.
pub(crate) struct Maintenance {
    store: DataStore,
    #[cfg(feature = "agent")]
    submissions: Option<crate::submissions::SubmissionService>,
    next: Mutex<DateTime<Utc>>,
}

impl Maintenance {
    pub(crate) fn new(
        store: DataStore,
        #[cfg(feature = "agent")] submissions: Option<crate::submissions::SubmissionService>,
    ) -> Self {
        let earliest = Utc::now() + MAINTENANCE_START;
        let next = store
            .root()
            .maintenance_due(MAINTENANCE_KEY)
            .ok()
            .flatten()
            .and_then(|at| DateTime::from_timestamp(at, 0))
            .map_or(earliest, |at| at.max(earliest));
        Self {
            store,
            #[cfg(feature = "agent")]
            submissions,
            next: Mutex::new(next),
        }
    }

    pub(crate) fn pass(&self, now: DateTime<Utc>) -> Result<(), String> {
        let gc = self
            .store
            .write(None, |transaction| crate::sync_gc::gc(transaction, now))
            .map_err(|error| error.to_string());
        let retired = self
            .store
            .root()
            .collect_retired(now)
            .map_err(|error| error.to_string());
        let snapshot = crate::backup::BackupService::new(self.store.clone())
            .write_scheduled(&self.store.paths().local(USER_SNAPSHOT_DIR))
            .map_err(|error| error.to_string());
        #[cfg(feature = "agent")]
        let ledger = match &self.submissions {
            Some(submissions) => submissions
                .maintain(now.timestamp())
                .map_err(|error| error.to_string()),
            None => Ok(()),
        };
        #[cfg(not(feature = "agent"))]
        let ledger: Result<(), String> = Ok(());
        gc.map(|_| ())
            .and(retired.map(|_| ()))
            .and(snapshot.map(|_| ()))
            .and(ledger)
    }
}

impl Duty for Maintenance {
    fn name(&self) -> &'static str {
        "maintenance"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Local
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.next.lock().ok().map(|next| *next)
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let duty = self.clone();
            let ran = tokio::task::spawn_blocking(move || {
                let now = Utc::now();
                let passed = duty.pass(now);
                let next = match &passed {
                    Ok(()) => now + MAINTENANCE_EVERY,
                    Err(_) => now + MAINTENANCE_RETRY,
                };
                let succeeded = passed.is_ok().then_some(now.timestamp());
                if let Err(error) = duty.store.root().schedule_maintenance(
                    MAINTENANCE_KEY,
                    succeeded,
                    next.timestamp(),
                ) {
                    crate::diagnostics::report("maintenance", &error);
                }
                if let Ok(mut slot) = duty.next.lock() {
                    *slot = next;
                }
                passed
            })
            .await;
            match ran {
                Ok(Ok(())) => {}
                Ok(Err(error)) => crate::diagnostics::report("maintenance", &error),
                Err(error) => crate::diagnostics::report("maintenance", &error),
            }
        })
    }
}

/// The desktop's MCP listener: started or stopped to match the "Allow
/// local agents" setting, which may change outside Settings (a backup
/// restore, cloud sync). The shell owns the socket.
#[cfg(feature = "agent")]
pub trait AgentListener: Send + Sync {
    /// Starts or stops listening to match `enabled`. `Err` says why a start
    /// failed; Core tries again later.
    fn reconcile(&self, enabled: bool) -> BoxFuture<'_, Result<(), String>>;
}

/// When a failed start is tried again: 5 s, doubled, at most 5 min.
#[cfg(feature = "agent")]
const LISTENER_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(feature = "agent")]
const LISTENER_RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(5 * 60);

#[cfg(feature = "agent")]
pub(crate) struct AgentAccess {
    store: DataStore,
    listener: Mutex<Option<Arc<dyn AgentListener>>>,
    /// When to reconcile again: at once after the setting changed, later
    /// after a failed start, never otherwise.
    next: Mutex<Option<DateTime<Utc>>>,
    failures: std::sync::atomic::AtomicU32,
}

#[cfg(feature = "agent")]
impl AgentAccess {
    pub(crate) fn new(store: DataStore) -> Self {
        Self {
            store,
            listener: Mutex::new(None),
            next: Mutex::new(None),
            failures: std::sync::atomic::AtomicU32::new(0),
        }
    }

    pub(crate) fn attach(&self, listener: Arc<dyn AgentListener>) {
        if let Ok(mut slot) = self.listener.lock() {
            *slot = Some(listener);
        }
        self.changed();
    }

    /// The setting may have changed: reconcile now.
    pub(crate) fn changed(&self) {
        if let Ok(mut next) = self.next.lock() {
            *next = Some(Utc::now());
        }
    }

    pub(crate) fn detach(&self) {
        if let Ok(mut slot) = self.listener.lock() {
            *slot = None;
        }
    }
}

#[cfg(feature = "agent")]
impl Duty for AgentAccess {
    fn name(&self) -> &'static str {
        "agent-listener"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Local
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.listener.lock().ok()?.as_ref()?;
        self.next.lock().ok().and_then(|next| *next)
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let Some(listener) = self.listener.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            if let Ok(mut next) = self.next.lock() {
                *next = None;
            }
            let store = self.store.clone();
            // A setting that cannot be read stops the listener (fail closed).
            let enabled = tokio::task::spawn_blocking(move || store.agent_access_enabled())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false);
            use std::sync::atomic::Ordering;
            match listener.reconcile(enabled).await {
                Ok(()) => self.failures.store(0, Ordering::Release),
                Err(error) => {
                    let failures = self.failures.fetch_add(1, Ordering::AcqRel);
                    let wait = LISTENER_RETRY
                        .saturating_mul(1 << failures.min(6))
                        .min(LISTENER_RETRY_MAX);
                    if failures == 0 {
                        log::error!("[mcp] server unavailable: {error}");
                    }
                    if let Ok(mut next) = self.next.lock() {
                        let retry = Utc::now() + Duration::from_std(wait).unwrap_or_default();
                        *next = Some(next.map_or(retry, |next| next.min(retry)));
                    }
                }
            }
        })
    }
}

/// The LMS writes whose outcome is unknown, read until the LMS shows one,
/// and the assignments a settled write changed, read again
/// (`approach/lms-submission-contract`).
#[cfg(feature = "agent")]
pub(crate) struct SubmissionChecks(pub(crate) crate::submissions::SubmissionService);

#[cfg(feature = "agent")]
impl Duty for SubmissionChecks {
    fn name(&self) -> &'static str {
        "submission-checks"
    }
    fn work(&self) -> BackgroundWork {
        BackgroundWork::Network
    }
    fn next_due(&self) -> Option<DateTime<Utc>> {
        self.0.next_due()
    }
    fn run(self: Arc<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            if let Err(error) = self.0.run_due().await {
                crate::diagnostics::report("submissions", &error);
            }
        })
    }
}
