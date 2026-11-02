//! Shared background lifetime. Shells supply facts, adapters and periodic
//! wakeups; school and reference reads belong to `crate::sync_engine`.
use crate::{
    runtime_tasks::{RuntimeTaskError, RuntimeTasks},
    scheduler::{Scheduler, SchedulerError, SchedulerServices},
    sync_engine::{Hint, SyncHints},
};
use std::{
    future::Future,
    sync::{atomic::Ordering, Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Task(#[from] RuntimeTaskError),
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

/// Cache gc stays clear of startup work, then asks again now and then;
/// `sync_gc::gc` decides whether a day has passed since its last run.
const GC_START_DELAY: Duration = Duration::from_secs(60);
const GC_CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;

pub struct CoreRuntime {
    /// The sync engine `Core` owns; the runtime forwards lifecycle facts.
    /// Weak: the engine's store observer owns this runtime.
    sync: SyncHints,
    pub notes: crate::notes::NoteStore,
    cloud: OnceLock<crate::cloud_sync::CloudSync>,
    pub downloads: crate::downloads::Downloader,
    scheduler: Scheduler,
    tasks: RuntimeTasks,
    next_scheduled: Mutex<Instant>,
    next_gc: Mutex<Instant>,
    ticking: Arc<tokio::sync::Mutex<()>>,
    lifecycle: tokio::sync::Mutex<()>,
    suspended: std::sync::atomic::AtomicBool,
}

impl CoreRuntime {
    pub fn new(sync: SyncHints) -> Self {
        Self {
            sync,
            notes: crate::notes::NoteStore::default(),
            cloud: OnceLock::new(),
            downloads: crate::downloads::Downloader::default(),
            scheduler: Scheduler::default(),
            tasks: RuntimeTasks::default(),
            next_scheduled: Mutex::new(Instant::now()),
            next_gc: Mutex::new(Instant::now() + GC_START_DELAY),
            ticking: Arc::new(tokio::sync::Mutex::new(())),
            lifecycle: tokio::sync::Mutex::new(()),
            suspended: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn attach_cloud(&self, cloud: crate::cloud_sync::CloudSync) -> Result<(), RuntimeError> {
        self.cloud
            .set(cloud)
            .map_err(|_| RuntimeError::CloudAlreadyAttached)
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

    /// The periodic shell wake: the sync planner's `Wake`, cache gc and the
    /// person's scheduled tasks.
    pub async fn wake(self: &Arc<Self>, services: SchedulerServices) -> Result<(), RuntimeError> {
        if self.tasks.is_closed() {
            return Err(RuntimeTaskError::Stopped.into());
        }
        if self.suspended.load(Ordering::Acquire) {
            return Ok(());
        }
        let _lifecycle = self.lifecycle.lock().await;
        if self.tasks.is_closed() {
            return Err(RuntimeTaskError::Stopped.into());
        }
        if self.suspended.load(Ordering::Acquire) {
            return Ok(());
        }
        self.sync.send(Hint::Wake);
        let now = Instant::now();
        let gc_due = {
            let mut next = self.next_gc.lock().map_err(|_| RuntimeError::Clock)?;
            if now < *next {
                false
            } else {
                *next = now + GC_CHECK_INTERVAL;
                true
            }
        };
        if gc_due {
            // `sync_gc::gc` itself runs at most once a day, across restarts.
            let store = services.store.clone();
            self.tasks.spawn(async move {
                let collected = tokio::task::spawn_blocking(move || {
                    store.write(None, |transaction| {
                        crate::sync_gc::gc(transaction, chrono::Utc::now())
                    })
                })
                .await;
                match collected {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => crate::diagnostics::report("sync-gc", &error),
                    Err(error) => crate::diagnostics::report("sync-gc", &error),
                }
            });
        }
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

    /// Track the complete operation, including result and event writes.
    /// Closing a shell request does not detach it.
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
        self.sync.send(Hint::Suspend);
    }

    pub async fn resume(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        if self.tasks.is_closed() {
            return;
        }
        self.suspended.store(false, Ordering::Release);
        self.sync.send(Hint::Resume);
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
        let sync = self.sync.engine();
        let sync = async move {
            if let Some(sync) = sync {
                sync.shutdown().await;
            }
        };
        tokio::join!(sync, self.downloads.shutdown(), cloud);
        self.tasks.wait().await;
    }
}
