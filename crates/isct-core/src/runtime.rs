//! Shared refresh lifetime. Shells supply facts, adapters and periodic wakeups.
use crate::{
    pull_coordinator::{PullCoordinator, PullNetwork},
    runtime_tasks::{RuntimeStopped, RuntimeTasks},
    scheduler::{Scheduler, SchedulerError, SchedulerServices},
    school_refresh::SchoolRefresh,
    school_service::SchoolError,
};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Stopped(#[from] RuntimeStopped),
    #[error(transparent)]
    School(#[from] SchoolError),
    #[error("Refresh clock is unavailable.")]
    Clock,
}

#[cfg(test)]
mod tests;

pub struct CoreRuntime {
    pub coordinator: PullCoordinator,
    pub refresh: SchoolRefresh,
    scheduler: Scheduler,
    tasks: RuntimeTasks,
    next_scheduled: Mutex<Instant>,
    ticking: Arc<tokio::sync::Mutex<()>>,
    lifecycle: tokio::sync::Mutex<()>,
    suspended: AtomicBool,
}

impl CoreRuntime {
    pub fn new(initial_network: PullNetwork) -> Self {
        Self {
            coordinator: PullCoordinator::default(),
            refresh: SchoolRefresh::new(initial_network),
            scheduler: Scheduler::default(),
            tasks: RuntimeTasks::default(),
            next_scheduled: Mutex::new(Instant::now()),
            ticking: Arc::new(tokio::sync::Mutex::new(())),
            lifecycle: tokio::sync::Mutex::new(()),
            suspended: AtomicBool::new(false),
        }
    }

    pub async fn wake(
        self: &Arc<Self>,
        services: SchedulerServices,
        school_visible: bool,
    ) -> Result<(), RuntimeError> {
        let _lifecycle = self.lifecycle.lock().await;
        if self.tasks.is_closed() {
            return Err(RuntimeStopped.into());
        }
        if self.suspended.load(Ordering::Acquire) {
            return Ok(());
        }
        self.refresh
            .publish_conditions(&services.store, &self.coordinator, school_visible)
            .await?;
        self.refresh
            .tick(
                &services.store,
                &services.school,
                services.executor.clone(),
                &self.tasks,
            )
            .await?;
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
                    let _ = runtime.scheduler.tick(&services).await;
                });
            }
        }
        Ok(())
    }

    /// Track the complete operation, including result/event writes after its
    /// coordinated pull finishes. Closing a shell request does not detach it.
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, RuntimeStopped> {
        self.tasks.run(work).await
    }

    pub async fn run_scheduled(
        self: &Arc<Self>,
        services: SchedulerServices,
        id: String,
    ) -> Result<Result<String, SchedulerError>, RuntimeStopped> {
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
        self.coordinator.shutdown().await;
        self.tasks.wait().await;
    }
}
