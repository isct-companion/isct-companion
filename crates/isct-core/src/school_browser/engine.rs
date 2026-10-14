use super::policy::{is_identity_login, is_login_step, sanitize_school_url};
use super::{
    assets::SCHOOL_LOGIN, ActionId, BrowserAction, BrowserCommand, BrowserError, BrowserId,
    LoadFailed, NavigationDecision, NewWindowDecision, PageId, PageLoaded, SchoolBrowser,
    SchoolBrowserError, SchoolBrowserListener, SchoolLoginEvent, SchoolPageCustomization,
    SchoolPullProgress, SchoolSessionEvent, SchoolSessionLifecycle, SchoolSessionLifecycleKind,
    SchoolSessionSnapshot, SchoolSessionState, SchoolSessionStates, SchoolWebResult,
};
use crate::{
    data::DataStore,
    events::CoreEvent,
    kyomu_ingest::{apply_kyomu_payload, KYOMU_EXTRACT_SCRIPT},
    lifecycle::SchoolSite,
    lms_ingest::{apply_lms_payload, LMS_EXTRACT_SCRIPT},
    ports::EventSink,
    pull_coordinator::{PullContext, PullPriority},
    runtime_tasks::RuntimeTasks,
    school_auth::{
        upcoming_otps, OtpWindow, SchoolAuthError, SchoolAuthProfile, SchoolAuthService,
        SchoolCredentials,
    },
    school_parser::{
        kyomu::{parse_payload as parse_kyomu_payload, KyomuStage},
        lms::{parse_payload as parse_lms_payload, LmsStage},
    },
    school_refresh::{RefreshEvent, SchoolRefresh},
    school_service::{IngestOutcome, SchoolExecution, SchoolExecutor, SchoolExecutorError},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard, PoisonError, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};
use url::Url;

const KYOMU_ROOT: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/";
const KYOMU_PULL_PAGES: [&str; 5] = [
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
    "Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
    "Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
    "Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
];
const KYOMU_PULL_KINDS: [&str; 5] = ["gpa", "registrations", "declared", "grades", "requirements"];
const SESSION_TIMEOUT: Duration = Duration::from_secs(90);
const LAUNCH_INTERVAL: Duration = Duration::from_secs(90);
const OPEN_TIMEOUT: Duration = Duration::from_secs(150);
const PAGE_TIMEOUT: Duration = Duration::from_secs(60);
const FOCUS_INTERVAL: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct SchoolBrowserEngine(Arc<Inner>);

/// Engine state is plain data that every critical section leaves consistent,
/// so a panic elsewhere while a lock was held must not turn into a permanent
/// "stopped" browser. Every lock in the engine recovers the guard instead.
trait Guarded<T> {
    fn guard(&self) -> MutexGuard<'_, T>;
    fn with<R>(&self, work: impl FnOnce(&mut T) -> R) -> R {
        work(&mut self.guard())
    }
}

impl<T> Guarded<T> for Mutex<T> {
    fn guard(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Inner {
    store: DataStore,
    auth: Arc<SchoolAuthService>,
    events: Arc<dyn EventSink>,
    refresh: SchoolRefresh,
    browser: Arc<dyn SchoolBrowser>,
    state: Mutex<State>,
    lane: AsyncMutex<()>,
    lifecycle: AsyncMutex<()>,
    traces: broadcast::Sender<Trace>,
    browsing_changed: Notify,
    priority_changed: Notify,
    tasks: RuntimeTasks,
    runtime: tokio::runtime::Handle,
    next_generation: AtomicU64,
    next_action: AtomicU64,
    manual_waiters: AtomicUsize,
    listener: Mutex<Option<Arc<dyn SchoolBrowserListener>>>,
    restore_io: Mutex<()>,
}

#[derive(Default)]
struct State {
    views: HashMap<SchoolSite, ViewState>,
    browsing: Option<SchoolSite>,
    login: Option<ActiveLogin>,
    auto_blocked: HashSet<SchoolAuthProfile>,
    extraction: Option<Extraction>,
    hidden: Option<HiddenWork>,
    sessions: HashMap<SchoolAuthProfile, SchoolSessionState>,
    revision: u64,
    last_focus: Option<Instant>,
    last_launch: Option<Instant>,
    heartbeat: Option<SchoolAuthProfile>,
    pending_profiles: HashSet<SchoolAuthProfile>,
    lifecycle_epoch: u64,
    customizations: HashMap<SchoolSite, SchoolPageCustomization>,
    guide: Vec<crate::kyomu_guide::GuideItem>,
    history: HashMap<SchoolSite, PageHistory>,
    suppress_history: HashSet<SchoolSite>,
    popup_return: HashMap<SchoolSite, PopupReturn>,
    restore: Option<(std::path::PathBuf, u64)>,
    restore_epoch: u64,
}

struct ViewState {
    id: BrowserId,
    last_page: u64,
    url: String,
    extraction_relay: String,
    auth_relay: String,
    progress_relay: String,
    navigation: Option<ActionId>,
    actions: HashMap<ActionId, bool>,
}

#[derive(Default)]
struct PageHistory {
    pages: Vec<String>,
    forward: Vec<String>,
}

#[derive(Clone)]
struct PopupReturn {
    opener: String,
    popup: String,
}

impl PageHistory {
    fn push(&mut self, url: String) {
        if self.pages.last() == Some(&url) {
            return;
        }
        self.pages.push(url);
        self.forward.clear();
        if self.pages.len() > 50 {
            self.pages.remove(0);
        }
    }
    fn back_target(&mut self, current: Option<&str>) -> Option<String> {
        let index = self
            .pages
            .iter()
            .rposition(|page| Some(page.as_str()) != current)?;
        if let Some(left) = self.pages.drain(index + 1..).next_back() {
            self.forward.push(left);
        }
        self.pages.last().cloned()
    }
    fn forward_target(&mut self) -> Option<String> {
        let next = self.forward.pop()?;
        self.pages.push(next.clone());
        Some(next)
    }
}

#[derive(Deserialize, Serialize)]
struct RestoreRecord {
    site: SchoolSite,
    url: String,
}

#[derive(Clone)]
struct ActiveLogin {
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
    run: String,
}

#[derive(Clone)]
struct Extraction {
    browser: BrowserId,
    run: String,
    course_id: Option<u64>,
    full: bool,
    baseline: u64,
    target: String,
    visible: bool,
}

/// A hidden pull navigation. It owns automatic sign-in in its view and is
/// resumed at `target` when that sign-in completes elsewhere.
struct HiddenWork {
    browser: BrowserId,
    run: String,
    target: Url,
    prefix: Option<String>,
}

impl HiddenWork {
    fn reached(&self, url: &Url) -> bool {
        match &self.prefix {
            Some(prefix) => url.as_str().starts_with(prefix.as_str()),
            None => url == &self.target,
        }
    }
}

/// Clears the hidden navigation and its unconsumed extraction on every exit.
struct HiddenGuard<'a> {
    inner: &'a Inner,
    run: String,
}

impl Drop for HiddenGuard<'_> {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state.guard();
            if state
                .hidden
                .as_ref()
                .is_some_and(|work| work.run == self.run)
            {
                state.hidden = None;
            }
            if state
                .extraction
                .as_ref()
                .is_some_and(|extraction| extraction.run == self.run)
            {
                state.extraction = None;
            }
        }
    }
}

/// How a hidden navigation ended when it did not fail.
enum HiddenLoad<T> {
    Ready(T),
    /// The view landed on a sign-in page that saved credentials cannot pass.
    SignInRequired,
}

#[derive(Clone, Debug)]
enum Trace {
    Loaded(PageLoaded),
    Failed(LoadFailed),
    Extraction {
        browser: BrowserId,
        run: String,
        raw: String,
    },
    Session {
        profile: SchoolAuthProfile,
        state: SchoolSessionState,
    },
    /// The visible site changed. `Some` preempts hidden work.
    Browsing(Option<SchoolSite>),
    Closed(BrowserId),
    Stopped,
}

struct Listener(Weak<Inner>);

struct ManualWaiter<'a> {
    count: &'a AtomicUsize,
    changed: &'a Notify,
}

impl<'a> ManualWaiter<'a> {
    fn new(count: &'a AtomicUsize, changed: &'a Notify) -> Self {
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
    pub fn new(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        events: Arc<dyn EventSink>,
        browser: Arc<dyn SchoolBrowser>,
        refresh: SchoolRefresh,
    ) -> Result<Arc<Self>, SchoolBrowserError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            SchoolBrowserError::Platform("School browser requires an async runtime.".into())
        })?;
        let (traces, _) = broadcast::channel(128);
        let inner = Arc::new(Inner {
            store,
            auth,
            events,
            refresh,
            browser: browser.clone(),
            state: Mutex::new(State::default()),
            lane: AsyncMutex::new(()),
            lifecycle: AsyncMutex::new(()),
            traces,
            browsing_changed: Notify::new(),
            priority_changed: Notify::new(),
            tasks: RuntimeTasks::default(),
            runtime,
            next_generation: AtomicU64::new(1),
            next_action: AtomicU64::new(1),
            manual_waiters: AtomicUsize::new(0),
            listener: Mutex::new(None),
            restore_io: Mutex::new(()),
        });
        let listener: Arc<dyn SchoolBrowserListener> = Arc::new(Listener(Arc::downgrade(&inner)));
        browser.attach(Arc::downgrade(&listener))?;
        inner.listener.guard().replace(listener);
        Ok(Arc::new(Self(inner)))
    }

    pub fn session_snapshot(&self) -> Result<SchoolSessionSnapshot, SchoolBrowserError> {
        let state = self.0.state.guard();
        Ok(snapshot(&state))
    }

    pub fn user_browsing(&self, site: Option<SchoolSite>) -> Result<(), SchoolBrowserError> {
        let mut state = self.0.state.guard();
        state.browsing = site;
        if site.is_some() {
            state.extraction = None;
            state.hidden = None;
        }
        drop(state);
        let _ = self.0.traces.send(Trace::Browsing(site));
        self.0.browsing_changed.notify_waiters();
        Ok(())
    }

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

    pub async fn open(
        &self,
        site: SchoolSite,
        target: Option<String>,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let current = self
            .0
            .state
            .guard()
            .views
            .get(&site)
            .map(|view| view.url.clone());
        let already_open = current.is_some();
        let explicit_target = target.is_some();
        let target = target
            .or(current)
            .or_else(|| self.restored_target(site))
            .unwrap_or_else(|| site.start_url().to_string());
        let url = if explicit_target {
            site_url(site, &target)?
        } else {
            safe_url(&target)?
        };
        self.user_browsing(Some(site))?;
        let opened = (|| {
            let browser = self.ensure_browser(site, url.clone())?;
            // Revealing an existing resident view must preserve its exact native URL.
            // Re-navigating to our sanitized identity would strip SSO query state.
            if already_open
                && explicit_target
                && self.0.browser.current_url(browser)?.as_ref() != Some(&url)
            {
                self.dispatch(browser, BrowserAction::Navigate(url))?;
            }
            Ok::<_, SchoolBrowserError>(())
        })();
        if let Err(error) = opened {
            let _ = self.user_browsing(None);
            return Err(error);
        }
        Ok(SchoolWebResult {
            site,
            url: target,
            phase: "navigationDispatched",
        })
    }

    pub fn set_customization(
        &self,
        site: SchoolSite,
        customization: SchoolPageCustomization,
    ) -> Result<(), SchoolBrowserError> {
        if customization.css.len() > 256 * 1024 || customization.javascript.len() > 256 * 1024 {
            return Err(SchoolBrowserError::InvalidInput(
                "School customization exceeds the safe size.".into(),
            ));
        }
        self.0
            .state
            .guard()
            .customizations
            .insert(site, customization);
        Ok(())
    }

    pub fn configure_restore(
        &self,
        enabled: bool,
        path: std::path::PathBuf,
    ) -> Result<bool, SchoolBrowserError> {
        let _restore_io = self.0.restore_io.guard();
        if !enabled {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(SchoolBrowserError::Storage(error.to_string())),
            }
        }
        let current = {
            let mut state = self.0.state.guard();
            state.restore_epoch = state.restore_epoch.saturating_add(1);
            let epoch = state.restore_epoch;
            state.restore = enabled.then_some((path.clone(), epoch));
            state.browsing.and_then(|site| {
                state
                    .views
                    .get(&site)
                    .and_then(|view| sanitize_school_url(&view.url).map(|url| (site, url)))
            })
        };
        if enabled {
            if let Some((site, url)) = current {
                let bytes = serde_json::to_vec(&RestoreRecord { site, url })
                    .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
                crate::notes::atomic_write(&path, &bytes)
                    .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
            }
        }
        Ok(true)
    }

    fn restored_target(&self, site: SchoolSite) -> Option<String> {
        let path = self.0.state.guard().restore.clone()?.0;
        let bytes = std::fs::read(path).ok()?;
        let record: RestoreRecord = serde_json::from_slice(&bytes).ok()?;
        (record.site == site)
            .then(|| sanitize_school_url(&record.url))
            .flatten()
            .filter(|url| SchoolSite::for_url(url) == Some(site))
    }

    pub fn navigate(&self, site: SchoolSite, raw: &str) -> Result<bool, SchoolBrowserError> {
        let url = site_url(site, raw)?;
        let browser = self.ensure_browser(site, url.clone())?;
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        Ok(true)
    }

    pub fn reload(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Ok(browser) = self.browser_for(site) else {
            return Ok(false);
        };
        self.dispatch(browser, BrowserAction::Reload)?;
        Ok(true)
    }

    pub fn back(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Ok(browser) = self.browser_for(site) else {
            return Ok(false);
        };
        let target = {
            let mut state = self.0.state.guard();
            let current = state.views.get(&site).map(|view| view.url.clone());
            let target = state
                .history
                .entry(site)
                .or_default()
                .back_target(current.as_deref());
            if target.is_some() {
                state.suppress_history.insert(site);
            }
            target
        };
        let Some(target) = target else {
            return Ok(false);
        };
        self.dispatch(browser, BrowserAction::Navigate(site_url(site, &target)?))?;
        Ok(true)
    }

    pub fn forward(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Ok(browser) = self.browser_for(site) else {
            return Ok(false);
        };
        let target = self
            .0
            .state
            .guard()
            .history
            .entry(site)
            .or_default()
            .forward_target();
        let Some(target) = target else {
            return Ok(false);
        };
        self.dispatch(browser, BrowserAction::Navigate(site_url(site, &target)?))?;
        Ok(true)
    }

    pub fn set_guide(
        &self,
        items: Vec<crate::kyomu_guide::GuideItem>,
    ) -> Result<bool, SchoolBrowserError> {
        let items = crate::kyomu_guide::validate(items)
            .map_err(|error| SchoolBrowserError::InvalidInput(error.to_string()))?;
        self.0.state.guard().guide = items;
        Ok(true)
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
            let lane = self.acquire_lane().await?;
            let result = self.ensure_session_once(profile).await;
            drop(lane);
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
    async fn acquire_pull_lane(
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

    #[cfg(test)]
    pub(super) fn poison_state_for_test(&self) {
        let _state = self.0.state.guard();
        panic!("simulated panic while the school state was locked");
    }

    #[cfg(test)]
    pub(super) fn manual_waiters(&self) -> usize {
        self.0.manual_waiters.load(Ordering::Acquire)
    }

    async fn saved_credentials(
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

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        let _ = self.0.traces.send(Trace::Stopped);
        self.0.browsing_changed.notify_waiters();
        self.0.tasks.wait().await;
        let _ = self.0.browser.shutdown().await;
        self.0.listener.guard().take();
    }

    async fn login_locked(
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

    async fn login_attempt(
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

    fn clear_login(&self, run: &str) -> bool {
        let mut state = self.0.state.guard();
        if state.login.as_ref().is_some_and(|login| login.run == run) {
            state.login = None;
            return true;
        }
        false
    }

    fn fail_login(&self, profile: SchoolAuthProfile, run: &str, error: &SchoolBrowserError) {
        if !self.clear_login(run) {
            return;
        }
        let _ = self.0.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
            profile,
            state: "error".into(),
            message: error.to_string(),
        }));
        self.set_session(profile, SchoolSessionState::Failed);
    }

    async fn ensure_session_once(
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
            } else {
                self.fail_heartbeat(profile);
            }
        }
        result
    }

    fn clear_heartbeat(&self, profile: SchoolAuthProfile) -> bool {
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

    fn fail_heartbeat(&self, profile: SchoolAuthProfile) {
        if self.clear_heartbeat(profile) {
            self.set_session(profile, SchoolSessionState::Failed);
        }
    }

    /// Records that a hidden navigation landed on an identity-provider sign-in
    /// page. Returns `true` when saved credentials cannot recover it: the
    /// profile's automatic sign-in is blocked after a failure.
    fn sign_in_landing(&self, url: &Url, signing_in: &mut Option<SchoolAuthProfile>) -> bool {
        let Some(profile) = is_identity_login(url)
            .then(|| url.host_str().and_then(SchoolAuthProfile::for_auth_host))
            .flatten()
        else {
            return false;
        };
        *signing_in = Some(profile);
        self.0
            .state
            .with(|state| state.login.is_none() && state.auto_blocked.contains(&profile))
    }

    /// Subscribes before checking visibility, so a page opened after this
    /// check reaches the caller's wait as `Trace::Browsing`. Hidden work must
    /// never start while the person is looking at a school page.
    fn hidden_traces(&self) -> Result<broadcast::Receiver<Trace>, SchoolBrowserError> {
        let traces = self.0.traces.subscribe();
        if self.0.state.guard().browsing.is_some() {
            return Err(SchoolBrowserError::Preempted);
        }
        Ok(traces)
    }

    async fn acquire_lane(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, SchoolBrowserError> {
        tokio::select! {
            lane = self.0.lane.lock() => Ok(lane),
            _ = self.0.tasks.closed() => Err(SchoolBrowserError::Stopped),
        }
    }

    async fn wait_until_hidden(&self, context: &PullContext) -> Result<(), SchoolBrowserError> {
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

    async fn wait_until_hidden_without_context(&self) -> Result<(), SchoolBrowserError> {
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

    async fn pull_once(
        &self,
        site: SchoolSite,
        course_id: Option<u64>,
        context: &PullContext,
    ) -> Result<Vec<IngestOutcome>, SchoolBrowserError> {
        match site {
            SchoolSite::ScienceLms => {
                let outcome = self
                    .extract_page(
                        site,
                        "lms",
                        "https://lms.s.isct.ac.jp/2025/my/",
                        course_id,
                        course_id.is_none(),
                        OPEN_TIMEOUT,
                        context,
                    )
                    .await?;
                Ok(vec![outcome])
            }
            SchoolSite::Kyomu => {
                let landing = self
                    .navigate_and_wait(
                        site,
                        &format!("{KYOMU_ROOT}Default.aspx"),
                        Some(&format!("{KYOMU_ROOT}Student/")),
                        OPEN_TIMEOUT,
                        context,
                    )
                    .await?;
                if let HiddenLoad::SignInRequired = landing {
                    let outcome = sign_in_required(site, "session");
                    let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
                    return Ok(vec![outcome]);
                }
                let mut outcomes = Vec::new();
                for (page, kind) in KYOMU_PULL_PAGES.into_iter().zip(KYOMU_PULL_KINDS) {
                    let outcome = self
                        .extract_page(
                            site,
                            kind,
                            &format!("{KYOMU_ROOT}{page}"),
                            None,
                            false,
                            PAGE_TIMEOUT,
                            context,
                        )
                        .await;
                    let outcome = match outcome {
                        Ok(outcome) => outcome,
                        Err(
                            error @ (SchoolBrowserError::Cancelled
                            | SchoolBrowserError::Preempted
                            | SchoolBrowserError::Stopped),
                        ) => return Err(error),
                        Err(error) if outcomes.is_empty() => return Err(error),
                        Err(error) => {
                            outcomes.push(IngestOutcome {
                                site,
                                kind: kind.into(),
                                state: "error".into(),
                                changed: 0,
                                message: error.to_string(),
                            });
                            break;
                        }
                    };
                    let stop = outcome.state == "reconnect-required";
                    outcomes.push(outcome);
                    if stop {
                        break;
                    }
                }
                Ok(outcomes)
            }
            _ => Err(SchoolBrowserError::InvalidInput(
                "Only LMS and 教務Web can be pulled.".into(),
            )),
        }
    }

    async fn navigate_and_wait(
        &self,
        site: SchoolSite,
        raw: &str,
        prefix: Option<&str>,
        timeout: Duration,
        context: &PullContext,
    ) -> Result<HiddenLoad<PageLoaded>, SchoolBrowserError> {
        let url = safe_url(raw)?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, url.clone())?;
        let run = new_id();
        let baseline = {
            let mut state = self.0.state.guard();
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            state.hidden = Some(HiddenWork {
                browser,
                run: run.clone(),
                target: url.clone(),
                prefix: prefix.map(str::to_string),
            });
            state
                .views
                .get(&site)
                .map(|view| view.last_page)
                .unwrap_or_default()
        };
        let _hidden = HiddenGuard {
            inner: &self.0,
            run,
        };
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        let mut signing_in = None;
        tokio::time::timeout(timeout, async {
            loop {
                if context.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                let received = tokio::select! {
                    value = traces.recv() => value,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
                };
                match received {
                    Ok(Trace::Loaded(event))
                        if event.page.browser == browser
                            && event.page.sequence > baseline
                            && prefix
                                .is_none_or(|prefix| event.url.as_str().starts_with(prefix)) =>
                    {
                        return Ok(HiddenLoad::Ready(event))
                    }
                    Ok(Trace::Loaded(event)) if event.page.browser == browser => {
                        if self.sign_in_landing(&event.url, &mut signing_in) {
                            return Ok(HiddenLoad::SignInRequired);
                        }
                    }
                    Ok(Trace::Session { profile, state })
                        if signing_in == Some(profile)
                            && sign_in_settled_without_session(state) =>
                    {
                        return Ok(HiddenLoad::SignInRequired)
                    }
                    Ok(Trace::Failed(event)) if event.page.browser == browser => {
                        return Err(SchoolBrowserError::Platform(event.message))
                    }
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
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
            SchoolBrowserError::Timeout(
                "The school page did not answer in time. Check the account in Settings.".into(),
            )
        })?
    }

    #[allow(clippy::too_many_arguments)]
    async fn extract_page(
        &self,
        site: SchoolSite,
        kind: &str,
        url: &str,
        course_id: Option<u64>,
        full: bool,
        timeout: Duration,
        context: &PullContext,
    ) -> Result<IngestOutcome, SchoolBrowserError> {
        let parsed = safe_url(url)?;
        let mut traces = self.hidden_traces()?;
        let browser = self.ensure_browser(site, parsed.clone())?;
        let run = new_id();
        {
            let mut state = self.0.state.guard();
            // Checked under the same lock that arms the extraction: opening a
            // page clears it, so the visible view is never navigated for a pull.
            if state.browsing.is_some() {
                return Err(SchoolBrowserError::Preempted);
            }
            let baseline = state
                .views
                .get(&site)
                .map(|view| view.last_page)
                .unwrap_or_default();
            state.extraction = Some(Extraction {
                browser,
                run: run.clone(),
                course_id,
                full,
                baseline,
                target: parsed.to_string(),
                visible: false,
            });
            state.hidden = Some(HiddenWork {
                browser,
                run: run.clone(),
                target: parsed.clone(),
                prefix: None,
            });
        }
        let _hidden = HiddenGuard {
            inner: &self.0,
            run: run.clone(),
        };
        self.dispatch(browser, BrowserAction::Navigate(parsed))?;
        let mut signing_in = None;
        let raw = tokio::time::timeout(timeout, async {
            loop {
                if context.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                let received = tokio::select! {
                    value = traces.recv() => value,
                    _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
                };
                match received {
                    Ok(Trace::Extraction {
                        browser: found,
                        run: found_run,
                        raw,
                    }) if found == browser && found_run == run => {
                        return Ok(HiddenLoad::Ready(raw))
                    }
                    Ok(Trace::Loaded(event)) if event.page.browser == browser => {
                        if self.sign_in_landing(&event.url, &mut signing_in) {
                            return Ok(HiddenLoad::SignInRequired);
                        }
                    }
                    Ok(Trace::Session { profile, state })
                        if signing_in == Some(profile)
                            && sign_in_settled_without_session(state) =>
                    {
                        return Ok(HiddenLoad::SignInRequired)
                    }
                    Ok(Trace::Browsing(Some(_))) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Failed(event)) if event.page.browser == browser => {
                        return Err(SchoolBrowserError::Platform(event.message))
                    }
                    Ok(Trace::Closed(id)) if id == browser => return Err(view_closed()),
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
            SchoolBrowserError::Timeout(
                "The school page did not answer in time. Check the account in Settings.".into(),
            )
        })??;
        let raw = match raw {
            HiddenLoad::Ready(raw) => raw,
            HiddenLoad::SignInRequired => {
                let outcome = sign_in_required(site, kind);
                let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
                return Ok(outcome);
            }
        };
        if !context.applying().await {
            return Err(SchoolBrowserError::Cancelled);
        }
        let store = self.0.store.clone();
        let raw_for_apply = raw.clone();
        let applied =
            tokio::task::spawn_blocking(move || apply_raw(&store, site, &raw_for_apply)).await;
        // The write gate covers this page's commit only; the next 教務Web page
        // can be cancelled again.
        context.applied().await;
        let outcome = applied.map_err(|_| SchoolBrowserError::Stopped)??;
        let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
        Ok(outcome)
    }

    fn ensure_browser(
        &self,
        site: SchoolSite,
        initial: Url,
    ) -> Result<BrowserId, SchoolBrowserError> {
        let created = {
            let mut state = self.0.state.guard();
            if let Some(view) = state.views.get(&site) {
                return Ok(view.id);
            }
            let id = BrowserId {
                site,
                generation: self.0.next_generation.fetch_add(1, Ordering::Relaxed),
            };
            let nonce = new_id();
            state.views.insert(
                site,
                ViewState {
                    id,
                    last_page: 0,
                    url: initial.to_string(),
                    extraction_relay: format!("isct-companion-extract://{nonce}/"),
                    auth_relay: format!("isct-companion-auth:{nonce}:"),
                    progress_relay: format!("isct-companion-progress:{nonce}:"),
                    navigation: None,
                    actions: HashMap::new(),
                },
            );
            id
        };
        if let Err(error) = self.0.browser.ensure(created, initial) {
            {
                let mut state = self.0.state.guard();
                state.views.remove(&site);
            }
            return Err(error.into());
        }
        Ok(created)
    }

    fn browser_for(&self, site: SchoolSite) -> Result<BrowserId, SchoolBrowserError> {
        self.0
            .state
            .guard()
            .views
            .get(&site)
            .map(|view| view.id)
            .ok_or_else(|| SchoolBrowserError::InvalidInput("School view is not open.".into()))
    }

    fn dispatch(
        &self,
        browser: BrowserId,
        command: BrowserAction,
    ) -> Result<ActionId, SchoolBrowserError> {
        self.0.dispatch(browser, command)
    }

    fn set_session(&self, profile: SchoolAuthProfile, value: SchoolSessionState) {
        self.0.set_session_from_inner(profile, value);
    }
}

impl SchoolExecutor for SchoolBrowserEngine {
    fn execute(
        &self,
        request: SchoolExecution,
        context: PullContext,
    ) -> crate::school_service::SchoolExecutorFuture<'_> {
        let service = self.clone();
        Box::pin(async move {
            service
                .pull(
                    request.site,
                    request.course_id,
                    request.credentials,
                    context,
                )
                .await
                .map_err(map_executor)
        })
    }
}

fn map_executor(error: SchoolBrowserError) -> SchoolExecutorError {
    match error {
        SchoolBrowserError::InvalidInput(message) => SchoolExecutorError::InvalidInput(message),
        SchoolBrowserError::Storage(message) => SchoolExecutorError::Storage(message),
        SchoolBrowserError::Auth(message) => SchoolExecutorError::Authentication(message),
        SchoolBrowserError::SiteChanged(message) => SchoolExecutorError::SiteChanged(message),
        SchoolBrowserError::Timeout(message) => SchoolExecutorError::Timeout(message),
        SchoolBrowserError::Cancelled => SchoolExecutorError::Cancelled,
        SchoolBrowserError::Stopped => {
            SchoolExecutorError::Stopped("The school browser has stopped.".into())
        }
        // Native adapter failures have no narrower portable category.
        SchoolBrowserError::Platform(message) => SchoolExecutorError::Execution(message),
        SchoolBrowserError::Preempted => {
            SchoolExecutorError::Execution("School job paused for visible navigation.".into())
        }
    }
}

#[cfg(test)]
mod error_tests {
    use super::*;

    #[test]
    fn browser_errors_keep_actionable_executor_categories() {
        assert_eq!(
            map_executor(SchoolBrowserError::Timeout("portal timed out".into())),
            SchoolExecutorError::Timeout("portal timed out".into())
        );
        assert_eq!(
            map_executor(SchoolBrowserError::Auth("sign-in expired".into())),
            SchoolExecutorError::Authentication("sign-in expired".into())
        );
        assert_eq!(
            map_executor(SchoolBrowserError::Storage("database unavailable".into())),
            SchoolExecutorError::Storage("database unavailable".into())
        );
        assert_eq!(
            map_executor(SchoolBrowserError::SiteChanged("portal changed".into())),
            SchoolExecutorError::SiteChanged("portal changed".into())
        );
        assert_eq!(
            map_executor(SchoolBrowserError::Stopped),
            SchoolExecutorError::Stopped("The school browser has stopped.".into())
        );
        assert_eq!(
            map_executor(SchoolBrowserError::Cancelled),
            SchoolExecutorError::Cancelled
        );
    }
}

impl SchoolBrowserListener for Listener {
    fn page_loaded(&self, event: PageLoaded) {
        if let Some(inner) = self.0.upgrade() {
            let owner = inner.clone();
            let _entered = inner.runtime.enter();
            inner.tasks.spawn(async move {
                owner.page_loaded(event);
            });
        }
    }
    fn load_failed(&self, event: LoadFailed) {
        if let Some(inner) = self.0.upgrade() {
            inner.load_failed(event);
        }
    }
    fn title_changed(&self, page: PageId, title: String) {
        if let Some(inner) = self.0.upgrade() {
            inner.title_changed(page, &title);
        }
    }
    fn decide_navigation(&self, page: PageId, url: Url, main_frame: bool) -> NavigationDecision {
        self.0
            .upgrade()
            .map_or(NavigationDecision::Cancel, |inner| {
                inner.decide_navigation(page, url, main_frame)
            })
    }
    fn new_window(&self, page: PageId, url: Url) -> NewWindowDecision {
        self.0.upgrade().map_or(NewWindowDecision::Ignore, |inner| {
            inner.new_window(page, url)
        })
    }
    fn closed(&self, browser: BrowserId) {
        if let Some(inner) = self.0.upgrade() {
            inner.closed(browser);
        }
    }
}

impl Inner {
    fn current(&self, browser: BrowserId) -> bool {
        if self.tasks.is_closed() {
            return false;
        }
        self.state
            .with(|state| {
                state
                    .views
                    .get(&browser.site)
                    .map(|view| view.id == browser)
            })
            .unwrap_or(false)
    }

    fn page_loaded(self: &Arc<Self>, event: PageLoaded) {
        if !self.current(event.page.browser) {
            return;
        }
        let Some(safe_loaded) = sanitize_school_url(event.url.as_str()) else {
            return;
        };
        let (
            login,
            heartbeat,
            extraction,
            auth_relay,
            progress_relay,
            customization,
            guide,
            popup_return,
            restore,
        ) = {
            let mut state = self.state.guard();
            let Some(view) = state.views.get_mut(&event.page.browser.site) else {
                return;
            };
            if event
                .cause
                .is_some_and(|cause| view.navigation != Some(cause))
            {
                return;
            }
            if event.page.sequence <= view.last_page {
                return;
            }
            view.last_page = event.page.sequence;
            if let Some(cause) = event.cause {
                view.actions.remove(&cause);
            } else if let Some(action) = view.navigation {
                view.actions.remove(&action);
            }
            view.navigation = None;
            view.url = safe_loaded.clone();
            let relays = (view.auth_relay.clone(), view.progress_relay.clone());
            let suppressed = state.suppress_history.remove(&event.page.browser.site);
            if !suppressed && state.browsing == Some(event.page.browser.site) {
                if let Some(safe) = history_entry(event.page.browser.site, event.url.as_str()) {
                    state
                        .history
                        .entry(event.page.browser.site)
                        .or_default()
                        .push(safe);
                }
            }
            if state.extraction.is_none()
                && state.browsing == Some(event.page.browser.site)
                && ingestion_script(event.url.as_str()).is_some()
            {
                state.extraction = Some(Extraction {
                    browser: event.page.browser,
                    run: new_id(),
                    course_id: None,
                    full: false,
                    baseline: event.page.sequence.saturating_sub(1),
                    target: event.url.to_string(),
                    visible: true,
                });
            }
            let popup_return = match state.popup_return.get(&event.page.browser.site).cloned() {
                Some(popup) if same_page(&safe_loaded, &popup.opener) => {
                    state.popup_return.remove(&event.page.browser.site);
                    if let Some(history) = state.history.get_mut(&event.page.browser.site) {
                        while history
                            .pages
                            .last()
                            .is_some_and(|page| !same_page(page, &popup.opener))
                        {
                            history.pages.pop();
                        }
                        history.forward.clear();
                    }
                    None
                }
                Some(popup) if same_page(&safe_loaded, &popup.popup) => Some(popup.opener),
                Some(_) => {
                    state.popup_return.remove(&event.page.browser.site);
                    None
                }
                None => None,
            };
            (
                state.login.clone(),
                state.heartbeat,
                state.extraction.clone(),
                relays.0,
                relays.1,
                state
                    .customizations
                    .get(&event.page.browser.site)
                    .cloned()
                    .unwrap_or_default(),
                (SchoolSite::for_url(&safe_loaded) == Some(SchoolSite::Kyomu))
                    .then(|| crate::kyomu_guide::script(&state.guide))
                    .flatten(),
                popup_return,
                state.restore.clone(),
            )
        };
        let _ = self.events.emit(CoreEvent::SchoolWebUrl {
            site: event.page.browser.site,
            url: safe_loaded.clone(),
        });
        if let Some((path, epoch)) = restore {
            let record = RestoreRecord {
                site: event.page.browser.site,
                url: safe_loaded,
            };
            if let Ok(bytes) = serde_json::to_vec(&record) {
                let owner = self.clone();
                let tasks = self.tasks.clone();
                let _entered = self.runtime.enter();
                tasks.spawn(async move {
                    let _ = tokio::task::spawn_blocking(move || {
                        let _io = owner.restore_io.guard();
                        let current = owner.state.with(|state| state.restore.clone());
                        if current == Some((path.clone(), epoch)) {
                            let _ = crate::notes::atomic_write(&path, &bytes);
                        }
                    })
                    .await;
                });
            }
        }
        if !customization.css.trim().is_empty() {
            if let Ok(css) = serde_json::to_string(&customization.css) {
                let _ = self.dispatch_eval(event.page.browser, format!("(()=>{{const s=document.createElement('style');s.textContent={css};document.documentElement.appendChild(s)}})()"), false);
            }
        }
        if !customization.javascript.trim().is_empty() {
            let _ = self.dispatch_eval(event.page.browser, customization.javascript, false);
        }
        if let Some(guide) = guide {
            let _ = self.dispatch_eval(event.page.browser, guide, false);
        }
        if let Some(return_url) = popup_return {
            if let Ok(return_url) = serde_json::to_string(&return_url) {
                let _ = self.dispatch_eval(
                    event.page.browser,
                    format!("window.close=()=>location.replace({return_url});"),
                    false,
                );
            }
        }
        if let Some(active) = login {
            if login_succeeded(event.url.as_str(), active.profile) {
                {
                    let mut state = self.state.guard();
                    if state
                        .login
                        .as_ref()
                        .is_some_and(|login| login.run == active.run)
                    {
                        state.login = None;
                        state.auto_blocked.remove(&active.profile);
                    }
                }
                let _ = self.events.emit(CoreEvent::SchoolLogin(SchoolLoginEvent {
                    profile: active.profile,
                    state: "connected".into(),
                    message: "Connected".into(),
                }));
                self.set_session_from_inner(active.profile, SchoolSessionState::Active);
                // A hidden pull that was sent through sign-in continues at its target.
                let resume = self.state.with(|state| {
                    state
                        .hidden
                        .as_ref()
                        .filter(|work| {
                            work.browser == event.page.browser
                                && state.browsing.is_none()
                                && !work.reached(&event.url)
                        })
                        .map(|work| work.target.clone())
                });
                if let Some(target) = resume {
                    let _ = self.dispatch(event.page.browser, BrowserAction::Navigate(target));
                }
            } else if let Some(script) = login_script(event.url.as_str(), &active, &auth_relay) {
                let _ = self.dispatch_eval(event.page.browser, script, true);
            }
        } else if let Some(profile) =
            heartbeat.filter(|profile| login_succeeded(event.url.as_str(), *profile))
        {
            self.set_session_from_inner(profile, SchoolSessionState::Active);
        } else if is_identity_login(&event.url) {
            self.arm_saved_login(event.page, event.url.clone(), auth_relay);
        }
        if let Some(extraction) = extraction.filter(|run| {
            run.browser == event.page.browser
                && event.page.sequence > run.baseline
                && event.url.as_str() == run.target
        }) {
            if let Some(script) = ingestion_script(event.url.as_str()) {
                let course = extraction
                    .course_id
                    .map(|id| format!("globalThis.__ISCT_COMPANION_LMS_COURSE_ID__={id};"))
                    .unwrap_or_else(|| {
                        "delete globalThis.__ISCT_COMPANION_LMS_COURSE_ID__;".into()
                    });
                let quoted_run =
                    serde_json::to_string(&extraction.run).unwrap_or_else(|_| "\"\"".into());
                let setup = format!("{course}globalThis.__ISCT_COMPANION_LMS_FULL__={};globalThis.__ISCT_COMPANION_LMS_PROGRESS__=v=>{{if(v&&typeof v==='object')v._isctExtractionRun={quoted_run};document.title='{progress_relay}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,'')}};", extraction.full);
                let relay = self.state.with(|state| {
                    state
                        .views
                        .get(&event.page.browser.site)
                        .map(|view| view.extraction_relay.clone())
                });
                if let Some(relay) = relay {
                    let script = extraction_script(&setup, script, &relay, &extraction.run);
                    let _ = self.dispatch_eval(event.page.browser, script, true);
                }
            }
        }
        let _ = self.traces.send(Trace::Loaded(event));
    }

    fn load_failed(&self, event: LoadFailed) {
        let relevant = self.state.with(|state| {
            let Some(view) = state.views.get_mut(&event.page.browser.site) else {
                return false;
            };
            if view.id != event.page.browser {
                return false;
            }
            if let Some(cause) = event.cause {
                if view.actions.remove(&cause) != Some(true) {
                    return false;
                }
            } else if event.page.sequence <= view.last_page {
                return false;
            }
            view.last_page = view.last_page.max(event.page.sequence);
            view.navigation = None;
            true
        });
        if relevant {
            let heartbeat = self.state.with(|state| {
                state
                    .heartbeat
                    .filter(|profile| profile.start_site() == event.page.browser.site)
            });
            if let Some(profile) = heartbeat {
                self.set_session_from_inner(profile, SchoolSessionState::Failed);
            }
            let _ = self.traces.send(Trace::Failed(event));
        }
    }

    fn title_changed(&self, page: PageId, title: &str) {
        if !self.current(page.browser) {
            return;
        }
        let prefixes = self.state.with(|state| {
            state
                .views
                .get(&page.browser.site)
                .map(|view| (view.auth_relay.clone(), view.progress_relay.clone()))
        });
        let Some((auth, progress)) = prefixes else {
            return;
        };
        if let Some(encoded) = title.strip_prefix(&auth) {
            self.login_relay(encoded);
        } else if let Some(encoded) = title.strip_prefix(&progress) {
            self.progress_relay(page.browser, encoded);
        }
    }

    fn decide_navigation(&self, page: PageId, url: Url, _main_frame: bool) -> NavigationDecision {
        if !self.current(page.browser) {
            return NavigationDecision::Cancel;
        }
        let relay = self.state.with(|state| {
            state
                .views
                .get(&page.browser.site)
                .map(|view| view.extraction_relay.clone())
        });
        if let Some(encoded) = relay
            .as_deref()
            .and_then(|prefix| url.as_str().strip_prefix(prefix))
        {
            if encoded.len() <= 11 * 1024 * 1024 {
                if let Some(raw) = URL_SAFE_NO_PAD
                    .decode(encoded)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                {
                    let run = serde_json::from_str::<serde_json::Value>(&raw)
                        .ok()
                        .and_then(|value| {
                            value
                                .get("_isctExtractionRun")?
                                .as_str()
                                .map(str::to_string)
                        });
                    if let Some(run) = run {
                        let accepted = self.state.with(|state| {
                            if state.extraction.as_ref().is_some_and(|active| {
                                active.browser == page.browser && active.run == run
                            }) {
                                state.extraction.take()
                            } else {
                                None
                            }
                        });
                        if let Some(extraction) = accepted {
                            if extraction.visible {
                                let store = self.store.clone();
                                let events = self.events.clone();
                                let site = page.browser.site;
                                let _entered = self.runtime.enter();
                                self.tasks.spawn(async move {
                                    if let Ok(Ok(outcome)) =
                                        tokio::task::spawn_blocking(move || {
                                            apply_raw(&store, site, &raw)
                                        })
                                        .await
                                    {
                                        let _ = events.emit(CoreEvent::SchoolIngest(outcome));
                                    }
                                });
                            } else {
                                let _ = self.traces.send(Trace::Extraction {
                                    browser: page.browser,
                                    run,
                                    raw,
                                });
                            }
                        }
                    }
                }
            }
            return NavigationDecision::Cancel;
        }
        if sanitize_school_url(url.as_str()).is_some() {
            NavigationDecision::Allow
        } else {
            NavigationDecision::Cancel
        }
    }

    fn new_window(&self, page: PageId, url: Url) -> NewWindowDecision {
        if !self.current(page.browser) {
            return NewWindowDecision::Ignore;
        }
        let school_url = sanitize_school_url(url.as_str())
            .and_then(|safe| Url::parse(&safe).ok())
            .and_then(|safe| SchoolSite::for_url(safe.as_str()).map(|site| (site, safe)));
        match school_url {
            Some((site, safe)) if site == page.browser.site => {
                {
                    let mut state = self.state.guard();
                    if let Some(opener) = state.views.get(&site).map(|view| view.url.clone()) {
                        if !same_page(&opener, safe.as_str()) {
                            state.popup_return.insert(
                                site,
                                PopupReturn {
                                    opener,
                                    popup: safe.to_string(),
                                },
                            );
                        }
                    }
                }
                NewWindowDecision::SameView(safe)
            }
            Some((site, safe)) => {
                let _ = self.events.emit(CoreEvent::SchoolWebOpen {
                    site,
                    url: safe.to_string(),
                });
                NewWindowDecision::OpenSchool { site, url: safe }
            }
            None if matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none() =>
            {
                NewWindowDecision::OpenExternal(url)
            }
            None => NewWindowDecision::Ignore,
        }
    }

    /// Forgets a view generation the shell closed or could not create, so the
    /// next request creates a new view instead of dispatching into nothing.
    fn closed(&self, browser: BrowserId) {
        {
            let mut state = self.state.guard();
            if state
                .views
                .get(&browser.site)
                .is_some_and(|view| view.id == browser)
            {
                state.views.remove(&browser.site);
            }
        }
        let _ = self.traces.send(Trace::Closed(browser));
    }

    fn sent(
        &self,
        browser: BrowserId,
        result: Result<(), BrowserError>,
    ) -> Result<(), BrowserError> {
        if let Err(BrowserError::Unavailable(_)) = &result {
            self.closed(browser);
        }
        result
    }

    fn dispatch(
        &self,
        browser: BrowserId,
        command: BrowserAction,
    ) -> Result<ActionId, SchoolBrowserError> {
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        if matches!(command, BrowserAction::Navigate(_)) {
            let mut state = self.state.guard();
            if let Some(view) = state.views.get_mut(&browser.site) {
                if view.id == browser {
                    view.navigation = Some(action);
                    if view.actions.len() >= 64 {
                        view.actions.clear();
                    }
                    view.actions.insert(action, true);
                }
            }
        } else if matches!(command, BrowserAction::Reload) {
            let mut state = self.state.guard();
            if let Some(view) = state.views.get_mut(&browser.site) {
                if view.id == browser {
                    view.navigation = Some(action);
                    view.actions.insert(action, true);
                }
            }
        }
        self.sent(
            browser,
            self.browser.dispatch(BrowserCommand {
                browser,
                action,
                command,
            }),
        )?;
        Ok(action)
    }

    fn dispatch_eval(
        &self,
        browser: BrowserId,
        script: String,
        critical: bool,
    ) -> Result<(), BrowserError> {
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        {
            let mut state = self.state.guard();
            if let Some(view) = state.views.get_mut(&browser.site) {
                if view.id == browser {
                    if view.actions.len() >= 64 {
                        view.actions.retain(|_, critical| *critical);
                    }
                    view.actions.insert(action, critical);
                }
            }
        }
        self.sent(
            browser,
            self.browser.dispatch(BrowserCommand {
                browser,
                action,
                command: BrowserAction::Eval(script),
            }),
        )
    }

    fn arm_saved_login(self: &Arc<Self>, page: PageId, url: Url, relay: String) {
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
                    && state.views.get(&page.browser.site).is_some_and(|view| {
                        view.id == page.browser
                            && view.last_page == page.sequence
                            && view.url == expected_url
                    })
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
                    && state.views.get(&page.browser.site).is_some_and(|view| {
                        view.id == page.browser
                            && view.last_page == page.sequence
                            && view.url == expected_url
                    })
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

    fn login_relay(&self, encoded: &str) {
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
        let mut failed = false;
        let matched = self.state.with(|state| {
            let matched = state
                .login
                .as_ref()
                .is_some_and(|active| active.profile == relay.profile && active.run == relay.run);
            if matched && relay.state != "progress" && relay.state != "needs-user" {
                state.login = None;
                state.auto_blocked.insert(relay.profile);
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
            self.set_session_from_inner(relay.profile, SchoolSessionState::Failed);
        }
    }

    fn progress_relay(&self, browser: BrowserId, encoded: &str) {
        if encoded.len() > 4 * 1024 {
            return;
        }
        let Some(progress) = URL_SAFE_NO_PAD
            .decode(encoded)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ProgressRelay>(&bytes).ok())
        else {
            return;
        };
        let active = self.state.with(|state| {
            state
                .extraction
                .as_ref()
                .is_some_and(|run| run.browser == browser && run.run == progress.extraction_run)
        });
        if active
            && matches!(
                progress.phase.as_str(),
                "catalog" | "course" | "material" | "assignment"
            )
            && progress.completed <= progress.total
            && progress.total <= 10_000
        {
            let _ = self
                .events
                .emit(CoreEvent::SchoolPullProgress(SchoolPullProgress {
                    site: SchoolSite::ScienceLms,
                    phase: progress.phase,
                    completed: progress.completed,
                    total: progress.total,
                    course_id: progress.course_id,
                    course_title: truncate(progress.course_title, 512),
                    term: truncate(progress.term, 16),
                    item_title: truncate(progress.item_title, 512),
                }));
        }
    }

    fn set_session_from_inner(&self, profile: SchoolAuthProfile, value: SchoolSessionState) {
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
            let _ = self.refresh.event(RefreshEvent {
                kind: "account-active".into(),
                profile: Some(profile),
                course_id: None,
                network: None,
            });
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressRelay {
    #[serde(default, rename = "_isctExtractionRun")]
    extraction_run: String,
    phase: String,
    completed: usize,
    total: usize,
    #[serde(default)]
    course_id: u64,
    #[serde(default)]
    course_title: String,
    #[serde(default)]
    term: String,
    #[serde(default)]
    item_title: String,
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

fn login_script(raw: &str, login: &ActiveLogin, relay: &str) -> Option<String> {
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

fn extraction_script(setup: &str, script: &str, relay: &str, run: &str) -> String {
    let run = serde_json::to_string(run).unwrap_or_else(|_| "\"\"".into());
    format!("{setup}Promise.resolve({script}).then(v=>{{if(v&&typeof v==='object')v._isctExtractionRun={run};return v}}).then(v=>location.href='{relay}'+btoa(unescape(encodeURIComponent(JSON.stringify(v)))).replace(/\\+/g,'-').replace(/\\//g,'_').replace(/=+$/,''))")
}

fn ingestion_script(raw: &str) -> Option<&'static str> {
    let url = Url::parse(raw).ok()?;
    if url.host_str() == Some("lms.s.isct.ac.jp")
        && url.path().starts_with("/2025/")
        && !url.path().contains("/pluginfile.php/")
    {
        return Some(LMS_EXTRACT_SCRIPT);
    }
    (url.host_str() == Some("kyomu0.gakumu.titech.ac.jp")
        && KYOMU_PULL_PAGES
            .iter()
            .any(|page| url.path() == format!("/Titech/{page}")))
    .then_some(KYOMU_EXTRACT_SCRIPT)
}

fn apply_raw(
    store: &DataStore,
    site: SchoolSite,
    raw: &str,
) -> Result<IngestOutcome, SchoolBrowserError> {
    let mut connection = rusqlite::Connection::open(store.path())
        .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
    let applied = if site == SchoolSite::ScienceLms {
        match parse_lms_payload(raw) {
            Ok((LmsStage::Ready, Some(payload))) => apply_lms_payload(&mut connection, &payload)
                .map(|changed| ("lms".to_string(), changed))
                .map_err(|error| ("lms".to_string(), error.to_string())),
            Ok(_) => Err(("lms".into(), "reconnect-required".into())),
            Err(error) => Err((
                "lms".into(),
                serde_json::from_str::<serde_json::Value>(raw)
                    .ok()
                    .and_then(|value| value.get("error")?.as_str().map(str::to_string))
                    .map(|reason| truncate(reason, 1_024))
                    .map(|reason| format!("{error}: {reason}"))
                    .unwrap_or_else(|| error.to_string()),
            )),
        }
    } else {
        match parse_kyomu_payload(raw) {
            Ok((KyomuStage::Ready, Some(payload))) => {
                let kind = payload.kind.clone();
                apply_kyomu_payload(&mut connection, &payload)
                    .map(|changed| (kind.clone(), changed))
                    .map_err(|error| (kind, error.to_string()))
            }
            Ok((_, payload)) => Err((
                payload
                    .map(|value| value.kind)
                    .unwrap_or_else(|| "session".into()),
                "reconnect-required".into(),
            )),
            Err(error) => Err(("unknown".into(), error.to_string())),
        }
    };
    Ok(match applied {
        Ok((kind, changed)) => IngestOutcome {
            site,
            kind,
            state: "ok".into(),
            changed,
            message: format!("{changed} rows changed"),
        },
        Err((kind, message)) if message == "reconnect-required" => sign_in_required(site, &kind),
        Err((kind, message)) => IngestOutcome {
            site,
            kind,
            state: "error".into(),
            changed: 0,
            message,
        },
    })
}

/// A per-view close is a platform failure for the work in that view, not a
/// shutdown of the whole school browser.
fn view_closed() -> SchoolBrowserError {
    SchoolBrowserError::Platform("The school view was closed.".into())
}

fn sign_in_required(site: SchoolSite, kind: &str) -> IngestOutcome {
    IngestOutcome {
        site,
        kind: kind.into(),
        state: "reconnect-required".into(),
        changed: 0,
        message: "Sign-in required".into(),
    }
}

/// Automatic sign-in belongs to the session check, the visible page or the
/// hidden pull that led this view to the identity provider.
fn owns_sign_in(state: &State, profile: SchoolAuthProfile, browser: BrowserId) -> bool {
    state.heartbeat == Some(profile)
        || state.browsing == Some(browser.site)
        || state
            .hidden
            .as_ref()
            .is_some_and(|work| work.browser == browser)
}

fn sign_in_settled_without_session(state: SchoolSessionState) -> bool {
    matches!(
        state,
        SchoolSessionState::Failed | SchoolSessionState::NotConfigured
    )
}

fn snapshot(state: &State) -> SchoolSessionSnapshot {
    let get = |profile| {
        state
            .sessions
            .get(&profile)
            .copied()
            .unwrap_or(SchoolSessionState::Pending)
    };
    SchoolSessionSnapshot {
        revision: state.revision,
        states: SchoolSessionStates {
            science: get(SchoolAuthProfile::Science),
            tokyo_tech: get(SchoolAuthProfile::TokyoTech),
        },
    }
}
fn safe_url(raw: &str) -> Result<Url, SchoolBrowserError> {
    let safe = sanitize_school_url(raw).ok_or_else(|| {
        SchoolBrowserError::InvalidInput(
            "School URL is outside the allowed official origins.".into(),
        )
    })?;
    Url::parse(&safe).map_err(|_| SchoolBrowserError::InvalidInput("School URL is invalid.".into()))
}
fn site_url(site: SchoolSite, raw: &str) -> Result<Url, SchoolBrowserError> {
    let url = safe_url(raw)?;
    if SchoolSite::for_url(url.as_str()) != Some(site) {
        return Err(SchoolBrowserError::InvalidInput(
            "School URL belongs to a different service.".into(),
        ));
    }
    Ok(url)
}
fn history_entry(site: SchoolSite, raw: &str) -> Option<String> {
    let safe = sanitize_school_url(raw)?;
    if SchoolSite::for_url(&safe) != Some(site) {
        return None;
    }
    let url = Url::parse(&safe).ok()?;
    match url.host_str().and_then(SchoolAuthProfile::for_auth_host) {
        Some(profile) if !login_succeeded(&safe, profile) => None,
        _ => Some(safe),
    }
}
fn same_page(left: &str, right: &str) -> bool {
    let (Ok(left), Ok(right)) = (Url::parse(left), Url::parse(right)) else {
        return false;
    };
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
        && left.path() == right.path()
}
fn pull_profile(site: SchoolSite) -> SchoolAuthProfile {
    if site == SchoolSite::ScienceLms {
        SchoolAuthProfile::Science
    } else {
        SchoolAuthProfile::TokyoTech
    }
}
fn new_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}
fn login_succeeded(raw: &str, profile: SchoolAuthProfile) -> bool {
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
fn valid_login_relay(state: &str, message: &str) -> bool {
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
fn truncate(value: String, max: usize) -> String {
    value.chars().take(max).collect()
}
fn clean_message(message: &str) -> String {
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
