use super::policy::{
    automation_target, browsing_url, is_identity_login, is_login_step, navigation_rule,
    new_window_rule, restore_record, DenyReason, Verdict, ViewRole,
};
use super::{
    ActionId, BecameDownload, BrowserAction, BrowserCommand, BrowserError, BrowserId,
    DownloadDisposition, History, HostCapabilities, LoadFailed, LoadFailure, NavigationDecision,
    NavigationRequest, NewWindowDecision, NewWindowRequest, PageId, PageLoaded, SchoolAutoSignIn,
    SchoolBrowser, SchoolBrowserError, SchoolBrowserListener, SchoolConnectResult,
    SchoolLoginEvent, SchoolPageCustomization, SchoolProfileSession, SchoolSessionEvent,
    SchoolSessionLifecycle, SchoolSessionLifecycleKind, SchoolSessionProfiles,
    SchoolSessionSnapshot, SchoolSessionState, SchoolSignInProblem, SchoolSignInProblemKind,
    SchoolWebEntry, SchoolWebFailure, SchoolWebNotice, SchoolWebPage, SchoolWebResult,
    SchoolWebStep,
};
use crate::{
    data::WorkspaceChangeSource,
    events::CoreEvent,
    lifecycle::SchoolSite,
    ports::EventSink,
    runtime_tasks::RuntimeTasks,
    school_auth::{
        upcoming_otps, OtpWindow, SchoolAuthProfile, SchoolAuthService, SchoolCredentials,
    },
    school_service::{IngestOutcome, IngestState},
    sync_engine::{Hint, SyncHints},
};
use adpt_isct::scripts::SCHOOL_LOGIN;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError, Weak,
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex as AsyncMutex, Notify};
use url::Url;

mod jobs;
mod lane;
mod login;
mod otp_enroll;
mod otp_setup;
mod pull;
mod restore;

use self::{jobs::*, lane::*, login::*, pull::*};

const SESSION_TIMEOUT: Duration = Duration::from_secs(90);
const LAUNCH_INTERVAL: Duration = Duration::from_secs(90);
const FOCUS_INTERVAL: Duration = Duration::from_secs(120);
/// Popups one view may hold at once, nested ones included.
const MAX_POPUPS: usize = 8;

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
    auth: Arc<SchoolAuthService>,
    events: Arc<dyn EventSink>,
    /// Sign-ins reach the sync planner here.
    hints: SyncHints,
    browser: Arc<dyn SchoolBrowser>,
    capabilities: HostCapabilities,
    state: Mutex<State>,
    lane: AsyncMutex<()>,
    traces: broadcast::Sender<Trace>,
    browsing_changed: Notify,
    tasks: RuntimeTasks,
    runtime: tokio::runtime::Handle,
    next_generation: AtomicU64,
    next_action: AtomicU64,
    next_job: AtomicU64,
    listener: Mutex<Option<Arc<dyn SchoolBrowserListener>>>,
    restore_io: Mutex<()>,
    transport: std::sync::OnceLock<Transport>,
}

#[derive(Default)]
struct State {
    otp_setup: Option<u64>,
    otp_enrollment: Option<otp_enroll::Enrollment>,
    views: HashMap<SchoolSite, ViewState>,
    browsing: Option<SchoolSite>,
    login: Option<ActiveLogin>,
    /// The last failed sign-in per profile and what automatic sign-in does
    /// about it; cleared by a successful sign-in or a credential change.
    sign_in: HashMap<SchoolAuthProfile, SignInRecord>,
    sessions: HashMap<SchoolAuthProfile, SchoolSessionState>,
    kyomu_maintenance: Option<String>,
    revision: u64,
    last_focus: Option<Instant>,
    last_launch: Option<Instant>,
    heartbeat: Option<Heartbeat>,
    /// The one session job each profile may have; every caller joins it.
    jobs: HashMap<SchoolAuthProfile, SessionJob>,
    /// Bumped when a new attempt takes a profile over (a session check, a
    /// sign-in, new credentials). A result that finishes after its attempt
    /// was superseded must not publish the profile's state.
    attempts: HashMap<SchoolAuthProfile, u64>,
    session_retry: HashMap<SchoolAuthProfile, tokio::time::Instant>,
    customizations: HashMap<SchoolSite, SchoolPageCustomization>,
    guide: Vec<crate::kyomu_guide::GuideItem>,
    submission: Option<super::submission::PendingSubmission>,
    restore: Option<(std::path::PathBuf, u64)>,
    restore_epoch: u64,
    /// The page last published for each site (`school-web-page`).
    pages: HashMap<SchoolSite, SchoolWebPage>,
    page_revision: u64,
}

struct ViewState {
    id: BrowserId,
    last_page: u64,
    url: String,
    user_url: Option<String>,
    background_navigation: bool,
    auth_relay: String,
    navigation: Option<ActionId>,
    actions: HashMap<ActionId, bool>,
    /// The stop sent to this view for an abandoned attempt, until the view
    /// confirms it. Hidden work does not lease the view meanwhile.
    stopping: Option<ActionId>,
    /// Windows its pages opened, in opening order: a popup's parent is the
    /// view or a popup before it. They live and close with the view.
    popups: Vec<Popup>,
    /// `None` until the host reports it; some hosts never do.
    history: Option<History>,
    failure: Option<SchoolWebFailure>,
    /// The surface that last brought the view on screen (`open`).
    presenter: Option<String>,
}

struct Popup {
    id: BrowserId,
    parent: BrowserId,
    last_page: u64,
    url: Option<String>,
    history: Option<History>,
    failure: Option<SchoolWebFailure>,
}

impl State {
    /// Who the view or popup `browser` serves; `None` once it is gone.
    fn role_of(&self, browser: BrowserId) -> Option<ViewRole> {
        let view = self.views.get(&browser.site)?;
        (view.id == browser || view.popups.iter().any(|popup| popup.id == browser)).then(|| {
            if self.browsing == Some(browser.site) {
                ViewRole::User
            } else {
                ViewRole::Automation
            }
        })
    }

    /// The view or popup the app's toolbar acts on for `site`.
    fn front(&self, site: SchoolSite, inline_popups: bool) -> Option<BrowserId> {
        let view = self.views.get(&site)?;
        Some(
            view.popups
                .last()
                .filter(|_| inline_popups)
                .map_or(view.id, |popup| popup.id),
        )
    }

    /// The page the person sees of `site`, with the revision left for the
    /// caller to stamp.
    fn page(&self, site: SchoolSite, inline_popups: bool) -> SchoolWebPage {
        let mut page = SchoolWebPage {
            site,
            revision: 0,
            url: None,
            popups: 0,
            back: SchoolWebStep::No,
            forward: SchoolWebStep::No,
            notice: None,
        };
        let Some(view) = self.views.get(&site) else {
            return page;
        };
        let step = |known: Option<bool>| match known {
            Some(true) => SchoolWebStep::Yes,
            Some(false) => SchoolWebStep::No,
            None => SchoolWebStep::Unknown,
        };
        match view.popups.last().filter(|_| inline_popups) {
            Some(popup) => {
                page.url = popup.url.clone();
                page.popups = view.popups.len();
                // Back closes a popup at the start of its history.
                page.back = step(popup.history.map(|_| true));
                page.forward = step(popup.history.map(|history| history.forward));
                page.notice = popup.failure.clone();
            }
            None => {
                page.url = view.user_url.clone().or_else(|| {
                    (view.last_page > 0 && !view.background_navigation).then(|| view.url.clone())
                });
                page.back = step(view.history.map(|history| history.back));
                page.forward = step(view.history.map(|history| history.forward));
                page.notice = view.failure.clone();
            }
        }
        page
    }

    /// The page of `site` under the revision it was last stamped with, or a
    /// new revision when it changed since; true when it changed.
    fn stamp_page(&mut self, site: SchoolSite, inline_popups: bool) -> (SchoolWebPage, bool) {
        let mut page = self.page(site, inline_popups);
        if let Some(last) = self.pages.get(&site) {
            page.revision = last.revision;
            if *last == page {
                return (page, false);
            }
        }
        self.page_revision += 1;
        page.revision = self.page_revision;
        self.pages.insert(site, page.clone());
        (page, true)
    }

    /// Removes `popup` and every popup opened from it, returning their ids.
    fn remove_popup(&mut self, popup: BrowserId) -> Vec<BrowserId> {
        let Some(view) = self.views.get_mut(&popup.site) else {
            return Vec::new();
        };
        let mut removed = Vec::new();
        if view.popups.iter().any(|open| open.id == popup) {
            removed.push(popup);
        }
        // Opening order puts every descendant after its parent.
        view.popups.retain(|open| {
            let gone = open.id == popup || removed.contains(&open.parent);
            if gone && open.id != popup {
                removed.push(open.id);
            }
            !gone
        });
        removed
    }
}

#[derive(Clone, Debug)]
enum Trace {
    Enrollment,
    Session {
        profile: SchoolAuthProfile,
        state: SchoolSessionState,
    },
    /// The visible site changed. `Some` preempts hidden work.
    Browsing(Option<SchoolSite>),
    /// A school page finished loading in any view (hidden or visible).
    Loaded(PageLoaded),
    Closed(BrowserId),
    /// The view confirmed the stop sent for an abandoned attempt.
    Halted(BrowserId),
    Stopped,
}

struct Listener(Weak<Inner>);

impl SchoolBrowserEngine {
    pub fn new(
        auth: Arc<SchoolAuthService>,
        events: Arc<dyn EventSink>,
        browser: Arc<dyn SchoolBrowser>,
        hints: SyncHints,
    ) -> Result<Arc<Self>, SchoolBrowserError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            SchoolBrowserError::Platform("School browser requires an async runtime.".into())
        })?;
        let (traces, _) = broadcast::channel(128);
        let inner = Arc::new(Inner {
            auth,
            events,
            hints,
            capabilities: browser.capabilities(),
            browser: browser.clone(),
            state: Mutex::new(State::default()),
            lane: AsyncMutex::new(()),
            traces,
            browsing_changed: Notify::new(),
            tasks: RuntimeTasks::default(),
            runtime,
            next_generation: AtomicU64::new(1),
            next_action: AtomicU64::new(1),
            next_job: AtomicU64::new(1),
            listener: Mutex::new(None),
            restore_io: Mutex::new(()),
            transport: std::sync::OnceLock::new(),
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
        if state.browsing == site {
            return Ok(());
        }
        if state.browsing == Some(SchoolSite::SciencePortal)
            && site != Some(SchoolSite::SciencePortal)
        {
            state.otp_setup = None;
            state.otp_enrollment = None;
        }
        // Transfer ownership before notifying the hidden waiter. Its later
        // cleanup must not erase a new visible login, or leave a stale run
        // blocking the newly opened page's saved-credential read.
        state.heartbeat = None;
        state.login = None;
        state.browsing = site;
        let login_page = site
            .and_then(|site| state.views.get(&site))
            .and_then(|view| {
                let url = Url::parse(&view.url).ok()?;
                is_identity_login(&url).then(|| {
                    (
                        PageId {
                            browser: view.id,
                            sequence: view.last_page,
                        },
                        url,
                        view.auth_relay.clone(),
                    )
                })
            });
        drop(state);
        let _ = self.0.traces.send(Trace::Browsing(site));
        self.0.browsing_changed.notify_waiters();
        // Changes made while the site was off screen were not published.
        if let Some(site) = site {
            self.0.publish_page(site);
        }
        // Revealing a resident login page does not produce another load.
        if let Some((page, url, relay)) = login_page {
            self.0.arm_saved_login(page, url, relay);
        }
        Ok(())
    }

    /// Brings `site`'s view on screen for `surface`, an opaque id of the app
    /// page showing it, which becomes the site's presenter.
    pub async fn open(
        &self,
        site: SchoolSite,
        surface: &str,
        entry: SchoolWebEntry,
    ) -> Result<SchoolWebResult, SchoolBrowserError> {
        let (already_open, current, background_changed, presenter) = {
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
                view.and_then(|v| v.presenter.clone()),
            )
        };
        // A surface coming back finds the view as it left it, unless another
        // surface showed it meanwhile.
        let target = match entry {
            SchoolWebEntry::Resume => None,
            SchoolWebEntry::Return { .. } if presenter.as_deref() == Some(surface) => None,
            SchoolWebEntry::Go { url } | SchoolWebEntry::Return { url } => Some(url),
        };
        let explicit_target = target.is_some();
        let target = target
            .or(current)
            .or_else(|| self.restored_target(site))
            .unwrap_or_else(|| site.start_url().to_string());
        let url = interactive_url(&target)?;
        self.user_browsing(Some(site))?;
        let opened = (|| {
            let browser = self.ensure_browser(site, url.clone())?;
            self.0.state.with(|state| {
                if let Some(view) = state.views.get_mut(&site) {
                    view.presenter = Some(surface.to_owned());
                }
            });
            // An app link lands in the view itself, never under a popup.
            if already_open && explicit_target {
                self.0.close_popups(site);
            }
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
        self.0.publish_page(site);
        Ok(SchoolWebResult {
            site,
            url: self.page(site).url.unwrap_or(target),
            phase: "navigationDispatched",
        })
    }

    /// The page the person sees of `site` now. A surface subscribes to
    /// `school-web-page` first and then reads this, keeping the higher
    /// revision.
    pub fn page(&self, site: SchoolSite) -> SchoolWebPage {
        self.0
            .state
            .with(|state| state.stamp_page(site, self.0.capabilities.inline_popups).0)
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

    /// Loads an address the person typed in the frontmost view or popup.
    pub fn navigate(&self, site: SchoolSite, raw: &str) -> Result<bool, SchoolBrowserError> {
        let url = interactive_url(raw)?;
        self.user_browsing(Some(site))?;
        self.ensure_browser(site, url.clone())?;
        let browser = self.front(site)?;
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        Ok(true)
    }

    /// The app's Reload. A host that reports form resubmission reloads
    /// natively, and the rule table turns a resubmission into a GET; for one
    /// that cannot, Core loads the current URL with GET itself.
    pub fn reload(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Ok(browser) = self.front(site) else {
            return Ok(false);
        };
        let command = if self.0.capabilities.form_resubmission {
            BrowserAction::Reload
        } else {
            match self.0.browser.current_url(browser)? {
                Some(url) => BrowserAction::Navigate(url),
                None => return Ok(false),
            }
        };
        self.dispatch(browser, command)?;
        Ok(true)
    }

    /// The app's Back, through the engine's own history. A popup at the
    /// start of its history closes instead, as a browser closes a tab a page
    /// opened, and the page under it is in front again.
    pub fn back(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Some((browser, popup, history)) = self.front_history(site) else {
            return Ok(false);
        };
        match history {
            Some(history) if !history.back && popup => {
                self.0.popup_closed(browser, true);
                Ok(true)
            }
            Some(history) if !history.back => Ok(false),
            _ => {
                self.dispatch(browser, BrowserAction::GoBack)?;
                Ok(true)
            }
        }
    }

    /// The app's Forward, the counterpart of Back.
    pub fn forward(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let Some((browser, _, history)) = self.front_history(site) else {
            return Ok(false);
        };
        if history.is_some_and(|history| !history.forward) {
            return Ok(false);
        }
        self.dispatch(browser, BrowserAction::GoForward)?;
        Ok(true)
    }

    /// The view or popup in front of `site`, whether it is a popup, and its
    /// history when the host reports it.
    fn front_history(&self, site: SchoolSite) -> Option<(BrowserId, bool, Option<History>)> {
        self.0.state.with(|state| {
            let view = state.views.get(&site)?;
            Some(
                match view
                    .popups
                    .last()
                    .filter(|_| self.0.capabilities.inline_popups)
                {
                    Some(popup) => (popup.id, true, popup.history),
                    None => (view.id, false, view.history),
                },
            )
        })
    }

    /// Closes the frontmost popup over `site`'s view, and any it opened.
    pub fn close_popup(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let popup = self.0.state.with(|state| {
            state
                .views
                .get(&site)
                .and_then(|view| view.popups.last().map(|popup| popup.id))
        });
        let Some(popup) = popup else {
            return Ok(false);
        };
        self.0.popup_closed(popup, true);
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

    pub fn prepare_submission(
        &self,
        raw: &str,
        files: Vec<super::SubmissionFile>,
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

    pub async fn shutdown(&self) {
        self.0.state.guard().submission = None;
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
                    auth_relay: format!("isct-companion-auth:{nonce}:"),
                    navigation: None,
                    actions: HashMap::new(),
                    stopping: None,
                    popups: Vec::new(),
                    history: None,
                    failure: None,
                    presenter: None,
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

    fn front(&self, site: SchoolSite) -> Result<BrowserId, SchoolBrowserError> {
        self.0
            .state
            .guard()
            .front(site, self.0.capabilities.inline_popups)
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
    fn history_changed(&self, browser: BrowserId, history: History) {
        if let Some(inner) = self.0.upgrade() {
            inner.history_changed(browser, history);
        }
    }
    fn decide_navigation(&self, request: NavigationRequest) -> NavigationDecision {
        self.0
            .upgrade()
            .map_or(NavigationDecision::Deny(DenyReason::Closed), |inner| {
                inner.decide_navigation(request)
            })
    }
    fn new_window(&self, request: NewWindowRequest) -> NewWindowDecision {
        self.0
            .upgrade()
            .map_or(NewWindowDecision::Deny(DenyReason::Closed), |inner| {
                inner.new_window(request)
            })
    }
    fn became_download(&self, event: BecameDownload) {
        if let Some(inner) = self.0.upgrade() {
            inner.became_download(event);
        }
    }
    fn closed(&self, browser: BrowserId) {
        if let Some(inner) = self.0.upgrade() {
            inner.closed(browser);
        }
    }
    fn stopped(&self, page: PageId, action: ActionId) {
        if let Some(inner) = self.0.upgrade() {
            inner.stopped(page, action);
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
            self.popup_loaded(event);
            return;
        }
        self.page_available(event);
    }

    /// A popup's page only moves the address the person sees; Core runs no
    /// script, restore or sign-in in it.
    fn popup_loaded(&self, event: PageLoaded) {
        if self.tasks.is_closed() {
            return;
        }
        let browser = event.page.browser;
        let Some(url) = browsing_url(event.url.as_str()) else {
            return;
        };
        let moved = self.state.with(|state| {
            let popup = state
                .views
                .get_mut(&browser.site)?
                .popups
                .iter_mut()
                .find(|popup| popup.id == browser)?;
            if event.page.sequence <= popup.last_page {
                return None;
            }
            popup.last_page = event.page.sequence;
            popup.url = Some(url);
            popup.failure = None;
            Some(())
        });
        if moved.is_some() {
            self.publish_page(browser.site);
        }
    }

    fn page_available(self: &Arc<Self>, event: PageLoaded) {
        if !self.current(event.page.browser) {
            return;
        }
        let official_loaded = restore_record(event.url.as_str());
        let foreground = self
            .state
            .with(|state| state.browsing == Some(event.page.browser.site));
        let Some(safe_loaded) = (if foreground {
            browsing_url(event.url.as_str())
        } else {
            official_loaded.clone()
        }) else {
            return;
        };
        let (login, heartbeat, auth_relay, customization, guide, restore, foreground) = {
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
            view.failure = None;
            if foreground {
                view.user_url = Some(safe_loaded.clone());
                view.background_navigation = false;
            }
            let auth_relay = view.auth_relay.clone();
            (
                state.login.clone(),
                state.heartbeat,
                auth_relay,
                state
                    .customizations
                    .get(&event.page.browser.site)
                    .cloned()
                    .unwrap_or_default(),
                (SchoolSite::for_url(&safe_loaded) == Some(SchoolSite::Kyomu))
                    .then(|| crate::kyomu_guide::script(&state.guide))
                    .flatten(),
                foreground.then(|| state.restore.clone()).flatten(),
                foreground,
            )
        };
        // The opener behind an inline popup does not move the page on screen.
        if foreground {
            self.publish_page(event.page.browser.site);
        }
        if let (Some((path, epoch)), Some(official)) = (restore, official_loaded.as_ref()) {
            self.write_restore(path, epoch, event.page.browser.site, official.clone());
        }
        let _ = self.traces.send(Trace::Loaded(event.clone()));
        // External browsing gets history/address updates, never school scripts,
        // saved credentials, customizations or automatic ingestion.
        if official_loaded.is_none() {
            return;
        }
        if !customization.css.trim().is_empty() {
            if let Ok(css) = serde_json::to_string(&customization.css) {
                let _ = self.dispatch_eval(event.page.browser, format!("(()=>{{const s=document.createElement('style');s.textContent={css};document.documentElement.appendChild(s)}})()"), false);
            }
        }
        if !customization.javascript.trim().is_empty() {
            let _ = self.dispatch_eval(event.page.browser, customization.javascript, false);
        }
        let _ = self.inject_submission(event.page.browser, event.url.as_str());
        if let Some(guide) = guide {
            let _ = self.dispatch_eval(event.page.browser, guide, false);
        }
        // Enrollment starts with a visible manual sign-in. In particular it
        // must never select App OTP using a missing or obsolete saved secret.
        if self.state.guard().otp_setup.is_some()
            && event.page.browser.site == SchoolSite::SciencePortal
        {
            if foreground && login_succeeded(event.url.as_str(), SchoolAuthProfile::Science) {
                if let Ok(url) = Url::parse(crate::school_auth::otp_setup::SETTINGS_URL) {
                    let _ = self.dispatch(event.page.browser, BrowserAction::Navigate(url));
                }
            }
            self.otp_enrollment_page(&event);
            return;
        }
        // A sign-in and a session check belong to the view they run in; a
        // page from any other view neither finishes nor drives them.
        let login = login.filter(|active| active.browser == event.page.browser);
        let heartbeat = heartbeat.filter(|heartbeat| heartbeat.browser == event.page.browser);
        if let Some(active) = login {
            if login_succeeded(event.url.as_str(), active.profile) {
                self.finish_login(active);
            } else if let Some(script) = login_script(event.url.as_str(), &active, &auth_relay) {
                let _ = self.dispatch_eval(event.page.browser, script, true);
            }
        } else if let Some(heartbeat) =
            heartbeat.filter(|heartbeat| login_succeeded(event.url.as_str(), heartbeat.profile))
        {
            self.publish_session(
                heartbeat.profile,
                SchoolSessionState::Active,
                Some(heartbeat.attempt),
            );
        } else if is_identity_login(&event.url) {
            self.arm_saved_login(event.page, event.url.clone(), auth_relay);
        }
    }

    fn report_kyomu_maintenance(&self, message: &str) {
        self.state
            .with(|state| state.kyomu_maintenance = Some(message.into()));
        let _ = self.events.emit(CoreEvent::SchoolIngest(IngestOutcome {
            site: SchoolSite::Kyomu,
            kind: "service".into(),
            state: IngestState::Maintenance,
            changed: 0,
            message: message.into(),
        }));
    }

    fn load_failed(&self, event: LoadFailed) {
        // A load the engine gave up for another one, or one the rule table
        // cancelled, is not a failure of the page.
        if event.reason == LoadFailure::Cancelled {
            return;
        }
        let browser = event.page.browser;
        log::info!(
            target: "app_core::school_browser",
            "load failed site={:?} reason={:?} host={}: {}",
            browser.site,
            event.reason,
            event.url.as_ref().and_then(Url::host_str).unwrap_or("-"),
            event.message
        );
        let failure = SchoolWebFailure {
            url: event
                .url
                .as_ref()
                .and_then(|url| browsing_url(url.as_str())),
            reason: event.reason,
        };
        let relevant = self.state.with(|state| {
            // Hidden work fails on its own terms; nothing is shown for it.
            let shown = state.role_of(browser) == Some(ViewRole::User);
            let Some(view) = state.views.get_mut(&browser.site) else {
                return false;
            };
            if view.id != browser {
                if let Some(popup) = view.popups.iter_mut().find(|popup| popup.id == browser) {
                    if event.page.sequence > popup.last_page {
                        popup.last_page = event.page.sequence;
                        popup.failure = shown.then_some(failure);
                    }
                }
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
            view.failure = shown.then_some(failure);
            true
        });
        self.publish_page(browser.site);
        if relevant {
            let heartbeat = self.state.with(|state| {
                state
                    .heartbeat
                    .filter(|heartbeat| heartbeat.browser == browser)
            });
            if let Some(heartbeat) = heartbeat {
                self.publish_session(
                    heartbeat.profile,
                    SchoolSessionState::Failed,
                    Some(heartbeat.attempt),
                );
            }
        }
    }

    fn notice(&self, site: SchoolSite, notice: SchoolWebNotice) {
        let _ = self
            .events
            .emit(CoreEvent::SchoolWebNotice { site, notice });
    }

    /// Records a load of `page` the rule table refused as its view's or
    /// popup's failure, while the person is looking at it.
    fn refused(&self, page: PageId, failure: SchoolWebFailure) {
        let browser = page.browser;
        let recorded = self.state.with(|state| {
            if state.role_of(browser) != Some(ViewRole::User) {
                return None;
            }
            let view = state.views.get_mut(&browser.site)?;
            if view.id == browser {
                view.failure = Some(failure);
            } else {
                view.popups
                    .iter_mut()
                    .find(|popup| popup.id == browser)?
                    .failure = Some(failure);
            }
            Some(())
        });
        if recorded.is_some() {
            self.publish_page(browser.site);
        }
    }

    /// Whether `browser` is what the person is looking at for its site.
    fn shown(&self, browser: BrowserId) -> bool {
        self.state.with(|state| {
            state.role_of(browser) == Some(ViewRole::User)
                && state.front(browser.site, self.capabilities.inline_popups) == Some(browser)
        })
    }

    fn title_changed(self: &Arc<Self>, page: PageId, title: &str) {
        if !self.current(page.browser) {
            return;
        }
        let auth = self.state.with(|state| {
            state
                .views
                .get(&page.browser.site)
                .map(|view| view.auth_relay.clone())
        });
        if let Some(encoded) = auth.and_then(|auth| title.strip_prefix(&auth).map(str::to_string)) {
            if let Some(encoded) = encoded.strip_prefix("otp:") {
                self.otp_enrollment_relay(page, encoded);
                return;
            }
            self.login_relay(page.browser, &encoded);
        }
    }

    fn role_of(&self, browser: BrowserId) -> Option<ViewRole> {
        if self.tasks.is_closed() {
            return None;
        }
        self.state.with(|state| state.role_of(browser))
    }

    fn decide_navigation(&self, request: NavigationRequest) -> NavigationDecision {
        let browser = request.page.browser;
        let Some(role) = self.role_of(browser) else {
            denied(browser.site, None, DenyReason::Closed, &request.url);
            return NavigationDecision::Deny(DenyReason::Closed);
        };
        match navigation_rule(role, &request) {
            Verdict::Allow | Verdict::Popup => NavigationDecision::Allow,
            Verdict::External => {
                log::info!(target: "app_core::school_browser", "to system browser site={:?} host={}", browser.site, request.url.host_str().unwrap_or("-"));
                NavigationDecision::External(request.url)
            }
            Verdict::Deny(reason) => {
                denied(browser.site, Some(role), reason, &request.url);
                if reason == DenyReason::Cleartext && request.main_frame {
                    self.refused(
                        request.page,
                        SchoolWebFailure {
                            url: browsing_url(request.url.as_str()),
                            reason: LoadFailure::Cleartext,
                        },
                    );
                }
                NavigationDecision::Deny(reason)
            }
            Verdict::SkipEntry => {
                denied(
                    browser.site,
                    Some(role),
                    DenyReason::Resubmission,
                    &request.url,
                );
                NavigationDecision::SkipEntry
            }
            Verdict::ReloadAsGet => {
                denied(
                    browser.site,
                    Some(role),
                    DenyReason::Resubmission,
                    &request.url,
                );
                let mut url = request.url.clone();
                url.set_fragment(None);
                let shown = self.shown(browser);
                if self
                    .dispatch(browser, BrowserAction::Navigate(url.clone()))
                    .is_ok()
                    && shown
                {
                    if let Some(url) = browsing_url(url.as_str()) {
                        self.notice(browser.site, SchoolWebNotice::Reopened { url });
                    }
                }
                NavigationDecision::Deny(DenyReason::Resubmission)
            }
        }
    }

    fn new_window(&self, request: NewWindowRequest) -> NewWindowDecision {
        let opener = request.opener.browser;
        let Some(role) = self.role_of(opener) else {
            denied(opener.site, None, DenyReason::Closed, &request.url);
            return NewWindowDecision::Deny(DenyReason::Closed);
        };
        match new_window_rule(role, &request) {
            Verdict::Popup => {
                let id = BrowserId {
                    site: opener.site,
                    generation: self.next_generation.fetch_add(1, Ordering::Relaxed),
                };
                let open = self.state.with(|state| {
                    let view = state.views.get_mut(&opener.site)?;
                    if view.popups.len() >= MAX_POPUPS {
                        return None;
                    }
                    view.popups.push(Popup {
                        id,
                        parent: opener,
                        last_page: 0,
                        url: None,
                        history: None,
                        failure: None,
                    });
                    Some(())
                });
                let Some(()) = open else {
                    denied(
                        opener.site,
                        Some(role),
                        DenyReason::TooManyPopups,
                        &request.url,
                    );
                    return NewWindowDecision::Deny(DenyReason::TooManyPopups);
                };
                self.publish_page(opener.site);
                NewWindowDecision::Popup(id)
            }
            Verdict::External => {
                log::info!(target: "app_core::school_browser", "window to system browser site={:?} host={}", opener.site, request.url.host_str().unwrap_or("-"));
                NewWindowDecision::External(request.url)
            }
            Verdict::Deny(reason) => {
                denied(opener.site, Some(role), reason, &request.url);
                NewWindowDecision::Deny(reason)
            }
            Verdict::Allow | Verdict::SkipEntry | Verdict::ReloadAsGet => {
                denied(opener.site, Some(role), DenyReason::Scheme, &request.url);
                NewWindowDecision::Deny(DenyReason::Scheme)
            }
        }
    }

    /// A popup opened only for a file that is saved has nothing left to
    /// show, so it closes, as a browser closes an empty download tab. A file
    /// shown in place, or a popup that already had a page, stays.
    fn became_download(&self, event: BecameDownload) {
        let popup = event.page.browser;
        let is_popup = self.state.with(|state| {
            state
                .views
                .get(&popup.site)
                .is_some_and(|view| view.popups.iter().any(|open| open.id == popup))
        });
        if is_popup && !event.had_page && event.disposition == DownloadDisposition::Saved {
            log::info!(target: "app_core::school_browser", "closing a popup that became a download site={:?}", popup.site);
            self.popup_closed(popup, true);
        }
    }

    /// Forgets a popup and the ones it opened. `dispose` also asks the host to
    /// close it; a host that reported the close itself has done so.
    fn popup_closed(&self, popup: BrowserId, dispose: bool) {
        let removed = self.state.with(|state| state.remove_popup(popup));
        if removed.is_empty() {
            return;
        }
        for id in &removed {
            if dispose || *id != popup {
                let _ = self.browser.dispose(*id);
            }
        }
        self.publish_page(popup.site);
    }

    /// Closes every popup over `site`'s view.
    fn close_popups(&self, site: SchoolSite) {
        let opened = self.state.with(|state| {
            state
                .views
                .get(&site)
                .map(|view| {
                    view.popups
                        .iter()
                        .filter(|popup| popup.parent == view.id)
                        .map(|popup| popup.id)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        for popup in opened {
            self.popup_closed(popup, true);
        }
    }

    fn history_changed(&self, browser: BrowserId, history: History) {
        let known = self.state.with(|state| {
            let view = state.views.get_mut(&browser.site)?;
            if view.id == browser {
                view.history = Some(history);
                return Some(());
            }
            view.popups
                .iter_mut()
                .find(|popup| popup.id == browser)?
                .history = Some(history);
            Some(())
        });
        if known.is_some() {
            self.publish_page(browser.site);
        }
    }

    /// Tells the app the page the person sees of `site` changed. Only the
    /// site on screen is told; `open` and `page` stamp the others.
    fn publish_page(&self, site: SchoolSite) {
        let page = self.state.with(|state| {
            if state.browsing != Some(site) {
                return None;
            }
            let (page, changed) = state.stamp_page(site, self.capabilities.inline_popups);
            changed.then_some(page)
        });
        if let Some(page) = page {
            let _ = self.events.emit(CoreEvent::SchoolWebPage(page));
        }
    }

    /// Forgets a view generation the shell closed or could not create, so the
    /// next request creates a new view instead of dispatching into nothing.
    fn closed(&self, browser: BrowserId) {
        let view = {
            let mut state = self.state.guard();
            match state.views.get(&browser.site) {
                Some(view) if view.id == browser => state.views.remove(&browser.site),
                _ => None,
            }
        };
        let Some(_view) = view else {
            self.popup_closed(browser, false);
            return;
        };
        // The host closes a view's popups with it.
        self.publish_page(browser.site);
        let _ = self.traces.send(Trace::Closed(browser));
    }

    /// Forgets a view and asks the shell to close it, so the next request
    /// creates a new generation.
    fn forget(&self, browser: BrowserId) {
        let popups = self.state.with(|state| {
            state
                .views
                .get(&browser.site)
                .filter(|view| view.id == browser)
                .map(|view| view.popups.iter().map(|popup| popup.id).collect::<Vec<_>>())
                .unwrap_or_default()
        });
        self.closed(browser);
        for popup in popups {
            let _ = self.browser.dispose(popup);
        }
        let _ = self.browser.dispose(browser);
    }

    /// Stops the view an abandoned attempt left mid-way: its loads end and
    /// Core's scripts in it stand down. A view the person is looking at is
    /// theirs and keeps going.
    fn halt(&self, browser: BrowserId) {
        // Shutting down closes every view anyway.
        if self.tasks.is_closed() {
            return;
        }
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        let marked = self.state.with(|state| {
            if state.browsing == Some(browser.site) {
                return false;
            }
            match state.views.get_mut(&browser.site) {
                Some(view) if view.id == browser => {
                    view.stopping = Some(action);
                    true
                }
                _ => false,
            }
        });
        if !marked {
            return;
        }
        let sent = self.sent(
            browser,
            self.browser.dispatch(BrowserCommand {
                browser,
                action,
                command: BrowserAction::Stop,
            }),
        );
        if let Err(error) = sent {
            // A view that cannot take the stop is replaced, never reused.
            log::warn!(target: "app_core::school_browser", "stop not sent site={:?}: {error}", browser.site);
            self.forget(browser);
        }
    }

    fn stopped(&self, page: PageId, action: ActionId) {
        let confirmed = self.state.with(|state| {
            let Some(view) = state.views.get_mut(&page.browser.site) else {
                return false;
            };
            if view.id != page.browser || view.stopping != Some(action) {
                return false;
            }
            view.stopping = None;
            // Pages up to the stop that report late were the abandoned
            // attempt's; the next attempt must not take them for its own.
            view.last_page = view.last_page.max(page.sequence);
            true
        });
        if confirmed {
            let _ = self.traces.send(Trace::Halted(page.browser));
        }
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
        let navigation = matches!(
            command,
            BrowserAction::Navigate(_)
                | BrowserAction::Reload
                | BrowserAction::GoBack
                | BrowserAction::GoForward
        );
        // A navigation Core sends leaves the failure it may fix behind.
        let cleared = navigation
            && self.state.with(|state| {
                let Some(view) = state.views.get_mut(&browser.site) else {
                    return false;
                };
                let failure = if view.id == browser {
                    view.navigation = Some(action);
                    if view.actions.len() >= 64 {
                        view.actions.clear();
                    }
                    view.actions.insert(action, true);
                    &mut view.failure
                } else if let Some(popup) = view.popups.iter_mut().find(|popup| popup.id == browser)
                {
                    &mut popup.failure
                } else {
                    return false;
                };
                failure.take().is_some()
            });
        if cleared {
            self.publish_page(browser.site);
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
    automation_target(raw).ok_or_else(|| {
        SchoolBrowserError::InvalidInput("School URL is outside the adapter sites.".into())
    })
}

/// Logs a refusal with the URL's scheme and host only: paths and queries
/// carry SSO tokens and personal data.
fn denied(site: SchoolSite, role: Option<ViewRole>, reason: DenyReason, url: &Url) {
    log::info!(
        target: "app_core::school_browser",
        "denied site={site:?} role={role:?} reason={reason:?} url={}://{}",
        url.scheme(),
        url.host_str().unwrap_or("")
    );
}
fn interactive_url(raw: &str) -> Result<Url, SchoolBrowserError> {
    let safe = browsing_url(raw).ok_or_else(|| {
        SchoolBrowserError::InvalidInput(
            "Enter an HTTP or HTTPS URL without embedded credentials.".into(),
        )
    })?;
    Url::parse(&safe).map_err(|_| SchoolBrowserError::InvalidInput("Web URL is invalid.".into()))
}
fn new_id() -> String {
    uuid::Uuid::now_v7().simple().to_string()
}
