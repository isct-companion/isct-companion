//! The single hidden lane: sign-in and session checks, and visibility yields.
use super::*;

/// A view stopped for an abandoned attempt that has not confirmed the stop by
/// then is replaced instead of waited on.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// When a session job must have answered: `SESSION_TIMEOUT` after it was
/// admitted, the wait for the lane included. Time a visible school page
/// keeps the job waiting does not count.
pub(super) struct Deadline(tokio::time::Instant);

impl Deadline {
    pub(super) fn admitted() -> Self {
        Self(tokio::time::Instant::now() + SESSION_TIMEOUT)
    }

    pub(super) fn at(&self) -> tokio::time::Instant {
        self.0
    }
}

pub(super) fn not_in_time() -> SchoolBrowserError {
    SchoolBrowserError::Timeout("The queued school sign-in did not start in time.".into())
}

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

    /// Waits for visible pages to hide, without holding the lane or spending
    /// the deadline, then for the lane by the deadline.
    pub(super) async fn hidden_lane(
        &self,
        deadline: &mut Deadline,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, SchoolBrowserError> {
        let hidden_from = tokio::time::Instant::now();
        self.wait_until_hidden_without_context().await?;
        deadline.0 += hidden_from.elapsed();
        tokio::select! {
            lane = tokio::time::timeout_at(deadline.0, self.0.lane.lock()) => {
                lane.map_err(|_| not_in_time())
            }
            _ = self.0.tasks.closed() => Err(SchoolBrowserError::Stopped),
        }
    }

    /// The view hidden work on `site` runs in. A view stopped for an abandoned
    /// attempt is used again only once it has confirmed the stop, so nothing
    /// that attempt left running can act on the next one.
    pub(super) async fn lease_view(
        &self,
        site: SchoolSite,
        initial: Url,
        deadline: tokio::time::Instant,
    ) -> Result<BrowserId, SchoolBrowserError> {
        let mut traces = self.0.traces.subscribe();
        let stopping = self.0.state.with(|state| {
            state
                .views
                .get(&site)
                .filter(|view| view.stopping.is_some())
                .map(|view| view.id)
        });
        if let Some(stopping) = stopping {
            let grace = deadline.min(tokio::time::Instant::now() + STOP_GRACE);
            let confirmed = tokio::time::timeout_at(grace, async {
                loop {
                    match traces.recv().await {
                        Ok(Trace::Halted(id) | Trace::Closed(id)) if id == stopping => {
                            return Ok(())
                        }
                        Ok(Trace::Stopped) | Err(broadcast::error::RecvError::Closed) => {
                            return Err(SchoolBrowserError::Stopped)
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    }
                }
            })
            .await;
            match confirmed {
                Ok(confirmed) => confirmed?,
                Err(_) => {
                    log::warn!(target: "app_core::school_browser", "view did not confirm its stop site={site:?}");
                    self.0.forget(stopping);
                    if tokio::time::Instant::now() >= deadline {
                        return Err(not_in_time());
                    }
                }
            }
        }
        self.ensure_browser(site, initial)
    }

    pub(super) async fn wait_until_hidden_without_context(&self) -> Result<(), SchoolBrowserError> {
        loop {
            if self.0.tasks.is_closed() {
                return Err(SchoolBrowserError::Stopped);
            }
            // Register before reading visibility: notify_waiters does not
            // retain a permit when the last visible page hides in this gap.
            let changed = self.0.browsing_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let visible = self.0.state.guard().browsing.is_some();
            if !visible {
                return Ok(());
            }
            tokio::select! {
                _ = &mut changed => {}
                _ = self.0.tasks.closed() => return Err(SchoolBrowserError::Stopped),
            }
        }
    }
}
