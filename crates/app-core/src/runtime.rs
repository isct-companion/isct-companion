//! Shared background lifetime. Shells supply facts and adapters; the
//! `PlanEngine` decides when the services here run (`crate::deadlines`), and
//! school and reference reads belong to it too.
use crate::{
    plan::{Hint, PlanHints},
    runtime_tasks::{RuntimeTaskError, RuntimeTasks},
    scheduler::{ScheduledExporter, Scheduler, SchedulerError, SchedulerServices},
};
use std::{
    future::Future,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Task(#[from] RuntimeTaskError),
    #[error("Cloud sync is already attached to this runtime.")]
    CloudAlreadyAttached,
    /// No shell attached a scheduled-export port.
    #[error("Scheduled tasks are unavailable here.")]
    NoExporter,
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;

pub struct CoreRuntime {
    /// The engine `Core` owns; the runtime forwards lifecycle facts.
    /// Weak: the engine's store observer owns this runtime.
    plan: PlanHints,
    pub notes: crate::notes::NoteStore,
    cloud: OnceLock<crate::cloud_sync::CloudSync>,
    pub downloads: crate::downloads::Downloader,
    scheduler: Scheduler,
    /// The shell's scheduled-export port, attached once.
    exporter: Mutex<Option<Arc<dyn ScheduledExporter>>>,
    tasks: RuntimeTasks,
    lifecycle: tokio::sync::Mutex<()>,
}

impl CoreRuntime {
    pub fn new(plan: PlanHints) -> Self {
        Self {
            plan,
            notes: crate::notes::NoteStore::default(),
            cloud: OnceLock::new(),
            downloads: crate::downloads::Downloader::default(),
            scheduler: Scheduler::default(),
            exporter: Mutex::new(None),
            tasks: RuntimeTasks::default(),
            lifecycle: tokio::sync::Mutex::new(()),
        }
    }

    pub fn attach_cloud(&self, cloud: crate::cloud_sync::CloudSync) -> Result<(), RuntimeError> {
        self.cloud
            .set(cloud)
            .map_err(|_| RuntimeError::CloudAlreadyAttached)?;
        self.plan.duties_changed();
        Ok(())
    }

    pub(crate) fn cloud(&self) -> Option<&crate::cloud_sync::CloudSync> {
        self.cloud.get()
    }

    /// The shell's scheduled-export port, for every scheduled task.
    pub fn attach_exporter(&self, exporter: Arc<dyn ScheduledExporter>) {
        if let Ok(mut slot) = self.exporter.lock() {
            *slot = Some(exporter);
        }
        self.plan.duties_changed();
    }

    pub(crate) fn scheduler(&self) -> &Scheduler {
        &self.scheduler
    }

    pub(crate) fn scheduler_services(&self, store: &crate::data::DataStore) -> Option<SchedulerServices> {
        let exporter = self.exporter.lock().ok()?.clone()?;
        Some(SchedulerServices {
            store: store.clone(),
            exporter,
        })
    }

    /// Local content changed: the next cloud pass captures it, a few
    /// seconds on so a burst of commits makes one capture.
    pub(crate) fn request_cloud_capture(&self) {
        if let Some(cloud) = self.cloud.get() {
            cloud.request_capture();
            self.plan.duties_changed();
        }
    }

    /// The person's scheduled tasks changed: plan their occurrences again.
    pub(crate) fn scheduled_tasks_changed(&self) {
        self.scheduler.changed();
        self.plan.duties_changed();
    }

    /// Work Core owns past its caller (an LMS write, a print job):
    /// shutdown waits for it to end.
    #[cfg_attr(
        not(any(
            feature = "agent",
            all(feature = "campus-printer", any(windows, target_os = "android"))
        )),
        allow(dead_code)
    )]
    pub(crate) fn tasks(&self) -> &RuntimeTasks {
        &self.tasks
    }

    /// Track the complete operation, including result and event writes.
    /// Closing a shell request does not detach it.
    pub async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, RuntimeTaskError> {
        self.tasks.run(work).await
    }

    /// Runs one scheduled task now, as the person asked.
    pub async fn run_scheduled(
        self: &Arc<Self>,
        store: &crate::data::DataStore,
        id: String,
    ) -> Result<Result<String, SchedulerError>, RuntimeError> {
        let services = self
            .scheduler_services(store)
            .ok_or(RuntimeError::NoExporter)?;
        let runtime = self.clone();
        let ran = self
            .run(async move { runtime.scheduler.run_now(&services, &id).await })
            .await?;
        self.scheduled_tasks_changed();
        Ok(ran)
    }

    pub async fn suspend(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        self.plan.send(Hint::Suspend);
    }

    pub async fn resume(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        if self.tasks.is_closed() {
            return;
        }
        self.plan.send(Hint::Resume);
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
        let plan = self.plan.engine();
        let plan = async move {
            if let Some(plan) = plan {
                plan.shutdown().await;
            }
        };
        tokio::join!(plan, self.downloads.shutdown(), cloud);
        self.tasks.wait().await;
        if let Ok(mut exporter) = self.exporter.lock() {
            *exporter = None;
        }
    }
}
