//! The single hidden lane: sign-in and session checks, and visibility yields.
use super::*;

impl SchoolBrowserEngine {
    /// Subscribes before checking visibility, so a page opened after this
    /// check reaches the caller's wait as `Trace::Browsing`. Hidden work must
    /// never start while the person is looking at a school page.
    pub(super) fn hidden_traces(&self) -> Result<broadcast::Receiver<Trace>, SchoolBrowserError> {
        let traces = self.0.traces.subscribe();
        if self.0.state.guard().browsing.is_some() {
            return Err(SchoolBrowserError::Preempted);
        }
        Ok(traces)
    }

    pub(super) async fn acquire_lane(
        &self,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, SchoolBrowserError> {
        tokio::select! {
            lane = self.0.lane.lock() => Ok(lane),
            _ = self.0.tasks.closed() => Err(SchoolBrowserError::Stopped),
        }
    }

    pub(super) async fn wait_until_hidden_without_context(&self) -> Result<(), SchoolBrowserError> {
        loop {
            if self.0.tasks.is_closed() {
                return Err(SchoolBrowserError::Stopped);
            }
            let visible = self.0.state.guard().browsing.is_some();
            if !visible {
                return Ok(());
            }
            tokio::select! {
                _ = self.0.browsing_changed.notified() => {}
                _ = self.0.tasks.closed() => return Err(SchoolBrowserError::Stopped),
            }
        }
    }
}
