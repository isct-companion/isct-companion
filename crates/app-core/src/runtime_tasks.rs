//! Owned task lifetime: stop admission, signal wake loops, then drain work.
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::runtime::Handle;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RuntimeTaskError {
    #[error("The runtime has stopped.")]
    Stopped,
    #[error("The async runtime is unavailable.")]
    Unavailable,
    #[error("The runtime operation panicked.")]
    Panicked,
    #[error("The runtime operation stopped before completion.")]
    Interrupted,
}

impl From<tokio::task::JoinError> for RuntimeTaskError {
    fn from(error: tokio::task::JoinError) -> Self {
        if error.is_panic() {
            Self::Panicked
        } else {
            Self::Interrupted
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeTasks(Arc<TaskScope>);

#[derive(Default)]
struct TaskScope {
    // TaskTracker permits spawning after close; serialize admission to make
    // shutdown's empty result final rather than a gap between later spawns.
    closed: Mutex<bool>,
    stop: CancellationToken,
    tracker: TaskTracker,
}

impl RuntimeTasks {
    pub fn spawn(&self, work: impl Future<Output = ()> + Send + 'static) -> bool {
        let Ok(runtime) = Handle::try_current() else {
            return false;
        };
        self.spawn_on(&runtime, work)
    }

    /// Admit work onto an explicitly owned runtime. Native shells that create
    /// Core outside an entered Tokio context can still give the task scope a
    /// stable executor without temporarily entering it themselves.
    pub fn spawn_on(
        &self,
        runtime: &Handle,
        work: impl Future<Output = ()> + Send + 'static,
    ) -> bool {
        let Ok(closed) = self.0.closed.lock() else {
            return false;
        };
        if *closed {
            return false;
        }
        self.0.tracker.spawn_on(work, runtime);
        true
    }

    pub async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, RuntimeTaskError> {
        let task = {
            let closed = self
                .0
                .closed
                .lock()
                .map_err(|_| RuntimeTaskError::Stopped)?;
            if *closed {
                return Err(RuntimeTaskError::Stopped);
            }
            let runtime = Handle::try_current().map_err(|_| RuntimeTaskError::Unavailable)?;
            // Dropping the caller's JoinHandle detaches, rather than cancels,
            // admitted work. TaskTracker still drains it during shutdown.
            self.0.tracker.spawn_on(work, &runtime)
        };
        task.await.map_err(RuntimeTaskError::from)
    }

    pub fn close(&self) {
        let mut closed = self
            .0
            .closed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *closed = true;
        self.0.stop.cancel();
        self.0.tracker.close();
    }

    pub fn is_closed(&self) -> bool {
        self.0.stop.is_cancelled()
    }
    pub async fn closed(&self) {
        self.0.stop.cancelled().await;
    }
    /// Never abort an in-flight commit. Callers signal cooperative cancellation
    /// through the coordinator before waiting for these tasks to finish.
    pub async fn wait(&self) {
        self.0.tracker.wait().await;
    }
}

#[cfg(test)]
#[path = "tests/runtime_tasks.rs"]
mod tests;
