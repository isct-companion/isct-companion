use super::policy::{
    browsing_url, is_identity_login, is_login_url, is_logout_url, navigation_rule, new_window_rule,
    reopen_url, restore_record, DenyReason, Verdict,
};
use super::{
    ActionId, BecameDownload, BrowserAction, BrowserCommand, BrowserError, BrowserId,
    DownloadDisposition, History, HostCapabilities, LoadFailed, LoadFailure, NavigationDecision,
    NavigationKind, NavigationRequest, NewWindowDecision, NewWindowRequest, OtpScript, PageId,
    PageLoaded, SchoolBrowser, SchoolBrowserError, SchoolBrowserListener, SchoolOtpEnrollment,
    SchoolOtpIntent, SchoolOtpStage, SchoolPageCustomization, SchoolWebAccess, SchoolWebEntry,
    SchoolWebFailure, SchoolWebNotice, SchoolWebPage, SchoolWebProblem, SchoolWebResult,
    SchoolWebStep,
};
use crate::{
    events::CoreEvent,
    lifecycle::SchoolSite,
    ports::EventSink,
    runtime_tasks::RuntimeTasks,
    school_auth::SchoolAuthProfile,
    school_session::{AccountViews, SchoolSessions},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError, Weak,
    },
};
use url::Url;

mod operation;
mod otp;
mod restore;

use operation::{Landing, Start, Visit, WebOp};

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
    events: Arc<dyn EventSink>,
    /// The accounts' identities, written into the web view before an
    /// App-initiated navigation.
    sessions: SchoolSessions,
    browser: Arc<dyn SchoolBrowser>,
    capabilities: HostCapabilities,
    state: Mutex<State>,
    tasks: RuntimeTasks,
    runtime: tokio::runtime::Handle,
    next_generation: AtomicU64,
    next_action: AtomicU64,
    listener: Mutex<Option<Arc<dyn SchoolBrowserListener>>>,
    restore_io: Mutex<()>,
}

#[derive(Default)]
struct State {
    views: HashMap<SchoolSite, ViewState>,
    /// The site on screen; only its page is published.
    browsing: Option<SchoolSite>,
    otp: Option<otp::Enrollment>,
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
    otp_loading: bool,
    url: String,
    navigation: Option<ActionId>,
    actions: HashMap<ActionId, bool>,
    /// The web operation signing the view in (`operation`).
    op: Option<WebOp>,
    /// The operation whose cookie write the platform has not answered: a
    /// fact of the view, not of that operation, so nothing superseding it
    /// opens over the write.
    write_pending: Option<u64>,
    /// That write began under a cover, which stays until it ends.
    write_covers: bool,
    /// It ran past `WEB_STORE_LIMIT`: the view is covered and says so.
    write_storing: bool,
    /// Whether a write is pending, for the operation waiting to start.
    write_changed: tokio::sync::watch::Sender<bool>,
    /// A navigation sent while a write was pending, sent when it ends.
    deferred_navigation: Option<(BrowserId, BrowserAction)>,
    /// What the last operation left to say, until the view shows another
    /// page.
    problem: Option<SchoolWebProblem>,
    /// The host was told to cover the view.
    covered: bool,
    visit: Visit,
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
    visit: Visit,
}

impl State {
    /// Whether the view or popup `browser` still exists.
    fn alive(&self, browser: BrowserId) -> bool {
        self.views.get(&browser.site).is_some_and(|view| {
            view.id == browser || view.popups.iter().any(|popup| popup.id == browser)
        })
    }

    /// The visit of the view or popup `browser`.
    fn visit(&self, browser: BrowserId) -> Option<&Visit> {
        let view = self.views.get(&browser.site)?;
        if view.id == browser {
            return Some(&view.visit);
        }
        view.popups
            .iter()
            .find(|popup| popup.id == browser)
            .map(|popup| &popup.visit)
    }

    fn visit_mut(&mut self, browser: BrowserId) -> Option<&mut Visit> {
        let view = self.views.get_mut(&browser.site)?;
        if view.id == browser {
            return Some(&mut view.visit);
        }
        view.popups
            .iter_mut()
            .find(|popup| popup.id == browser)
            .map(|popup| &mut popup.visit)
    }

    /// The address Core last saw in the view or popup `browser`.
    fn known_url(&self, browser: BrowserId) -> Option<String> {
        let view = self.views.get(&browser.site)?;
        if view.id == browser {
            return (view.last_page > 0).then(|| view.url.clone());
        }
        view.popups
            .iter()
            .find(|popup| popup.id == browser)
            .and_then(|popup| popup.url.clone())
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
            access: None,
            sign_in: false,
            otp: (site == SchoolSite::SciencePortal)
                .then(|| self.otp.as_ref().map(|otp| otp.view.clone()))
                .flatten(),
        };
        let Some(view) = self.views.get(&site) else {
            return page;
        };
        page.access = match &view.op {
            _ if view.write_storing => Some(SchoolWebAccess::Problem {
                problem: SchoolWebProblem::Storage,
            }),
            _ if view.wants_cover() => Some(SchoolWebAccess::SigningIn),
            _ => view
                .problem
                .map(|problem| SchoolWebAccess::Problem { problem }),
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
                page.sign_in = view.op.is_none() && popup.visit.offer;
            }
            None => {
                page.url = (view.last_page > 0).then(|| view.url.clone());
                page.back = step(view.history.map(|history| history.back));
                page.forward = step(view.history.map(|history| history.forward));
                page.notice = view.failure.clone();
                page.sign_in = view.op.is_none() && view.visit.offer;
            }
        }
        if page
            .otp
            .as_ref()
            .is_some_and(|_| self.otp.as_ref().is_some_and(otp::Enrollment::active))
        {
            page.sign_in = false;
            page.access = None;
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

/// The account whose identity the web view needs for `site`: the sites
/// signed in through Extic and through GetAccess. Public sites need none.
fn site_profile(site: SchoolSite) -> Option<SchoolAuthProfile> {
    match site {
        SchoolSite::SciencePortal | SchoolSite::ScienceLms => Some(SchoolAuthProfile::Science),
        SchoolSite::TokyoPortal
        | SchoolSite::TokyoMail
        | SchoolSite::Kyomu
        | SchoolSite::Library
        | SchoolSite::Tsubame => Some(SchoolAuthProfile::TokyoTech),
        SchoolSite::Syllabus | SchoolSite::StudyGuide => None,
    }
}

struct Listener(Weak<Inner>);

/// The target `entry` sends `site`'s view to when `surface` opens it, and
/// `presenter` showed it last; `None` keeps the page as it is
/// (`approach/school-web-front-page`). Only the entry and the presenter
/// decide, never the address the view shows: Resume and a Return to the
/// surface that presented it keep the page, while Go, a Return from another
/// surface (taken as a Go) and an unsigned entry always navigate, so an
/// explicit Go to the page already shown still opens it and may start the
/// identity's recovery.
pub(super) fn entry_target(
    entry: SchoolWebEntry,
    presenter: Option<&str>,
    surface: &str,
    site: SchoolSite,
) -> Option<String> {
    match entry {
        SchoolWebEntry::Resume => None,
        SchoolWebEntry::Return { .. } if presenter == Some(surface) => None,
        SchoolWebEntry::Go { url } | SchoolWebEntry::Return { url } => Some(url),
        SchoolWebEntry::Unsigned => Some(site.start_url().to_string()),
        SchoolWebEntry::OtpSetup { .. } => Some(otp::ENTRY.into()),
    }
}

impl SchoolBrowserEngine {
    pub fn new(
        sessions: SchoolSessions,
        events: Arc<dyn EventSink>,
        browser: Arc<dyn SchoolBrowser>,
    ) -> Result<Arc<Self>, SchoolBrowserError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            SchoolBrowserError::Platform("School browser requires an async runtime.".into())
        })?;
        let inner = Arc::new(Inner {
            events,
            sessions,
            capabilities: browser.capabilities(),
            browser: browser.clone(),
            state: Mutex::new(State::default()),
            tasks: RuntimeTasks::default(),
            runtime,
            next_generation: AtomicU64::new(1),
            next_action: AtomicU64::new(1),
            listener: Mutex::new(None),
            restore_io: Mutex::new(()),
        });
        let listener: Arc<dyn SchoolBrowserListener> = Arc::new(Listener(Arc::downgrade(&inner)));
        browser.attach(Arc::downgrade(&listener))?;
        inner.listener.guard().replace(listener);
        let views: std::sync::Weak<dyn AccountViews> = Arc::downgrade(&inner) as _;
        inner.sessions.attach_views(views);
        Ok(Arc::new(Self(inner)))
    }

    /// Which site is on screen, the one whose page Core publishes.
    pub fn user_browsing(&self, site: Option<SchoolSite>) -> Result<(), SchoolBrowserError> {
        {
            let mut state = self.0.state.guard();
            if state.browsing == site {
                return Ok(());
            }
            state.browsing = site;
            if site != Some(SchoolSite::SciencePortal) {
                if let Some(otp) = state.otp.as_mut() {
                    otp.stop();
                }
            }
        }
        // Changes made while the site was off screen were not published.
        if let Some(site) = site {
            self.0.publish_page(site);
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
        let (view, current, presenter) = {
            let state = self.0.state.guard();
            let view = state.views.get(&site);
            (
                view.map(|v| v.id),
                view.filter(|v| v.last_page > 0).map(|v| v.url.clone()),
                view.and_then(|v| v.presenter.clone()),
            )
        };
        let already_open = view.is_some();
        let enrollment = match &entry {
            SchoolWebEntry::OtpSetup { username } if site == SchoolSite::SciencePortal => {
                Some(username.clone())
            }
            SchoolWebEntry::OtpSetup { .. } => {
                return Err(SchoolBrowserError::InvalidInput(
                    "OTP setup requires the Science Tokyo portal.".into(),
                ))
            }
            _ => None,
        };
        let unsigned = entry == SchoolWebEntry::Unsigned || enrollment.is_some();
        let target = entry_target(entry, presenter.as_deref(), surface, site);
        if enrollment.is_none() && site == SchoolSite::SciencePortal && target.is_some() {
            self.0.leave_otp();
        }
        if unsigned {
            self.cancel_sign_in(site)?;
        }
        let explicit_target = target.is_some();
        let target = target
            .or(current)
            .or_else(|| self.restored_target(site))
            .unwrap_or_else(|| site.start_url().to_string());
        let url = interactive_url(&target)?;
        // A view the App sends somewhere: a new one, or one an entry gives
        // a target, whatever page it shows now. Revealing a view as it is
        // navigates nothing, and keeps its exact native URL.
        let navigates = view.is_none() || explicit_target;
        // Where the account's identity can be written, a web operation opens
        // the target (`operation`); an unsigned entry never does.
        let operation =
            site_profile(site).filter(|_| navigates && !unsigned && self.0.sessions.seeds_web());
        self.user_browsing(Some(site))?;
        // A new view that gets the account's identity starts blank: its
        // store exists before the cookies are written, and its first page
        // loads after.
        let created = match view {
            Some(browser) => Ok(browser),
            None if operation.is_some() || enrollment.is_some() => Url::parse("about:blank")
                .map_err(|_| SchoolBrowserError::InvalidInput("Web URL is invalid.".into()))
                .and_then(|initial| self.ensure_browser(site, initial)),
            None => self.ensure_browser(site, url.clone()),
        };
        let opened = (|| {
            let browser = created?;
            self.0.state.with(|state| {
                if let Some(view) = state.views.get_mut(&site) {
                    view.presenter = Some(surface.to_owned());
                }
            });
            if unsigned {
                self.0.state.with(|state| {
                    if let Some(visit) = state.visit_mut(browser) {
                        visit.hold();
                    }
                });
            }
            // An app link lands in the view itself, never under a popup.
            if already_open && explicit_target {
                self.0.close_popups(site);
            }
            if let Some(username) = enrollment.as_ref() {
                if self.start_otp(browser, username)? {
                    self.dispatch(browser, BrowserAction::Navigate(url.clone()))?;
                }
            } else {
                match operation {
                    Some(profile) => self.0.start(site, browser, profile, Start::Go(url.clone())),
                    None if already_open && navigates => {
                        self.dispatch(browser, BrowserAction::Navigate(url.clone()))?;
                    }
                    // A view coming back on the sign-in page it reached while
                    // hidden.
                    None => {
                        let front = self.0.state.with(|state| {
                            let front = state.front(site, self.0.capabilities.inline_popups)?;
                            let url = Url::parse(&state.known_url(front)?).ok()?;
                            is_identity_login(&url).then_some(front)
                        });
                        if let Some(front) = front {
                            self.0.sign_in_page(front);
                        }
                    }
                }
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
        if site == SchoolSite::SciencePortal {
            self.0.leave_otp();
        }
        let url = interactive_url(raw)?;
        self.user_browsing(Some(site))?;
        self.ensure_browser(site, url.clone())?;
        let browser = self.front(site)?;
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        Ok(true)
    }

    /// The app's Reload, the same on every platform: it opens the last
    /// school page of the view or popup in front again with a GET
    /// (`Visit::reopen`), never the engine's reload, so no form is sent
    /// twice. On a known sign-in page of a site with an account it starts a
    /// web operation that signs in first and then opens that same page
    /// (`operation`), and a Reload while one signs the view in joins it. The
    /// page's `sign_in` button calls this too: the way out when the automatic
    /// recovery got stuck.
    pub fn reload(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        if site == SchoolSite::SciencePortal {
            self.0.leave_otp();
        }
        let Ok(browser) = self.front(site) else {
            return Ok(false);
        };
        let (joins, on_sign_in, reopen) = self.0.state.with(|state| {
            let on_sign_in = state
                .known_url(browser)
                .and_then(|url| Url::parse(&url).ok())
                .is_some_and(|url| is_identity_login(&url));
            // An operation signing the view in, or one whose page is the
            // sign-in page, takes the Reload in; one only loading its target
            // gives way to it.
            let joins = state
                .views
                .get(&site)
                .and_then(|view| view.op.as_ref())
                .is_some_and(|op| op.covering || on_sign_in);
            let reopen = state
                .visit(browser)
                .cloned()
                .unwrap_or_default()
                .reopen(site);
            (joins, on_sign_in, reopen)
        });
        if joins {
            return Ok(true);
        }
        if let Some(profile) =
            site_profile(site).filter(|_| on_sign_in && self.0.sessions.seeds_web())
        {
            self.0.start(site, browser, profile, Start::Reload);
            return Ok(true);
        }
        let Some(url) = reopen else {
            return Ok(false);
        };
        self.dispatch(browser, BrowserAction::Navigate(url))?;
        Ok(true)
    }

    /// The app's Back, through the engine's own history. A popup at the
    /// start of its history closes instead, as a browser closes a tab a page
    /// opened, and the page under it is in front again.
    pub fn back(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        if site == SchoolSite::SciencePortal {
            self.0.leave_otp();
        }
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
        if site == SchoolSite::SciencePortal {
            self.0.leave_otp();
        }
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
            if self.0.sessions.web_cleanup_pending() {
                return Err(SchoolBrowserError::SignOutIncomplete);
            }
            if let Some(view) = state.views.get(&site) {
                return Ok(view.id);
            }
            let id = BrowserId {
                site,
                generation: self.0.next_generation.fetch_add(1, Ordering::Relaxed),
            };
            state.views.insert(
                site,
                ViewState {
                    id,
                    last_page: 0,
                    otp_loading: false,
                    url: initial.to_string(),
                    navigation: None,
                    actions: HashMap::new(),
                    op: None,
                    write_pending: None,
                    write_covers: false,
                    write_storing: false,
                    write_changed: tokio::sync::watch::channel(false).0,
                    deferred_navigation: None,
                    problem: None,
                    covered: false,
                    visit: Visit::default(),
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
    fn otp_script_allowed(&self, page: PageId, action: ActionId, url: &Url) -> bool {
        self.0
            .upgrade()
            .is_some_and(|inner| inner.otp_script_allowed(page, action, url))
    }
    fn otp_script_result(&self, page: PageId, action: ActionId, result: Option<String>) {
        if let Some(inner) = self.0.upgrade() {
            inner.otp_result(page, action, result);
        }
    }
    fn load_failed(&self, event: LoadFailed) {
        if let Some(inner) = self.0.upgrade() {
            inner.load_failed(event);
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

    /// A popup's page moves the address the person sees; Core runs no script
    /// or restore in it, and only recovers its sign-in page.
    fn popup_loaded(self: &Arc<Self>, event: PageLoaded) {
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
            popup.visit.arrived(&event.url);
            Some(())
        });
        if moved.is_some() {
            self.publish_page(browser.site);
            if let Some(landing) = Landing::of(browser.site, &event.url) {
                self.landed(browser, event.cause, landing);
                if landing == Landing::SignIn {
                    self.sign_in_page(browser);
                }
            }
        }
    }

    fn page_available(self: &Arc<Self>, event: PageLoaded) {
        if !self.current(event.page.browser) {
            return;
        }
        let official_loaded = restore_record(event.url.as_str());
        let Some(safe_loaded) = browsing_url(event.url.as_str()) else {
            return;
        };
        let (customization, guide, restore, foreground) = {
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
            view.otp_loading = false;
            if let Some(cause) = event.cause {
                view.actions.remove(&cause);
            } else if let Some(action) = view.navigation {
                view.actions.remove(&action);
            }
            view.navigation = None;
            view.url = safe_loaded.clone();
            view.failure = None;
            view.visit.arrived(&event.url);
            // Another page: what an operation left to say is over.
            if view.op.is_none() {
                view.problem = None;
            }
            (
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
        if event.page.browser.site == SchoolSite::SciencePortal
            && self
                .state
                .with(|state| state.otp.as_ref().is_some_and(otp::Enrollment::active))
        {
            self.otp_page(&event);
            return;
        }
        // The opener behind an inline popup does not move the page on screen.
        if foreground {
            self.publish_page(event.page.browser.site);
        }
        if let (Some((path, epoch)), Some(official)) = (restore, official_loaded.as_ref()) {
            self.write_restore(path, epoch, event.page.browser.site, official.clone());
        }
        if let Some(landing) = Landing::of(event.page.browser.site, &event.url) {
            self.landed(event.page.browser, event.cause, landing);
            if landing == Landing::SignIn {
                self.sign_in_page(event.page.browser);
            }
        }
        // External browsing gets history/address updates, never school
        // scripts, customizations or a pending submission.
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
        self.state.with(|state| {
            let Some(view) = state.views.get_mut(&browser.site) else {
                return;
            };
            if view.id != browser {
                if let Some(popup) = view.popups.iter_mut().find(|popup| popup.id == browser) {
                    if event.page.sequence > popup.last_page {
                        popup.last_page = event.page.sequence;
                        popup.failure = Some(failure);
                    }
                }
                return;
            }
            if let Some(cause) = event.cause {
                if view.actions.remove(&cause) != Some(true) {
                    return;
                }
            } else if event.page.sequence <= view.last_page {
                return;
            }
            view.last_page = view.last_page.max(event.page.sequence);
            view.navigation = None;
            view.failure = Some(failure);
            if browser.site == SchoolSite::SciencePortal {
                if let Some(otp) = state.otp.as_mut() {
                    otp.stop();
                }
            }
        });
        self.publish_page(browser.site);
        self.landed(browser, event.cause, Landing::Failed);
    }

    fn notice(&self, site: SchoolSite, notice: SchoolWebNotice) {
        let _ = self
            .events
            .emit(CoreEvent::SchoolWebNotice { site, notice });
    }

    /// Records a load of `page` the rule table refused as its view's or
    /// popup's failure.
    fn refused(&self, page: PageId, failure: SchoolWebFailure) {
        let browser = page.browser;
        let recorded = self.state.with(|state| {
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
            state.browsing == Some(browser.site)
                && state.front(browser.site, self.capabilities.inline_popups) == Some(browser)
        })
    }

    fn alive(&self, browser: BrowserId) -> bool {
        !self.tasks.is_closed() && self.state.with(|state| state.alive(browser))
    }

    fn decide_navigation(&self, request: NavigationRequest) -> NavigationDecision {
        let browser = request.page.browser;
        if !self.alive(browser) {
            denied(browser.site, DenyReason::Closed, &request.url);
            return NavigationDecision::Deny(DenyReason::Closed);
        }
        match navigation_rule(&request) {
            Verdict::Allow | Verdict::Popup => {
                self.state.with(|state| {
                    if request.main_frame && browser.site == SchoolSite::SciencePortal {
                        if let Some(view) = state
                            .views
                            .get_mut(&browser.site)
                            .filter(|view| view.id == browser)
                        {
                            view.otp_loading = true;
                        }
                    }
                    if let Some(visit) = state.visit_mut(browser) {
                        visit.requested(&request);
                    }
                });
                NavigationDecision::Allow
            }
            Verdict::External => {
                log::info!(target: "app_core::school_browser", "to system browser site={:?} host={}", browser.site, request.url.host_str().unwrap_or("-"));
                NavigationDecision::External(request.url)
            }
            Verdict::Deny(reason) => {
                denied(browser.site, reason, &request.url);
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
                denied(browser.site, DenyReason::Resubmission, &request.url);
                NavigationDecision::SkipEntry
            }
            Verdict::ReloadAsGet => {
                denied(browser.site, DenyReason::Resubmission, &request.url);
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
        if !self.alive(opener) {
            denied(opener.site, DenyReason::Closed, &request.url);
            return NewWindowDecision::Deny(DenyReason::Closed);
        }
        match new_window_rule(&request) {
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
                        visit: Visit::default(),
                    });
                    Some(())
                });
                let Some(()) = open else {
                    denied(opener.site, DenyReason::TooManyPopups, &request.url);
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
                denied(opener.site, reason, &request.url);
                NewWindowDecision::Deny(reason)
            }
            Verdict::Allow | Verdict::SkipEntry | Verdict::ReloadAsGet => {
                denied(opener.site, DenyReason::Scheme, &request.url);
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
        let removed = self.state.with(|state| {
            let removed = state.remove_popup(popup);
            // An operation on a popup that closed ends with it.
            if let Some(op) = state
                .views
                .get(&popup.site)
                .and_then(|view| view.op.as_ref())
                .filter(|op| removed.contains(&op.browser))
            {
                op.cancel();
            }
            removed
        });
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
                Some(view) if view.id == browser => {
                    if browser.site == SchoolSite::SciencePortal {
                        if let Some(otp) = state.otp.as_mut() {
                            otp.stop();
                        }
                    }
                    state.views.remove(&browser.site)
                }
                _ => None,
            }
        };
        let Some(view) = view else {
            self.popup_closed(browser, false);
            return;
        };
        if let Some(op) = &view.op {
            op.cancel();
        }
        // The host closes a view's popups with it.
        self.publish_page(browser.site);
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
        self.dispatch_for(browser, command, None)
    }

    /// Sends `command` to `browser`. A navigation that is not operation
    /// `op`'s own supersedes the view's operation.
    fn dispatch_for(
        &self,
        browser: BrowserId,
        command: BrowserAction,
        op: Option<u64>,
    ) -> Result<ActionId, SchoolBrowserError> {
        let action = ActionId(self.next_action.fetch_add(1, Ordering::Relaxed));
        let navigation = matches!(
            command,
            BrowserAction::Navigate(_) | BrowserAction::GoBack | BrowserAction::GoForward
        );
        let mut superseded = None;
        let mut deferred = false;
        // A navigation Core sends leaves the failure it may fix behind.
        let cleared = navigation
            && self.state.with(|state| {
                let Some(view) = state.views.get_mut(&browser.site) else {
                    return false;
                };
                if view.write_pending.is_some() {
                    if op.is_none() {
                        superseded = view.op.take();
                        view.deferred_navigation = Some((browser, command.clone()));
                    }
                    deferred = true;
                    return false;
                }
                if op.is_none() {
                    superseded = view.op.take();
                    view.problem = None;
                }
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
        if let Some(superseded) = &superseded {
            superseded.cancel();
            self.sync_cover(browser.site);
        }
        if cleared || superseded.is_some() || deferred {
            self.publish_page(browser.site);
        }
        if deferred {
            return Ok(action);
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

/// Logs a refusal with the URL's scheme and host only: paths and queries
/// carry SSO tokens and personal data.
fn denied(site: SchoolSite, reason: DenyReason, url: &Url) {
    log::info!(
        target: "app_core::school_browser",
        "denied site={site:?} reason={reason:?} url={}://{}",
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
