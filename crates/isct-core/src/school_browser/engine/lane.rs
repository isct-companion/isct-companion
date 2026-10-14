//! The single hidden lane: admission, priority and visibility yields.
use super::*;

pub(super) struct ManualWaiter<'a> {
    count: &'a AtomicUsize,
    changed: &'a Notify,
}

impl<'a> ManualWaiter<'a> {
    pub(super) fn new(count: &'a AtomicUsize, changed: &'a Notify) -> Self {
        count.fetch_add(1, Ordering::AcqRel);
        Self { count, changed }
    }
}

impl Drop for ManualWaiter<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }
}

impl SchoolBrowserEngine {
    /// Runs one coordinated pull in the hidden lane. Sign-in happens under the
    /// same lane admission, either with `credentials` supplied at the start or
    /// with saved credentials when a join later asks for `login_first`.
    pub async fn pull(
        &self,
        site: SchoolSite,
        course_id: Option<u64>,
        credentials: Option<(SchoolAuthProfile, SchoolCredentials)>,
        context: PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolBrowserError> {
        if !matches!(site, SchoolSite::ScienceLms | SchoolSite::Kyomu) {
            return Err(SchoolBrowserError::InvalidInput(
                "Only LMS and 教務Web can be pulled.".into(),
            ));
        }
        if course_id == Some(0) || (course_id.is_some() && site != SchoolSite::ScienceLms) {
            return Err(SchoolBrowserError::InvalidInput(
                "LMS course ID must be positive and use the LMS site.".into(),
            ));
        }
        let mut signed_in = false;
        loop {
            self.wait_until_hidden(&context).await?;
            let lane = self.acquire_pull_lane(&context).await?;
            // A page may have opened while this pull waited for the lane; every
            // hidden step below re-checks visibility and yields with Preempted.
            if context.is_cancelled() {
                return Err(SchoolBrowserError::Cancelled);
            }
            let result = if !signed_in && (credentials.is_some() || context.login_first()) {
                let (profile, saved) = match credentials.clone() {
                    Some(credentials) => credentials,
                    None => {
                        let profile = pull_profile(site);
                        (profile, self.saved_credentials(profile).await?)
                    }
                };
                if context.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                match self.login_locked(profile, saved).await {
                    Ok(_) => {
                        signed_in = true;
                        self.pull_once(site, course_id, &context).await
                    }
                    Err(error) => Err(error),
                }
            } else {
                self.pull_once(site, course_id, &context).await
            };
            drop(lane);
            match result {
                Err(SchoolBrowserError::Preempted) if context.is_cancelled() => {
                    return Err(SchoolBrowserError::Cancelled)
                }
                Err(SchoolBrowserError::Preempted) => continue,
                result => return result,
            }
        }
    }

    /// Waits for the hidden lane. The job's priority is read on every pass:
    /// a manual join promotes a job that is already waiting here.
    pub(super) async fn acquire_pull_lane(
        &self,
        context: &PullContext,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, SchoolBrowserError> {
        let mut manual_waiter = None;
        loop {
            if context.is_cancelled() {
                return Err(SchoolBrowserError::Cancelled);
            }
            if self.0.tasks.is_closed() {
                return Err(SchoolBrowserError::Stopped);
            }
            let manual = context.priority() == PullPriority::Manual;
            if manual && manual_waiter.is_none() {
                manual_waiter = Some(ManualWaiter::new(
                    &self.0.manual_waiters,
                    &self.0.priority_changed,
                ));
            }
            if manual || self.0.manual_waiters.load(Ordering::Acquire) == 0 {
                if let Ok(lane) = self.0.lane.try_lock() {
                    return Ok(lane);
                }
            }
            tokio::select! {
                _ = self.0.priority_changed.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                _ = self.0.tasks.closed() => return Err(SchoolBrowserError::Stopped),
            }
        }
    }

    pub(super) async fn saved_credentials(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<SchoolCredentials, SchoolBrowserError> {
        let auth = self.0.auth.clone();
        match tokio::task::spawn_blocking(move || auth.load(profile))
            .await
            .map_err(|_| SchoolBrowserError::Stopped)?
        {
            Ok(Some(credentials)) => Ok(credentials),
            Ok(None) => Err(SchoolBrowserError::Auth(
                "School credentials are not configured.".into(),
            )),
            Err(SchoolAuthError::Storage(message)) => Err(SchoolBrowserError::Storage(message)),
            Err(SchoolAuthError::InvalidInput(message)) => {
                Err(SchoolBrowserError::InvalidInput(message))
            }
            Err(error) => Err(SchoolBrowserError::Auth(error.to_string())),
        }
    }

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

    pub(super) async fn wait_until_hidden(
        &self,
        context: &PullContext,
    ) -> Result<(), SchoolBrowserError> {
        loop {
            if self.0.tasks.is_closed() {
                return Err(SchoolBrowserError::Stopped);
            }
            if context.is_cancelled() {
                return Err(SchoolBrowserError::Cancelled);
            }
            let visible = self.0.state.guard().browsing.is_some();
            if !visible {
                return Ok(());
            }
            tokio::select! {
                _ = self.0.browsing_changed.notified() => {}
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                _ = self.0.tasks.closed() => return Err(SchoolBrowserError::Stopped),
            }
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

pub(super) fn pull_profile(site: SchoolSite) -> SchoolAuthProfile {
    if site == SchoolSite::ScienceLms {
        SchoolAuthProfile::Science
    } else {
        SchoolAuthProfile::TokyoTech
    }
}
