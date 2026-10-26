//! Sign-in attempts, saved-credential renewal and session checks.
use super::*;

/// The page that starts a 教務Web session, and where it lands once started.
const KYOMU_ENTRY: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
const KYOMU_STUDENT: &str = "/Titech/Student/";

/// A retryable failure waits at least one one-time-password period per
/// failure in a row, so the next attempt never resends a turned-down code.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// Retryable failures in a row before automatic sign-in waits for the person.
const MAX_RETRIES: u8 = 3;

#[derive(Clone)]
pub(super) struct ActiveLogin {
    pub(super) profile: SchoolAuthProfile,
    pub(super) credentials: SchoolCredentials,
    pub(super) run: String,
    /// Login form steps this run filled in and submitted.
    pub(super) submitted: HashSet<String>,
}

impl ActiveLogin {
    pub(super) fn new(profile: SchoolAuthProfile, credentials: SchoolCredentials) -> Self {
        Self {
            profile,
            credentials,
            run: new_id(),
            submitted: HashSet::new(),
        }
    }

    /// Whether the school saw every secret these credentials hold: only then
    /// does a success prove them, rather than a session that was still valid.
    fn proved(&self) -> bool {
        let second = match self.profile {
            SchoolAuthProfile::Science => "otp",
            SchoolAuthProfile::TokyoTech => "matrix",
        };
        self.submitted.contains("password") && self.submitted.contains(second)
    }
}

#[derive(Clone, Debug)]
pub(super) struct SignInRecord {
    problem: SchoolSignInProblem,
    auto: SchoolAutoSignIn,
    retry_at: Option<tokio::time::Instant>,
    /// Retryable failures in a row with these credentials.
    retries: u8,
    /// The credentials that failed. Any others lift the pause.
    credentials: SchoolCredentials,
}

impl SignInRecord {
    /// Core's retry policy. The school's verdict on a password, Matrix or
    /// lock waits for the person: resubmitting would only lock the account.
    /// A one-time password or a page warning may be a timing matter, so it
    /// is retried in a later window a few times.
    fn failed(
        previous: Option<&Self>,
        problem: SchoolSignInProblem,
        credentials: SchoolCredentials,
    ) -> Self {
        let retryable = matches!(
            problem.kind,
            SchoolSignInProblemKind::InvalidOtp
                | SchoolSignInProblemKind::PageWarning
                | SchoolSignInProblemKind::NeedsUser
        );
        let retries = if retryable {
            previous
                .filter(|previous| previous.credentials == credentials)
                .map_or(0, |previous| previous.retries)
                .saturating_add(1)
        } else {
            0
        };
        let auto = if !retryable || retries >= MAX_RETRIES {
            SchoolAutoSignIn::Paused
        } else {
            SchoolAutoSignIn::RetryLater
        };
        Self {
            problem,
            auto,
            retry_at: (auto == SchoolAutoSignIn::RetryLater)
                .then(|| tokio::time::Instant::now() + RETRY_AFTER * u32::from(retries)),
            retries,
            credentials,
        }
    }
}

/// Whether automatic sign-in may use `credentials` now. Credentials other
/// than the ones that failed (changed here, or synced from another device)
/// lift the record.
fn auto_allowed(
    state: &mut State,
    profile: SchoolAuthProfile,
    credentials: &SchoolCredentials,
) -> bool {
    let Some(record) = state.sign_in.get(&profile) else {
        return true;
    };
    if &record.credentials != credentials {
        state.sign_in.remove(&profile);
        return true;
    }
    match record.auto {
        SchoolAutoSignIn::Allowed => true,
        SchoolAutoSignIn::RetryLater => record
            .retry_at
            .is_none_or(|at| tokio::time::Instant::now() >= at),
        SchoolAutoSignIn::Paused => false,
    }
}

fn problem_kind(state: &str) -> Option<SchoolSignInProblemKind> {
    Some(match state {
        "invalid-credentials" => SchoolSignInProblemKind::InvalidCredentials,
        "locked" => SchoolSignInProblemKind::Locked,
        "invalid-otp" => SchoolSignInProblemKind::InvalidOtp,
        "invalid-matrix" => SchoolSignInProblemKind::InvalidMatrix,
        "error" => SchoolSignInProblemKind::PageWarning,
        "needs-user" => SchoolSignInProblemKind::NeedsUser,
        _ => return None,
    })
}

impl SchoolBrowserEngine {
    pub fn credentials_changed(
        &self,
        profile: SchoolAuthProfile,
        configured: bool,
    ) -> Result<SchoolSessionSnapshot, SchoolBrowserError> {
        // New credentials are the person's answer to whatever failed before.
        self.0.state.guard().sign_in.remove(&profile);
        self.set_session(
            profile,
            if configured {
                SchoolSessionState::Pending
            } else {
                SchoolSessionState::NotConfigured
            },
        );
        self.session_snapshot()
    }

    pub async fn login(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let _lane = self.acquire_lane().await?;
        loop {
            match self.login_locked(profile, credentials.clone()).await {
                // A visible page paused this sign-in. Resume it once the page
                // hides instead of reporting the pause as a sign-in failure.
                Err(SchoolBrowserError::Preempted) => continue,
                result => return result,
            }
        }
    }

    /// Signs in with the credentials the person just entered. A failure is
    /// a result, with Core's reason, not an error.
    pub async fn connect(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolConnectResult, SchoolBrowserError> {
        let outcome = self.login(profile, credentials.clone()).await;
        if let Err(error @ (SchoolBrowserError::Stopped | SchoolBrowserError::InvalidInput(_))) =
            outcome
        {
            return Err(error);
        }
        let auth = self.0.auth.clone();
        let verified = outcome.is_ok()
            && tokio::task::spawn_blocking(move || auth.is_verified(profile, &credentials))
                .await
                .is_ok_and(|verified| verified.unwrap_or(false));
        let session = profile_session(&self.0.state.guard(), profile);
        Ok(SchoolConnectResult {
            state: session.state,
            problem: session.problem,
            verified,
        })
    }

    pub async fn ensure_session(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        loop {
            self.wait_until_hidden_without_context().await?;
            let started = Instant::now();
            log::info!(target: "isct_core::school_browser", "session check waiting profile={profile:?}");
            let lane = self.acquire_lane().await?;
            log::info!(target: "isct_core::school_browser", "session check acquired profile={profile:?} waited_ms={}", started.elapsed().as_millis());
            let result = self.ensure_session_once(profile).await;
            drop(lane);
            log::info!(target: "isct_core::school_browser", "session check finished profile={profile:?} elapsed_ms={} result={result:?}", started.elapsed().as_millis());
            match result {
                Err(SchoolBrowserError::Preempted) => continue,
                result => return result,
            }
        }
    }

    /// Admit a background session check without making the native caller own
    /// its lifetime. Repeated requests for the same active or pending profile
    /// coalesce into the existing check.
    pub fn request_session(
        self: &Arc<Self>,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        {
            let mut state = self.0.state.guard();
            if !state.pending_profiles.insert(profile) {
                return Ok(false);
            }
        }
        let engine = self.clone();
        let _entered = self.0.runtime.enter();
        if !self.0.tasks.spawn(async move {
            let _ = engine.ensure_session(profile).await;
            {
                let mut state = engine.0.state.guard();
                state.pending_profiles.remove(&profile);
            }
        }) {
            {
                let mut state = self.0.state.guard();
                state.pending_profiles.remove(&profile);
            }
            return Err(SchoolBrowserError::Stopped);
        }
        Ok(true)
    }

    pub async fn launch_sessions(&self) -> Result<SchoolSessionSnapshot, SchoolBrowserError> {
        let epoch = self.0.state.guard().lifecycle_epoch;
        let _run = self.0.lifecycle.lock().await;
        if self.0.state.guard().lifecycle_epoch != epoch {
            return self.session_snapshot();
        }
        {
            let mut state = self.0.state.guard();
            let now = Instant::now();
            if state
                .last_launch
                .is_some_and(|last| now.duration_since(last) < LAUNCH_INTERVAL)
            {
                return Ok(snapshot(&state));
            }
            state.last_launch = Some(now);
        }
        for profile in [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech] {
            let _ = self.ensure_session(profile).await;
        }
        {
            let mut state = self.0.state.guard();
            state.lifecycle_epoch = state.lifecycle_epoch.saturating_add(1);
        }
        self.session_snapshot()
    }

    pub async fn focus_sessions(
        &self,
    ) -> Result<Option<SchoolSessionSnapshot>, SchoolBrowserError> {
        {
            let mut state = self.0.state.guard();
            let now = Instant::now();
            if state
                .last_focus
                .is_some_and(|last| now.duration_since(last) < FOCUS_INTERVAL)
            {
                return Ok(None);
            }
            state.last_focus = Some(now);
        }
        self.launch_sessions().await.map(Some)
    }

    pub async fn lifecycle(&self, event: SchoolSessionLifecycle) -> Result<(), SchoolBrowserError> {
        match event.kind {
            SchoolSessionLifecycleKind::Launch => {
                self.launch_sessions().await?;
            }
            SchoolSessionLifecycleKind::Focus if event.visible => {
                self.focus_sessions().await?;
            }
            SchoolSessionLifecycleKind::Focus => {}
        }
        Ok(())
    }

    pub(super) async fn login_locked(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let login = ActiveLogin::new(profile, credentials);
        let run = login.run.clone();
        let result = self.login_attempt(login).await;
        // No exit may leave this run active: a stale run blocks automatic
        // sign-in for the profile. A completed sign-in has already cleared it.
        self.clear_login(&run);
        result
    }

    pub(super) async fn login_attempt(
        &self,
        login: ActiveLogin,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let profile = login.profile;
        let run = &login.run.clone();
        self.wait_until_hidden_without_context().await?;
        let site = profile.start_site();
        let url = safe_url(site.start_url())?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, url.clone())?;
        {
            let mut state = self.0.state.guard();
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            // Signing in on request is how the person retries after a pause.
            state.sign_in.remove(&profile);
            state.login = Some(login);
        }
        self.set_session(profile, SchoolSessionState::Pending);
        if let Err(error) = self.dispatch(browser, BrowserAction::Navigate(url)) {
            self.fail_login(profile, run, &error);
            return Err(error);
        }
        let state = tokio::time::timeout(SESSION_TIMEOUT, async {
            loop {
                match traces.recv().await {
                    // Pending only restates that this attempt is under way.
                    Ok(Trace::Session {
                        profile: found,
                        state,
                    }) if found == profile && state != SchoolSessionState::Pending => {
                        return Ok(state)
                    }
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Stopped) => return Err(SchoolBrowserError::Stopped),
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(SchoolBrowserError::Stopped)
                    }
                }
            }
        })
        .await
        .map_err(|_| {
            SchoolBrowserError::Timeout("The queued school sign-in did not start in time.".into())
        });
        let state = match state {
            Ok(Ok(state)) => state,
            Ok(Err(error)) => {
                if !matches!(
                    error,
                    SchoolBrowserError::Preempted | SchoolBrowserError::Stopped
                ) {
                    self.fail_login(profile, run, &error);
                }
                return Err(error);
            }
            Err(error) => {
                self.fail_login(profile, run, &error);
                return Err(error);
            }
        };
        if state != SchoolSessionState::Active {
            return Err(SchoolBrowserError::Auth("School sign-in failed.".into()));
        }
        Ok(SchoolWebResult {
            site,
            url: site.start_url().into(),
            phase: "sessionRestored",
        })
    }

    pub(super) fn clear_login(&self, run: &str) -> bool {
        self.take_login(run).is_some()
    }

    fn take_login(&self, run: &str) -> Option<ActiveLogin> {
        let mut state = self.0.state.guard();
        if state.login.as_ref().is_some_and(|login| login.run == run) {
            return state.login.take();
        }
        None
    }

    pub(super) fn fail_login(
        &self,
        profile: SchoolAuthProfile,
        run: &str,
        error: &SchoolBrowserError,
    ) {
        let Some(login) = self.take_login(run) else {
            return;
        };
        if !matches!(error, SchoolBrowserError::Maintenance(_)) {
            // A page that did not answer is not the school's verdict: keep
            // what the school said before, or note the outage without
            // holding automatic sign-in back.
            let mut state = self.0.state.guard();
            if state.sign_in.get(&profile).is_none_or(|record| {
                record.auto == SchoolAutoSignIn::Allowed
                    && record.problem.kind != SchoolSignInProblemKind::NeedsUser
            }) {
                state.sign_in.insert(
                    profile,
                    SignInRecord {
                        problem: SchoolSignInProblem {
                            kind: SchoolSignInProblemKind::Unavailable,
                            message: error.to_string(),
                        },
                        auto: SchoolAutoSignIn::Allowed,
                        retry_at: None,
                        retries: 0,
                        credentials: login.credentials,
                    },
                );
            }
        }
        let _ = self.0.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
            profile,
            state: if matches!(error, SchoolBrowserError::Maintenance(_)) {
                "maintenance"
            } else {
                "error"
            }
            .into(),
            message: error.to_string(),
        }));
        self.set_session(
            profile,
            if matches!(error, SchoolBrowserError::Maintenance(_)) {
                SchoolSessionState::Maintenance
            } else {
                SchoolSessionState::Failed
            },
        );
    }

    /// Portal authentication alone does not create the application's cookie:
    /// the resident LMS/教務 view must finish its SSO hop before HTTP retries.
    pub(crate) async fn ensure_transport_session(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        if !self.ensure_session(profile).await? {
            return Ok(false);
        }
        loop {
            self.wait_until_hidden_without_context().await?;
            let lane = self.acquire_lane().await?;
            let result = self.enter_service_once(profile).await;
            drop(lane);
            log::info!(target: "isct_core::school_browser", "service entry profile={profile:?} result={result:?}");
            match result {
                Err(SchoolBrowserError::Preempted) => continue,
                result => return result,
            }
        }
    }

    async fn enter_service_once(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        let (site, entry) = match profile {
            SchoolAuthProfile::Science => (SchoolSite::ScienceLms, crate::lms_session::LMS_HOME),
            SchoolAuthProfile::TokyoTech => (SchoolSite::Kyomu, KYOMU_ENTRY),
        };
        let url = safe_url(entry)?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, url.clone())?;
        let baseline = {
            let mut state = self.0.state.guard();
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            let view = state.views.get_mut(&site);
            view.map(|view| {
                view.background_navigation = true;
                view.last_page
            })
            .unwrap_or_default()
        };
        self.dispatch(browser, BrowserAction::Navigate(url.clone()))?;
        tokio::time::timeout(SESSION_TIMEOUT, async {
            loop {
                match traces.recv().await {
                    Ok(Trace::Loaded(event))
                        if event.page.browser == browser && event.page.sequence > baseline =>
                    {
                        let ready = match profile {
                            SchoolAuthProfile::Science => {
                                crate::school_parser::lms::is_lms_origin(&event.url)
                                    && (event.url.path() == url.path()
                                        || event.url.path().strip_suffix("index.php")
                                            == Some(url.path()))
                            }
                            SchoolAuthProfile::TokyoTech => {
                                event.url.host_str() == Some(crate::school_parser::kyomu::HOST)
                                    && event.url.path().starts_with(KYOMU_STUDENT)
                            }
                        };
                        if ready {
                            return Ok(true);
                        }
                        // Sign-on hops in between (GetAccess, form posts)
                        // also load pages; only a sign-in form ends the wait.
                        if is_identity_login(&event.url) {
                            return Ok(false);
                        }
                    }
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Stopped) | Err(broadcast::error::RecvError::Closed) => {
                        return Err(SchoolBrowserError::Stopped)
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        })
        .await
        .map_err(|_| SchoolBrowserError::Timeout(format!("{site:?} did not open in time.")))?
    }

    pub(super) async fn ensure_session_once(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        let site = profile.start_site();
        let url = safe_url(site.start_url())?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, url.clone())?;
        {
            let mut state = self.0.state.guard();
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            state.heartbeat = Some(profile);
        }
        // Credentials another device saved since the last check sign in here.
        self.0.auth.refresh();
        self.set_session(profile, SchoolSessionState::Pending);
        if let Err(error) = self.dispatch(browser, BrowserAction::Navigate(url)) {
            self.fail_heartbeat(profile);
            return Err(error);
        }
        let result = tokio::time::timeout(SESSION_TIMEOUT, async {
            loop {
                match traces.recv().await {
                    Ok(Trace::Session {
                        profile: found,
                        state,
                    }) if found == profile && state != SchoolSessionState::Pending => {
                        return Ok(state == SchoolSessionState::Active)
                    }
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Stopped) | Err(broadcast::error::RecvError::Closed) => {
                        return Err(SchoolBrowserError::Stopped)
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
        })
        .await
        .map_err(|_| {
            SchoolBrowserError::Timeout("The queued school sign-in did not start in time.".into())
        });
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.fail_heartbeat(profile);
                return Err(error);
            }
        };
        if let Err(error) = &result {
            if matches!(
                error,
                SchoolBrowserError::Preempted | SchoolBrowserError::Stopped
            ) {
                self.clear_heartbeat(profile);
            } else if matches!(error, SchoolBrowserError::Maintenance(_)) {
                self.clear_heartbeat(profile);
                self.set_session(profile, SchoolSessionState::Maintenance);
            } else {
                self.fail_heartbeat(profile);
            }
        }
        result
    }

    pub(super) fn clear_heartbeat(&self, profile: SchoolAuthProfile) -> bool {
        let mut state = self.0.state.guard();
        if state.heartbeat != Some(profile) {
            return false;
        }
        state.heartbeat = None;
        if state
            .login
            .as_ref()
            .is_some_and(|login| login.profile == profile)
        {
            state.login = None;
        }
        true
    }

    pub(super) fn fail_heartbeat(&self, profile: SchoolAuthProfile) {
        if self.clear_heartbeat(profile) {
            self.set_session(profile, SchoolSessionState::Failed);
        }
    }
}

impl Inner {
    pub(super) fn arm_saved_login(self: &Arc<Self>, page: PageId, url: Url, relay: String) {
        let Some(profile) = url.host_str().and_then(SchoolAuthProfile::for_auth_host) else {
            return;
        };
        let Some(expected_url) = sanitize_school_url(url.as_str()) else {
            return;
        };
        if self.state.with(|state| state.login.is_some()) {
            return;
        }
        let auth = self.auth.clone();
        let weak = Arc::downgrade(self);
        self.tasks.spawn(async move {
            // Read even when paused: credentials synced from another device
            // since the failure lift the pause.
            let loaded =
                tokio::task::spawn_blocking(move || auth.sign_in_credentials(profile)).await;
            let Some(inner) = weak.upgrade() else { return };
            let owned = inner.state.with(|state| {
                owns_sign_in(state, profile, page.browser)
                    && still_on_login_page(state, page, &expected_url)
            });
            if !owned {
                return;
            }
            let saved = match loaded {
                Ok(Ok(Some(saved))) => saved,
                Ok(Ok(None)) => {
                    inner.set_session_from_inner(profile, SchoolSessionState::NotConfigured);
                    return;
                }
                Ok(Err(_)) | Err(_) => {
                    inner.set_session_from_inner(profile, SchoolSessionState::Failed);
                    return;
                }
            };
            let login = ActiveLogin::new(profile, saved.credentials.clone());
            let (accepted, blocked_heartbeat, lifted) = inner.state.with(|state| {
                if state.login.is_some()
                    || !owns_sign_in(state, profile, page.browser)
                    || !still_on_login_page(state, page, &expected_url)
                {
                    return (false, false, false);
                }
                // A relaunch remembers what the school said about a candidate.
                if let Some(problem) = saved.failure.clone() {
                    state.sign_in.entry(profile).or_insert_with(|| {
                        SignInRecord::failed(None, problem, saved.credentials.clone())
                    });
                }
                let blocked = state.sign_in.contains_key(&profile);
                if !auto_allowed(state, profile, &saved.credentials) {
                    return (false, state.heartbeat == Some(profile), false);
                }
                state.login = Some(login.clone());
                (
                    true,
                    false,
                    blocked && !state.sign_in.contains_key(&profile),
                )
            });
            if lifted {
                // The account the synced data belongs to may have changed.
                if let Some(sync) = inner.hints.engine() {
                    sync.reevaluate();
                }
            }
            if blocked_heartbeat {
                inner.set_session_from_inner(profile, SchoolSessionState::Failed);
                return;
            }
            if accepted {
                if let Some(script) = login_script(url.as_str(), &login, &relay) {
                    let _ = inner.dispatch_eval(page.browser, script, true);
                }
            }
        });
    }

    pub(super) fn login_relay(&self, encoded: &str) {
        if encoded.len() > 4 * 1024 {
            return;
        }
        let Some(relay) = URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<LoginRelay>(&bytes).ok())
        else {
            return;
        };
        if !valid_login_relay(&relay.state, &relay.message) {
            return;
        }
        let maintenance = relay.state == "maintenance";
        let message = clean_message(&relay.message);
        let problem = problem_kind(&relay.state).map(|kind| SchoolSignInProblem {
            kind,
            message: message.clone(),
        });
        let mut failed = false;
        let mut needs_user = false;
        let mut paused = None;
        let matched = self.state.with(|state| {
            let Some(active) = state
                .login
                .as_mut()
                .filter(|active| active.profile == relay.profile && active.run == relay.run)
            else {
                return false;
            };
            let steps = relay
                .submitted
                .iter()
                .map(String::as_str)
                .chain((relay.state == "progress").then_some(relay.message.as_str()));
            for step in steps {
                if valid_login_relay("progress", step) {
                    active.submitted.insert(step.into());
                }
            }
            let credentials = active.credentials.clone();
            if relay.state == "progress" {
                return true;
            }
            // The page waits for the person; the run stays theirs to finish.
            if relay.state != "needs-user" {
                state.login = None;
                failed = true;
            } else {
                needs_user = true;
            }
            if let Some(problem) = problem.clone() {
                let record =
                    SignInRecord::failed(state.sign_in.get(&relay.profile), problem, credentials);
                if record.auto == SchoolAutoSignIn::Paused {
                    paused = Some((record.credentials.clone(), record.problem.clone()));
                }
                state.sign_in.insert(relay.profile, record);
            }
            true
        });
        if matched {
            let _ = self.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
                profile: relay.profile,
                state: relay.state,
                message,
            }));
        }
        if failed && !maintenance {
            // The school may be turning down what another device replaced.
            self.auth.refresh();
        }
        if let Some((credentials, problem)) = paused {
            // Kept with a candidate, so a relaunch does not submit it again.
            let auth = self.auth.clone();
            let profile = relay.profile;
            self.tasks.spawn(async move {
                let _ = tokio::task::spawn_blocking(move || {
                    auth.record_failure(profile, &credentials, &problem)
                })
                .await;
            });
        }
        if needs_user {
            // Still pending, now with the reason shells show.
            self.set_session_from_inner(relay.profile, SchoolSessionState::Pending);
        }
        if failed {
            self.set_session_from_inner(
                relay.profile,
                if maintenance {
                    SchoolSessionState::Maintenance
                } else {
                    SchoolSessionState::Failed
                },
            );
        }
    }

    /// The school's own page after the sign-in form: the session is live.
    /// Credentials the school just saw in full become the verified ones
    /// before the session counts as signed in, so a connect that reports
    /// success has already promoted them.
    pub(super) fn finish_login(self: &Arc<Self>, active: ActiveLogin) {
        let finished = self.state.with(|state| {
            if state
                .login
                .as_ref()
                .is_some_and(|login| login.run == active.run)
            {
                state.sign_in.remove(&active.profile);
                state.login.take()
            } else {
                None
            }
        });
        let _ = self.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
            profile: active.profile,
            state: "connected".into(),
            message: "Connected".into(),
        }));
        let Some(finished) = finished.filter(ActiveLogin::proved) else {
            self.set_session_from_inner(active.profile, SchoolSessionState::Active);
            return;
        };
        let auth = self.auth.clone();
        let weak = Arc::downgrade(self);
        let spawned = self.tasks.spawn(async move {
            let profile = finished.profile;
            let promoted = tokio::task::spawn_blocking(move || {
                auth.promote(finished.profile, &finished.credentials)
            })
            .await;
            if let Ok(Err(error)) = &promoted {
                log::warn!(target: "isct_core::school_browser", "verified credentials not saved profile={profile:?}: {error}");
            }
            if let Some(inner) = weak.upgrade() {
                inner.set_session_from_inner(profile, SchoolSessionState::Active);
            }
        });
        if !spawned {
            self.set_session_from_inner(active.profile, SchoolSessionState::Active);
        }
    }

    pub(super) fn set_session_from_inner(
        &self,
        profile: SchoolAuthProfile,
        value: SchoolSessionState,
    ) {
        let event = self.state.with(|state| {
            if value != SchoolSessionState::Pending && state.heartbeat == Some(profile) {
                state.heartbeat = None;
                if state
                    .login
                    .as_ref()
                    .is_some_and(|login| login.profile == profile)
                {
                    state.login = None;
                }
            }
            state.sessions.insert(profile, value);
            state.revision = state.revision.saturating_add(1);
            SchoolSessionEvent {
                profile,
                state: value,
                revision: state.revision,
            }
        });
        if value == SchoolSessionState::Active {
            self.hints.send(Hint::SignedIn { profile });
        }
        let _ = self.events.emit(CoreEvent::SchoolSession(event));
        let _ = self.traces.send(Trace::Session {
            profile,
            state: value,
        });
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRelay {
    profile: SchoolAuthProfile,
    run: String,
    state: String,
    message: String,
    /// Every step this run has submitted so far, across the page loads.
    #[serde(default)]
    submitted: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginConfig<'a> {
    profile: &'static str,
    run: &'a str,
    username: &'a str,
    password: &'a str,
    otp: Vec<OtpWindow>,
    matrix: &'a str,
    auth_relay: &'a str,
    debug_relay: Option<&'a str>,
}

pub(super) fn login_script(raw: &str, login: &ActiveLogin, relay: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str().and_then(SchoolAuthProfile::for_auth_host) != Some(login.profile)
        || !is_login_step(&url)
    {
        return None;
    }
    let credentials = &login.credentials;
    let otp = if credentials.otp_secret.is_empty() {
        Vec::new()
    } else {
        upcoming_otps(&credentials.otp_secret).ok()?
    };
    let profile = match login.profile {
        SchoolAuthProfile::Science => "science",
        SchoolAuthProfile::TokyoTech => "tokyo-tech",
    };
    let config = serde_json::to_string(&LoginConfig {
        profile,
        run: &login.run,
        username: &credentials.username,
        password: &credentials.password,
        otp,
        matrix: &credentials.matrix,
        auth_relay: relay,
        debug_relay: None,
    })
    .ok()?
    .replace('<', "\\u003c")
    .replace('>', "\\u003e");
    Some(SCHOOL_LOGIN.replace("__ISCT_COMPANION_LOGIN_CONFIG__", &config))
}

/// Whether the view still shows the sign-in page that armed a saved login. A
/// visible view records the full browsing URL, a hidden one the sanitized URL,
/// so both sides are compared in sanitized form: the GetAccess agreement page
/// (`portal.pl?GASF=...`) loses its whole query when sanitized.
fn still_on_login_page(state: &State, page: PageId, expected_url: &str) -> bool {
    state.views.get(&page.browser.site).is_some_and(|view| {
        view.id == page.browser
            && view.last_page == page.sequence
            && sanitize_school_url(&view.url).as_deref() == Some(expected_url)
    })
}

/// Automatic sign-in belongs to the session check, the visible page or the
/// hidden pull that led this view to the identity provider.
pub(super) fn owns_sign_in(state: &State, profile: SchoolAuthProfile, browser: BrowserId) -> bool {
    if profile == SchoolAuthProfile::Science && state.otp_setup.is_some() {
        return false;
    }
    state.heartbeat == Some(profile) || state.browsing == Some(browser.site)
}

pub(super) fn profile_session(state: &State, profile: SchoolAuthProfile) -> SchoolProfileSession {
    let record = state.sign_in.get(&profile);
    SchoolProfileSession {
        state: state
            .sessions
            .get(&profile)
            .copied()
            .unwrap_or(SchoolSessionState::Pending),
        problem: record.map(|record| record.problem.clone()),
        auto: record.map_or(SchoolAutoSignIn::Allowed, |record| record.auto),
    }
}

pub(super) fn snapshot(state: &State) -> SchoolSessionSnapshot {
    SchoolSessionSnapshot {
        revision: state.revision,
        kyomu_maintenance: state.kyomu_maintenance.clone(),
        profiles: SchoolSessionProfiles {
            science: profile_session(state, SchoolAuthProfile::Science),
            tokyo_tech: profile_session(state, SchoolAuthProfile::TokyoTech),
        },
    }
}

pub(super) fn login_succeeded(raw: &str, profile: SchoolAuthProfile) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default();
    match profile {
        SchoolAuthProfile::Science => {
            (host == "isct.ex-tic.com" && url.path().starts_with("/idm/user/portal"))
                || (host == "lms.s.isct.ac.jp"
                    && !url.path().contains("/login")
                    && !url.path().contains("saml"))
        }
        SchoolAuthProfile::TokyoTech => {
            (host == "portal.nap.gsic.titech.ac.jp" && url.path().contains("ResourceList"))
                || host == "kyomu0.gakumu.titech.ac.jp"
                || (host == "portal.t4.gsic.titech.ac.jp"
                    && url.path().starts_with("/ptl/")
                    && !matches!(url.path(), "/ptl/" | "/ptl/requestLogin")
                    && !url.path().starts_with("/ptl/ga/"))
        }
    }
}

pub(super) fn valid_login_relay(state: &str, message: &str) -> bool {
    match state {
        "progress" => matches!(
            message,
            "agree" | "identifier" | "password" | "method" | "otp" | "matrix"
        ),
        "needs-user"
        | "error"
        | "invalid-credentials"
        | "invalid-otp"
        | "invalid-matrix"
        | "locked"
        | "maintenance" => true,
        _ => false,
    }
}

pub(super) fn clean_message(message: &str) -> String {
    let cleaned: String = message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(300)
        .collect();
    if cleaned.trim().is_empty() {
        "The school portal did not provide an error message.".into()
    } else {
        cleaned
    }
}
