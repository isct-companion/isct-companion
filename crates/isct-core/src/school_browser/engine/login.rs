//! Sign-in attempts, saved-credential renewal and session checks.
use super::*;

/// The page that starts a 教務Web session, and where it lands once started.
const KYOMU_ENTRY: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
const KYOMU_STUDENT: &str = "/Titech/Student/";

#[derive(Clone)]
pub(super) struct ActiveLogin {
    pub(super) profile: SchoolAuthProfile,
    pub(super) credentials: SchoolCredentials,
    pub(super) run: String,
}

impl SchoolBrowserEngine {
    pub fn credentials_changed(
        &self,
        profile: SchoolAuthProfile,
        configured: bool,
    ) -> Result<SchoolSessionSnapshot, SchoolBrowserError> {
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
        let run = new_id();
        let result = self.login_attempt(profile, credentials, &run).await;
        // No exit may leave this run active: a stale run blocks automatic
        // sign-in for the profile. A completed sign-in has already cleared it.
        self.clear_login(&run);
        result
    }

    pub(super) async fn login_attempt(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
        run: &str,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
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
            state.auto_blocked.remove(&profile);
            state.login = Some(ActiveLogin {
                profile,
                credentials,
                run: run.into(),
            });
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
        let mut state = self.0.state.guard();
        if state.login.as_ref().is_some_and(|login| login.run == run) {
            state.login = None;
            return true;
        }
        false
    }

    pub(super) fn fail_login(
        &self,
        profile: SchoolAuthProfile,
        run: &str,
        error: &SchoolBrowserError,
    ) {
        if !self.clear_login(run) {
            return;
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

    /// 教務Web starts its ASP.NET session only from its entry page, through
    /// the GetAccess sign-on; Rust cannot follow that hop itself. The hidden
    /// 教務 view opens the entry page, so the shared cookie store holds a
    /// fresh session for `net::kyomu`. `false`: it landed on a sign-in page.
    pub(crate) async fn enter_kyomu(&self) -> Result<bool, SchoolBrowserError> {
        loop {
            self.wait_until_hidden_without_context().await?;
            let lane = self.acquire_lane().await?;
            let result = self.enter_kyomu_once().await;
            drop(lane);
            log::info!(target: "isct_core::school_browser", "kyomu entry result={result:?}");
            match result {
                Err(SchoolBrowserError::Preempted) => continue,
                result => return result,
            }
        }
    }

    async fn enter_kyomu_once(&self) -> Result<bool, SchoolBrowserError> {
        let site = SchoolSite::Kyomu;
        let url = safe_url(KYOMU_ENTRY)?;
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
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        tokio::time::timeout(SESSION_TIMEOUT, async {
            loop {
                match traces.recv().await {
                    Ok(Trace::Loaded(event))
                        if event.page.browser == browser && event.page.sequence > baseline =>
                    {
                        if event.url.host_str() == Some(crate::school_parser::kyomu::HOST)
                            && event.url.path().starts_with(KYOMU_STUDENT)
                        {
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
        .map_err(|_| SchoolBrowserError::Timeout("教務Web did not open in time.".into()))?
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
        let (allowed, blocked_heartbeat) = self.state.with(|state| {
            (
                state.login.is_none() && !state.auto_blocked.contains(&profile),
                state.heartbeat == Some(profile) && state.auto_blocked.contains(&profile),
            )
        });
        if blocked_heartbeat {
            self.set_session_from_inner(profile, SchoolSessionState::Failed);
            return;
        }
        if !allowed {
            return;
        }
        let auth = self.auth.clone();
        let weak = Arc::downgrade(self);
        self.tasks.spawn(async move {
            let loaded = tokio::task::spawn_blocking(move || auth.load(profile)).await;
            let Some(inner) = weak.upgrade() else { return };
            let owned = inner.state.with(|state| {
                owns_sign_in(state, profile, page.browser)
                    && still_on_login_page(state, page, &expected_url)
            });
            if !owned {
                return;
            }
            let credentials = match loaded {
                Ok(Ok(Some(credentials))) => credentials,
                Ok(Ok(None)) => {
                    inner.set_session_from_inner(profile, SchoolSessionState::NotConfigured);
                    return;
                }
                Ok(Err(_)) | Err(_) => {
                    inner.set_session_from_inner(profile, SchoolSessionState::Failed);
                    return;
                }
            };
            let login = ActiveLogin {
                profile,
                credentials,
                run: new_id(),
            };
            let accepted = inner.state.with(|state| {
                if state.login.is_none()
                    && owns_sign_in(state, profile, page.browser)
                    && still_on_login_page(state, page, &expected_url)
                {
                    state.login = Some(login.clone());
                    true
                } else {
                    false
                }
            });
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
        let mut failed = false;
        let matched = self.state.with(|state| {
            let matched = state
                .login
                .as_ref()
                .is_some_and(|active| active.profile == relay.profile && active.run == relay.run);
            if matched && relay.state != "progress" && relay.state != "needs-user" {
                state.login = None;
                if !maintenance {
                    state.auto_blocked.insert(relay.profile);
                }
                failed = true;
            }
            matched
        });
        if matched {
            let message = clean_message(&relay.message);
            let _ = self.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
                profile: relay.profile,
                state: relay.state,
                message,
            }));
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
    state.heartbeat == Some(profile) || state.browsing == Some(browser.site)
}

pub(super) fn snapshot(state: &State) -> SchoolSessionSnapshot {
    let get = |profile| {
        state
            .sessions
            .get(&profile)
            .copied()
            .unwrap_or(SchoolSessionState::Pending)
    };
    SchoolSessionSnapshot {
        revision: state.revision,
        kyomu_maintenance: state.kyomu_maintenance.clone(),
        states: SchoolSessionStates {
            science: get(SchoolAuthProfile::Science),
            tokyo_tech: get(SchoolAuthProfile::TokyoTech),
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
