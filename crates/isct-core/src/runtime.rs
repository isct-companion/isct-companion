//! Shared refresh lifetime. Shells supply facts, adapters and periodic wakeups.
use crate::{
    pull_coordinator::{PullCoordinator, PullNetwork},
    runtime_tasks::{RuntimeTaskError, RuntimeTasks},
    scheduler::{Scheduler, SchedulerError, SchedulerServices},
    school_refresh::SchoolRefresh,
    school_service::SchoolError,
};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Task(#[from] RuntimeTaskError),
    #[error(transparent)]
    School(#[from] SchoolError),
    #[error("Refresh clock is unavailable.")]
    Clock,
    #[error("Cloud sync is already attached to this runtime.")]
    CloudAlreadyAttached,
}

#[derive(Debug, thiserror::Error)]
pub enum CloudWakeError {
    #[error("Cloud sync is not attached to this runtime.")]
    NotAttached,
    #[error(transparent)]
    Sync(#[from] crate::cloud_sync::CloudSyncError),
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;

pub struct CoreRuntime {
    /// Pulls go through `run`-tracked services; shells read and cancel via
    /// the methods below.
    pub(crate) coordinator: PullCoordinator,
    pub notes: crate::notes::NoteStore,
    cloud: OnceLock<crate::cloud_sync::CloudSync>,
    pub downloads: crate::downloads::Downloader,
    pub lms_session: crate::lms_session::LmsSession,
    pub(crate) refresh: SchoolRefresh,
    scheduler: Scheduler,
    tasks: RuntimeTasks,
    next_scheduled: Mutex<Instant>,
    ticking: Arc<tokio::sync::Mutex<()>>,
    lifecycle: tokio::sync::Mutex<()>,
    suspended: AtomicBool,
    event_forwarder_started: AtomicBool,
}

impl CoreRuntime {
    pub fn new(initial_network: PullNetwork) -> Self {
        Self {
            coordinator: PullCoordinator::default(),
            notes: crate::notes::NoteStore::default(),
            cloud: OnceLock::new(),
            downloads: crate::downloads::Downloader::default(),
            lms_session: crate::lms_session::LmsSession::default(),
            refresh: SchoolRefresh::new(initial_network),
            scheduler: Scheduler::default(),
            tasks: RuntimeTasks::default(),
            next_scheduled: Mutex::new(Instant::now()),
            ticking: Arc::new(tokio::sync::Mutex::new(())),
            lifecycle: tokio::sync::Mutex::new(()),
            suspended: AtomicBool::new(false),
            event_forwarder_started: AtomicBool::new(false),
        }
    }

    /// Start the runtime-owned pull snapshot bridge exactly once. Its lifetime
    /// is the runtime task scope, so every shell gets identical forwarding and
    /// shutdown behavior.
    pub fn start_event_forwarding(
        self: &Arc<Self>,
        events: Arc<dyn crate::ports::EventSink>,
        handle: &Handle,
    ) -> Result<(), RuntimeTaskError> {
        if self.tasks.is_closed() {
            return Err(RuntimeTaskError::Stopped);
        }
        if self
            .event_forwarder_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        let mut states = self.subscribe_pulls();
        let tasks = self.tasks.clone();
        if !tasks.clone().spawn_on(handle, async move {
            loop {
                tokio::select! {
                    _ = tasks.closed() => break,
                    changed = states.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                }
                let snapshot = states.borrow_and_update().clone();
                let _ = events.emit(crate::events::CoreEvent::PullStateChanged(snapshot));
            }
        }) {
            self.event_forwarder_started.store(false, Ordering::Release);
            return Err(RuntimeTaskError::Stopped);
        }
        Ok(())
    }

    pub fn attach_cloud(&self, cloud: crate::cloud_sync::CloudSync) -> Result<(), RuntimeError> {
        self.cloud
            .set(cloud.clone())
            .map_err(|_| RuntimeError::CloudAlreadyAttached)?;
        // Notes live outside school.db, so they report their own changes.
        self.notes.on_change(move || cloud.request_capture());
        Ok(())
    }

    /// Bring the next cloud capture forward after local content changed.
    /// The shell's next wake runs it; nothing is captured without a wake.
    pub(crate) fn request_cloud_capture(&self) {
        if let Some(cloud) = self.cloud.get() {
            cloud.request_capture();
        }
    }

    pub fn wake_cloud(
        &self,
        services: crate::cloud_sync::CloudServices,
    ) -> Result<bool, CloudWakeError> {
        self.cloud
            .get()
            .ok_or(CloudWakeError::NotAttached)?
            .tick(services)
            .map_err(CloudWakeError::from)
    }

    pub async fn wake(
        self: &Arc<Self>,
        services: SchedulerServices,
        school_visible: bool,
    ) -> Result<(), RuntimeError> {
        if self.tasks.is_closed() {
            return Err(RuntimeTaskError::Stopped.into());
        }
        if self.suspended.load(Ordering::Acquire) {
            return Ok(());
        }
        // Storage reads happen before the lifecycle lock, so suspend/resume
        // (called on the UI thread) only ever wait for in-memory work.
        let inputs = SchoolRefresh::load_inputs(&services.store).await;
        let _lifecycle = self.lifecycle.lock().await;
        if self.tasks.is_closed() {
            return Err(RuntimeTaskError::Stopped.into());
        }
        if self.suspended.load(Ordering::Acquire) {
            return Ok(());
        }
        self.refresh
            .apply_conditions(inputs.as_ref(), &self.coordinator, school_visible)
            .await?;
        if let Ok(inputs) = &inputs {
            self.refresh.tick(
                inputs,
                &services.school,
                services.executor.clone(),
                &self.tasks,
            )?;
        }
        let now = Instant::now();
        let due = {
            let mut next = self
                .next_scheduled
                .lock()
                .map_err(|_| RuntimeError::Clock)?;
            if now < *next {
                false
            } else {
                *next = now + Duration::from_secs(30);
                true
            }
        };
        if due {
            if let Ok(guard) = self.ticking.clone().try_lock_owned() {
                let runtime = self.clone();
                self.tasks.spawn(async move {
                    let _guard = guard;
                    if let Err(error) = runtime.scheduler.tick(&services).await {
                        crate::diagnostics::report("scheduler", &error);
                    }
                });
            }
        }
        Ok(())
    }

    /// Republish pull conditions after a shell fact changed (focus, network,
    /// school view). Async and off the lifecycle lock: safe to spawn from a UI
    /// event handler without blocking it.
    pub async fn publish_conditions(
        &self,
        store: &crate::data::DataStore,
        school_visible: bool,
    ) -> Result<(), RuntimeError> {
        self.refresh
            .publish_conditions(store, &self.coordinator, school_visible)
            .await
            .map_err(RuntimeError::from)
    }

    /// Forward a shell fact (foreground, account, lecture, network).
    pub fn refresh_event(
        &self,
        event: crate::school_refresh::RefreshEvent,
    ) -> Result<(), RuntimeError> {
        Ok(self.refresh.event(event)?)
    }

    pub async fn pull_snapshots(&self) -> Vec<crate::pull_coordinator::PullSnapshot> {
        self.coordinator.snapshots().await
    }

    pub fn subscribe_pulls(
        &self,
    ) -> tokio::sync::watch::Receiver<Vec<crate::pull_coordinator::PullSnapshot>> {
        self.coordinator.subscribe()
    }

    /// Cancel a pull before it applies; false when it cannot be cancelled.
    pub async fn cancel_pull(&self, id: &str) -> bool {
        self.coordinator.cancel(id).await
    }

    /// Track the complete operation, including result/event writes after its
    /// coordinated pull finishes. Closing a shell request does not detach it.
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, RuntimeTaskError> {
        self.tasks.run(work).await
    }

    pub async fn run_scheduled(
        self: &Arc<Self>,
        services: SchedulerServices,
        id: String,
    ) -> Result<Result<String, SchedulerError>, RuntimeTaskError> {
        let runtime = self.clone();
        self.run(async move { runtime.scheduler.run_now(&services, &id).await })
            .await
    }

    pub async fn suspend(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        self.suspended.store(true, Ordering::Release);
        self.coordinator.suspend().await;
    }

    pub async fn resume(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        if self.tasks.is_closed() {
            return;
        }
        self.coordinator.resume().await;
        self.suspended.store(false, Ordering::Release);
        if let Ok(mut next) = self.next_scheduled.lock() {
            *next = Instant::now();
        }
    }

    pub async fn shutdown(&self) {
        {
            let _lifecycle = self.lifecycle.lock().await;
            self.tasks.close();
        }
        let cloud = async {
            if let Some(cloud) = self.cloud.get() {
                cloud.shutdown().await;
            }
        };
        tokio::join!(
            self.coordinator.shutdown(),
            self.downloads.shutdown(),
            cloud
        );
        self.tasks.wait().await;
    }
}
