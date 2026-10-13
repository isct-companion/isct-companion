//! Owned task lifetime: stop admission, signal wake loops, then drain work.
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::oneshot;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("The runtime has stopped.")]
pub struct RuntimeStopped;

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
        let Ok(closed) = self.0.closed.lock() else {
            return false;
        };
        if *closed {
            return false;
        }
        self.0.tracker.spawn(work);
        true
    }

    pub async fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, RuntimeStopped> {
        let (send, receive) = oneshot::channel();
        if !self.spawn(async move {
            let _ = send.send(work.await);
        }) {
            return Err(RuntimeStopped);
        }
        receive.await.map_err(|_| RuntimeStopped)
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
mod tests {
    use super::*;
    #[tokio::test]
    async fn close_rejects_new_work_and_waits_for_existing_work() {
        let tasks = RuntimeTasks::default();
        let (send, receive) = oneshot::channel();
        assert!(tasks.spawn(async move {
            receive.await.unwrap();
        }));
        tasks.close();
        assert!(!tasks.spawn(async {}));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), tasks.wait())
                .await
                .is_err()
        );
        send.send(()).unwrap();
        tasks.wait().await;
        assert_eq!(tasks.run(async { 42 }).await, Err(RuntimeStopped));
    }
}
