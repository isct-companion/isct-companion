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
    kyomu_ingest::{apply_kyomu_payload, record_kyomu_unavailable, KYOMU_EXTRACT_SCRIPT},
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
    school_service::{
        IngestOutcome, IngestState, SchoolExecution, SchoolExecutor, SchoolExecutorError,
    },
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

mod extraction;
mod history;
mod lane;
mod login;
mod restore;

use self::{extraction::*, history::*, login::*};

const SESSION_TIMEOUT: Duration = Duration::from_secs(90);
const LAUNCH_INTERVAL: Duration = Duration::from_secs(90);
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
    #[cfg(feature = "desktop-workspaces")]
    submission: Option<super::submission::PendingSubmission>,
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
    user_url: Option<String>,
    background_navigation: bool,
    extraction_relay: String,
    auth_relay: String,
    progress_relay: String,
    navigation: Option<ActionId>,
    actions: HashMap<ActionId, bool>,
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

    pub async fn open(
        &self,
        site: SchoolSite,
        target: Option<String>,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let (already_open, current, background_changed) = {
            let state = self.0.state.guard();
            let view = state.views.get(&site);
            (
                view.is_some(),
                view.and_then(|v| {
                    v.user_url
                        .clone()
                        .or_else(|| (!v.background_navigation).then(|| v.url.clone()))
                }),
                view.is_some_and(|v| v.background_navigation),
            )
        };
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
                && (explicit_target || background_changed)
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

    pub fn set_guide(
        &self,
        items: Vec<crate::kyomu_guide::GuideItem>,
    ) -> Result<bool, SchoolBrowserError> {
        let items = crate::kyomu_guide::validate(items)
            .map_err(|error| SchoolBrowserError::InvalidInput(error.to_string()))?;
        self.0.state.guard().guide = items;
        Ok(true)
    }

    #[cfg(feature = "desktop-workspaces")]
    pub fn prepare_submission(
        &self,
        raw: &str,
        files: Vec<crate::workspaces::SubmissionFile>,
        messages: super::SubmissionMessages,
    ) -> Result<String, super::SubmissionError> {
        let pending = super::submission::PendingSubmission::new(raw, files, messages)?;
        let url = pending.url.clone();
        let browser = {
            let mut state = self.0.state.guard();
            let browser = state
                .views
                .get(&SchoolSite::ScienceLms)
                .filter(|view| view.navigation.is_none() && pending.matches(&view.url))
                .map(|view| view.id);
            state.submission = Some(pending);
            browser
        };
        if let Some(browser) = browser {
            self.0.inject_submission(browser, &url)?;
        }
        Ok(url)
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

    pub async fn shutdown(&self) {
        #[cfg(feature = "desktop-workspaces")]
        {
            self.0.state.guard().submission = None;
        }
        self.0.tasks.close();
        let _ = self.0.traces.send(Trace::Stopped);
        self.0.browsing_changed.notify_waiters();
        self.0.tasks.wait().await;
        let _ = self.0.browser.shutdown().await;
        self.0.listener.guard().take();
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
                    user_url: None,
                    background_navigation: false,
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

pub(crate) fn map_executor(error: SchoolBrowserError) -> SchoolExecutorError {
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
            foreground,
        ) = {
            let mut state = self.state.guard();
            let foreground = state.browsing == Some(event.page.browser.site);
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
            if foreground {
                view.user_url = Some(safe_loaded.clone());
                view.background_navigation = false;
            }
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
                    catalog_page: None,
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
                foreground.then(|| state.restore.clone()).flatten(),
                foreground,
            )
        };
        if foreground {
            let _ = self.events.emit(CoreEvent::SchoolWebUrl {
                site: event.page.browser.site,
                url: safe_loaded.clone(),
            });
        }
        if let Some((path, epoch)) = restore {
            self.write_restore(path, epoch, event.page.browser.site, safe_loaded);
        }
        if !customization.css.trim().is_empty() {
            if let Ok(css) = serde_json::to_string(&customization.css) {
                let _ = self.dispatch_eval(event.page.browser, format!("(()=>{{const s=document.createElement('style');s.textContent={css};document.documentElement.appendChild(s)}})()"), false);
            }
        }
        if !customization.javascript.trim().is_empty() {
            let _ = self.dispatch_eval(event.page.browser, customization.javascript, false);
        }
        #[cfg(feature = "desktop-workspaces")]
        let _ = self.inject_submission(event.page.browser, event.url.as_str());
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
                let catalog_page = serde_json::to_string(&extraction.catalog_page)
                    .unwrap_or_else(|_| "null".into());
                let course =
                    format!("{course}globalThis.__ISCT_COMPANION_KYOMU_PAGE__={catalog_page};");
                let quoted_run =
                    serde_json::to_string(&extraction.run).unwrap_or_else(|_| "\"\"".into());
                let course = format!(
                    "{course}globalThis.__ISCT_COMPANION_KYOMU_HIDDEN__={};",
                    !extraction.visible
                );
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

    #[cfg(feature = "desktop-workspaces")]
    fn inject_submission(
        &self,
        browser: BrowserId,
        url: &str,
    ) -> Result<(), super::SubmissionError> {
        if browser.site != SchoolSite::ScienceLms {
            return Ok(());
        }
        let pending = {
            let mut state = self.state.guard();
            if state
                .submission
                .as_ref()
                .is_some_and(|pending| pending.expired())
            {
                state.submission = None;
            }
            if state
                .submission
                .as_ref()
                .is_some_and(|pending| pending.matches(url))
            {
                state.submission.take()
            } else {
                None
            }
        };
        if let Some(pending) = pending {
            self.dispatch_eval(browser, pending.script()?, false)
                .map_err(|error| super::SubmissionError::Browser(error.to_string()))?;
        }
        Ok(())
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
}

/// A per-view close is a platform failure for the work in that view, not a
/// shutdown of the whole school browser.
fn view_closed() -> SchoolBrowserError {
    SchoolBrowserError::Platform("The school view was closed.".into())
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
fn new_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}
fn truncate(value: String, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
#[path = "../tests/school_browser__engine__error_tests.rs"]
mod error_tests;
