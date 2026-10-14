use super::sanitize_school_url;
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
        upcoming_otps, OtpWindow, SchoolAuthProfile, SchoolAuthService, SchoolCredentials,
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
        Arc, Mutex, Weak,
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
    Browsing,
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
        inner
            .listener
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .replace(listener);
        Ok(Arc::new(Self(inner)))
    }

    pub fn session_snapshot(&self) -> Result<SchoolSessionSnapshot, SchoolBrowserError> {
        let state = self
            .0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?;
        Ok(snapshot(&state))
    }

    pub fn user_browsing(&self, site: Option<SchoolSite>) -> Result<(), SchoolBrowserError> {
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?;
        state.browsing = site;
        if site.is_some() {
            state.extraction = None;
        }
        drop(state);
        let _ = self.0.traces.send(Trace::Browsing);
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
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
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
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .customizations
            .insert(site, customization);
        Ok(())
    }

    pub fn configure_restore(
        &self,
        enabled: bool,
        path: std::path::PathBuf,
    ) -> Result<bool, SchoolBrowserError> {
        let _restore_io = self
            .0
            .restore_io
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?;
        if !enabled {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(SchoolBrowserError::Storage(error.to_string())),
            }
        }
        let current = {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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
        let path = self.0.state.lock().ok()?.restore.clone()?.0;
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
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
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
        self.0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .guide = items;
        Ok(true)
    }

    pub async fn login(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let _lane = self.acquire_lane().await?;
        self.login_locked(profile, credentials).await
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
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
            if !state.pending_profiles.insert(profile) {
                return Ok(false);
            }
        }
        let engine = self.clone();
        let _entered = self.0.runtime.enter();
        if !self.0.tasks.spawn(async move {
            let _ = engine.ensure_session(profile).await;
            if let Ok(mut state) = engine.0.state.lock() {
                state.pending_profiles.remove(&profile);
            }
        }) {
            if let Ok(mut state) = self.0.state.lock() {
                state.pending_profiles.remove(&profile);
            }
            return Err(SchoolBrowserError::Stopped);
        }
        Ok(true)
    }

    pub async fn launch_sessions(&self) -> Result<SchoolSessionSnapshot, SchoolBrowserError> {
        let epoch = self
            .0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .lifecycle_epoch;
        let _run = self.0.lifecycle.lock().await;
        if self
            .0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .lifecycle_epoch
            != epoch
        {
            return self.session_snapshot();
        }
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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
        if let Ok(mut state) = self.0.state.lock() {
            state.lifecycle_epoch = state.lifecycle_epoch.saturating_add(1);
        }
        self.session_snapshot()
    }

    pub async fn focus_sessions(
        &self,
    ) -> Result<Option<SchoolSessionSnapshot>, SchoolBrowserError> {
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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

    pub async fn pull(
        &self,
        site: SchoolSite,
        course_id: Option<u64>,
        context: PullContext,
        priority: PullPriority,
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
        loop {
            self.wait_until_hidden(&context).await?;
            let manual = priority == PullPriority::Manual;
            let _manual_waiter =
                manual.then(|| ManualWaiter::new(&self.0.manual_waiters, &self.0.priority_changed));
            let lane = loop {
                if context.is_cancelled() {
                    return Err(SchoolBrowserError::Cancelled);
                }
                if self.0.tasks.is_closed() {
                    return Err(SchoolBrowserError::Stopped);
                }
                if manual || self.0.manual_waiters.load(Ordering::Acquire) == 0 {
                    if let Ok(lane) = self.0.lane.try_lock() {
                        break lane;
                    }
                }
                tokio::select! {
                    _ = self.0.priority_changed.notified() => {}
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                    _ = self.0.tasks.closed() => return Err(SchoolBrowserError::Stopped),
                }
            };
            if context.is_cancelled() {
                return Err(SchoolBrowserError::Cancelled);
            }
            let result = self.pull_once(site, course_id, &context).await;
            drop(lane);
            match result {
                Err(SchoolBrowserError::Preempted) if !context.is_cancelled() => continue,
                result => return result,
            }
        }
    }

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        let _ = self.0.traces.send(Trace::Stopped);
        self.0.browsing_changed.notify_waiters();
        self.0.tasks.wait().await;
        let _ = self.0.browser.shutdown().await;
        if let Ok(mut listener) = self.0.listener.lock() {
            listener.take();
        }
    }

    async fn login_locked(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        self.wait_until_hidden_without_context().await?;
        let site = profile.start_site();
        let url = safe_url(site.start_url())?;
        let browser = self.ensure_browser(site, url.clone())?;
        let run = new_id();
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
            state.auto_blocked.remove(&profile);
            state.login = Some(ActiveLogin {
                profile,
                credentials,
                run: run.clone(),
            });
        }
        self.set_session(profile, SchoolSessionState::Pending);
        let mut traces = self.0.traces.subscribe();
        if let Err(error) = self.dispatch(browser, BrowserAction::Navigate(url)) {
            self.fail_login(profile, &run, &error);
            return Err(error);
        }
        let state = tokio::time::timeout(SESSION_TIMEOUT, async {
            loop {
                match traces.recv().await {
                    Ok(Trace::Session {
                        profile: found,
                        state,
                    }) if found == profile => return Ok(state),
                    Ok(Trace::Browsing) => return Err(SchoolBrowserError::Preempted),
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
                    self.fail_login(profile, &run, &error);
                } else {
                    self.clear_login(&run);
                }
                return Err(error);
            }
            Err(error) => {
                self.fail_login(profile, &run, &error);
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
        if let Ok(mut state) = self.0.state.lock() {
            if state.login.as_ref().is_some_and(|login| login.run == run) {
                state.login = None;
                return true;
            }
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
        let browser = self.ensure_browser(site, url.clone())?;
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
            state.heartbeat = Some(profile);
        }
        self.set_session(profile, SchoolSessionState::Pending);
        let mut traces = self.0.traces.subscribe();
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
                    }) if found == profile => return Ok(state == SchoolSessionState::Active),
                    Ok(Trace::Browsing) => return Err(SchoolBrowserError::Preempted),
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
        if let Ok(mut state) = self.0.state.lock() {
            if state.heartbeat == Some(profile) {
                state.heartbeat = None;
                if state
                    .login
                    .as_ref()
                    .is_some_and(|login| login.profile == profile)
                {
                    state.login = None;
                }
                return true;
            }
        }
        false
    }

    fn fail_heartbeat(&self, profile: SchoolAuthProfile) {
        if self.clear_heartbeat(profile) {
            self.set_session(profile, SchoolSessionState::Failed);
        }
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
            let visible = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?
                .browsing
                .is_some();
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
            let visible = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?
                .browsing
                .is_some();
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
                self.navigate_and_wait(
                    site,
                    &format!("{KYOMU_ROOT}Default.aspx"),
                    Some(&format!("{KYOMU_ROOT}Student/")),
                    OPEN_TIMEOUT,
                    context,
                )
                .await?;
                let mut outcomes = Vec::new();
                for (page, kind) in KYOMU_PULL_PAGES.into_iter().zip(KYOMU_PULL_KINDS) {
                    let outcome = self
                        .extract_page(
                            site,
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
    ) -> Result<PageLoaded, SchoolBrowserError> {
        let url = safe_url(raw)?;
        let browser = self.ensure_browser(site, url.clone())?;
        let baseline = self
            .0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .views
            .get(&site)
            .map(|view| view.last_page)
            .unwrap_or_default();
        let mut traces = self.0.traces.subscribe();
        self.dispatch(browser, BrowserAction::Navigate(url))?;
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
                        return Ok(event)
                    }
                    Ok(Trace::Failed(event)) if event.page.browser == browser => {
                        return Err(SchoolBrowserError::Platform(event.message))
                    }
                    Ok(Trace::Browsing) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Closed(id)) if id == browser => {
                        return Err(SchoolBrowserError::Stopped)
                    }
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

    async fn extract_page(
        &self,
        site: SchoolSite,
        url: &str,
        course_id: Option<u64>,
        full: bool,
        timeout: Duration,
        context: &PullContext,
    ) -> Result<IngestOutcome, SchoolBrowserError> {
        let parsed = safe_url(url)?;
        let browser = self.ensure_browser(site, parsed.clone())?;
        let run = new_id();
        {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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
        }
        let mut traces = self.0.traces.subscribe();
        self.dispatch(browser, BrowserAction::Navigate(parsed))?;
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
                    }) if found == browser && found_run == run => return Ok(raw),
                    Ok(Trace::Browsing) => return Err(SchoolBrowserError::Preempted),
                    Ok(Trace::Failed(event)) if event.page.browser == browser => {
                        return Err(SchoolBrowserError::Platform(event.message))
                    }
                    Ok(Trace::Closed(id)) if id == browser => {
                        return Err(SchoolBrowserError::Stopped)
                    }
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
        if !context.applying().await {
            return Err(SchoolBrowserError::Cancelled);
        }
        let store = self.0.store.clone();
        let raw_for_apply = raw.clone();
        let outcome = tokio::task::spawn_blocking(move || apply_raw(&store, site, &raw_for_apply))
            .await
            .map_err(|_| SchoolBrowserError::Stopped)??;
        let _ = self.0.events.emit(CoreEvent::SchoolIngest(outcome.clone()));
        Ok(outcome)
    }

    fn ensure_browser(
        &self,
        site: SchoolSite,
        initial: Url,
    ) -> Result<BrowserId, SchoolBrowserError> {
        let created = {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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
            if let Ok(mut state) = self.0.state.lock() {
                state.views.remove(&site);
            }
            return Err(error.into());
        }
        Ok(created)
    }

    fn browser_for(&self, site: SchoolSite) -> Result<BrowserId, SchoolBrowserError> {
        self.0
            .state
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
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
        let action = ActionId(self.0.next_action.fetch_add(1, Ordering::Relaxed));
        if matches!(command, BrowserAction::Navigate(_)) {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
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
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| SchoolBrowserError::Stopped)?;
            if let Some(view) = state.views.get_mut(&browser.site) {
                if view.id == browser {
                    view.navigation = Some(action);
                    view.actions.insert(action, true);
                }
            }
        }
        self.0.browser.dispatch(BrowserCommand {
            browser,
            action,
            command,
        })?;
        Ok(action)
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
            let priority = context.priority();
            if let Some((profile, credentials)) = request.credentials {
                service
                    .login(profile, credentials)
                    .await
                    .map_err(map_executor)?;
            }
            service
                .pull(request.site, request.course_id, context, priority)
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
            .lock()
            .ok()
            .and_then(|state| {
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
            let Ok(mut state) = self.state.lock() else {
                return;
            };
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
                        let Ok(_io) = owner.restore_io.lock() else {
                            return;
                        };
                        let current = owner
                            .state
                            .lock()
                            .ok()
                            .and_then(|state| state.restore.clone());
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
                if let Ok(mut state) = self.state.lock() {
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
            } else if let Some(script) = login_script(event.url.as_str(), &active, &auth_relay) {
                let _ = self.dispatch_eval(event.page.browser, script, true);
            }
        } else if let Some(profile) =
            heartbeat.filter(|profile| login_succeeded(event.url.as_str(), *profile))
        {
            self.set_session_from_inner(profile, SchoolSessionState::Active);
        } else if is_login_url(&event.url) {
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
                let relay = self.state.lock().ok().and_then(|state| {
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
        let relevant = self.state.lock().ok().is_some_and(|mut state| {
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
            view.last_page = event.page.sequence;
            view.navigation = None;
            true
        });
        if relevant {
            let heartbeat = self.state.lock().ok().and_then(|state| {
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
        let prefixes = self.state.lock().ok().and_then(|state| {
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
        let relay = self.state.lock().ok().and_then(|state| {
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
                        let accepted = self.state.lock().ok().and_then(|mut state| {
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
                if let Ok(mut state) = self.state.lock() {
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

    fn closed(&self, browser: BrowserId) {
        if let Ok(mut state) = self.state.lock() {
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

    fn dispatch_eval(
        &self,
        browser: BrowserId,
        script: String,
        critical: bool,
    ) -> Result<(), BrowserError> {
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        if let Ok(mut state) = self.state.lock() {
            if let Some(view) = state.views.get_mut(&browser.site) {
                if view.id == browser {
                    if view.actions.len() >= 64 {
                        view.actions.retain(|_, critical| *critical);
                    }
                    view.actions.insert(action, critical);
                }
            }
        }
        self.browser.dispatch(BrowserCommand {
            browser,
            action,
            command: BrowserAction::Eval(script),
        })
    }

    fn arm_saved_login(self: &Arc<Self>, page: PageId, url: Url, relay: String) {
        let Some(profile) = url.host_str().and_then(SchoolAuthProfile::for_auth_host) else {
            return;
        };
        let Some(expected_url) = sanitize_school_url(url.as_str()) else {
            return;
        };
        let (allowed, blocked_heartbeat) = self.state.lock().ok().map_or((false, false), |state| {
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
            let owned = inner.state.lock().ok().is_some_and(|state| {
                (state.heartbeat == Some(profile) || state.browsing == Some(page.browser.site))
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
            let accepted = inner.state.lock().ok().is_some_and(|mut state| {
                if state.login.is_none()
                    && (state.heartbeat == Some(profile)
                        || state.browsing == Some(page.browser.site))
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
        let matched = self.state.lock().ok().is_some_and(|mut state| {
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
        let active = self.state.lock().ok().is_some_and(|state| {
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
        let event = self.state.lock().ok().map(|mut state| {
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
        if let Some(event) = event {
            if value == SchoolSessionState::Active {
                let _ = self.refresh.event(RefreshEvent {
                    kind: crate::school_refresh::RefreshKind::AccountActive,
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
        Err((kind, message)) if message == "reconnect-required" => IngestOutcome {
            site,
            kind,
            state: "reconnect-required".into(),
            changed: 0,
            message: "Sign-in required".into(),
        },
        Err((kind, message)) => IngestOutcome {
            site,
            kind,
            state: "error".into(),
            changed: 0,
            message,
        },
    })
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
fn new_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}
fn is_login_url(url: &Url) -> bool {
    url.host_str()
        .and_then(SchoolAuthProfile::for_auth_host)
        .is_some()
        && is_login_step(url)
}
fn is_login_step(url: &Url) -> bool {
    let path = url.path().trim_end_matches('/');
    match url.host_str() {
        Some("isct.ex-tic.com") => matches!(
            path,
            "/auth" | "/auth/session" | "/auth/session/second_factor"
        ),
        Some("portal.nap.gsic.titech.ac.jp" | "portal.titech.ac.jp") => {
            matches!(path, "" | "/portal.pl" | "/GetAccess/Login")
        }
        _ => false,
    }
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
