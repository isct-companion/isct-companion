//! Web operations: the one way Core signs a school view in, as a browser
//! would treat a sign-in: open the page first, and act only when the page
//! lands on the identity provider's sign-in page.
//!
//! An operation starts from an App link that opens a new target (`Go`, a
//! `Return` taken as one, a view's first page), from the App's Reload while
//! the page in front is a known sign-in page, or from that page reaching the
//! view or popup on screen by itself (`Landed`). The last runs once per
//! `Visit::spent`: only the person's own navigation or an App action allows
//! another, so a page that keeps going back to the sign-in page cannot keep
//! sending the password. It does not run after the person's Back, their own
//! sign-out, a form they submit on the sign-in page, an unsigned entry or
//! "Cancel, I'll sign in myself" (`Visit::hold`).
//!
//! A Reload's operation, like the App's Reload anywhere else, opens the last
//! school page of the visit with a GET (`Visit`), on every platform: no
//! form is ever sent again.
//!
//! An operation carries its id, its view (or the popup in front), the account
//! and storage generations it began in, and one deadline. A new navigation of
//! the view, closing it, or a sign-out or reset supersede it; repeated
//! Reloads join it. It sends the password at most once: getting the identity
//! ready may sign in, and then it never signs in again; only when a verified
//! identity was written and the page still lands on the sign-in page does it
//! sign in, write and open the target once more. The network, maintenance
//! and the store's failures never send a password. While it recovers the view
//! is covered and the person may cancel. A write already handed to the
//! platform is the view's fact (`ViewState::write_pending`): it outlives the
//! operation, the next one waits for it, a navigation sent meanwhile is held,
//! and a view it covers comes back only when it ended.

use super::*;
use crate::school_session::{
    SchoolSessionState, WebStop, WebWrite, SEED_WAIT, WEB_OPERATION_DEADLINE, WEB_STORE_LIMIT,
};
use tokio::{sync::mpsc, time::Instant};
use tokio_util::sync::CancellationToken;

/// The last page of a view's or popup's visit that the App's Reload opens
/// again.
#[derive(Clone, Debug, Default)]
pub(super) struct Visit {
    /// The last school page that arrived, however it was reached, as
    /// `reopen_url` keeps it: the page itself, without its fragment or what
    /// would act or leak. Reload opens it with a GET, so a page a form
    /// produced is reopened by its address and the form is never sent
    /// again. Memory only: the restore record on disk is narrower.
    last: Option<Url>,
    /// The page the person's link asked for, until a page arrives: when the
    /// link ended on the sign-in page, this is where they were going.
    asked: Option<Url>,
    /// The automatic recovery was used; only the person's own navigation or
    /// an App action allows another.
    spent: bool,
    /// The person brought this sign-in page about, or wants it: no automatic
    /// recovery until a school page arrives or they act again.
    hold: bool,
    /// The page is a sign-in page and the account's saved credentials may
    /// sign it in: the manual way out when the automatic one got stuck.
    pub(super) offer: bool,
}

fn on_provider(url: &Url) -> bool {
    url.host_str()
        .and_then(SchoolAuthProfile::for_auth_host)
        .is_some()
}

impl Visit {
    /// The view or popup asks to navigate; a rule table allowed it.
    pub(super) fn requested(&mut self, request: &NavigationRequest) {
        if !request.main_frame {
            return;
        }
        let url = &request.url;
        let submits_sign_in =
            matches!(request.kind, NavigationKind::FormSubmit) && is_identity_login(url);
        if submits_sign_in
            || matches!(request.kind, NavigationKind::BackForward)
            || is_logout_url(url)
        {
            self.hold();
            return;
        }
        let link = matches!(request.kind, NavigationKind::Link);
        if (request.user_gesture || link) && !is_identity_login(url) {
            self.allow();
            self.asked = link.then(|| reopen_url(url.as_str())).flatten();
        }
    }

    /// An App link's target: where a sign-in on the way should come back to.
    pub(super) fn ask(&mut self, url: &Url) {
        self.asked = reopen_url(url.as_str());
    }

    /// The person's action: another automatic recovery may run.
    pub(super) fn allow(&mut self) {
        self.spent = false;
        self.hold = false;
    }

    pub(super) fn hold(&mut self) {
        self.hold = true;
        self.asked = None;
    }

    pub(super) fn spend(&mut self) {
        self.spent = true;
    }

    /// Whether the automatic recovery may run, with a page to return to.
    pub(super) fn recovers(&self) -> bool {
        !self.spent && !self.hold && (self.last.is_some() || self.asked.is_some())
    }

    /// A main-frame page arrived. The identity provider's pages and sign-in
    /// routes leave the visit as it was.
    pub(super) fn arrived(&mut self, url: &Url) {
        self.offer = false;
        // The identity provider also hosts these two ordinary portal pages.
        let portal = [SchoolSite::SciencePortal, SchoolSite::TokyoPortal]
            .iter()
            .filter_map(|site| Url::parse(site.start_url()).ok())
            .any(|entry| entry.origin() == url.origin() && entry.path() == url.path());
        if (on_provider(url) && !portal) || is_login_url(url) {
            return;
        }
        self.hold = false;
        self.asked = None;
        if let Some(last) = reopen_url(url.as_str()) {
            self.last = Some(last);
        }
    }

    /// Where the App's Reload opens: the page the person's link asked for,
    /// or else the last page recorded. While none is
    /// (the view opened on the sign-in page, or only on pages that are no
    /// school site's), it is `site`'s start page: the one address
    /// known to open with a GET, where the engine's own reload might send a
    /// form again.
    pub(super) fn reopen(&self, site: SchoolSite) -> Option<Url> {
        self.asked
            .clone()
            .or_else(|| self.last.clone())
            .or_else(|| Url::parse(site.start_url()).ok())
    }
}

/// Where a page of an operation's navigation arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Landing {
    /// The identity provider's sign-in page.
    SignIn,
    /// Anywhere else but the provider's hand-off pages.
    Arrived,
    Failed,
}

impl Landing {
    /// Where `url` is for a view of `site`; `None` on a hand-off page.
    pub(super) fn of(site: SchoolSite, url: &Url) -> Option<Self> {
        if is_identity_login(url) {
            return Some(Self::SignIn);
        }
        // The provider's own site (the Science Tokyo portal on Extic) is a
        // destination; its other pages are the trip.
        if on_provider(url) && SchoolSite::for_url(url.as_str()) != Some(site) {
            return None;
        }
        Some(Self::Arrived)
    }
}

/// The operation running for a view.
pub(super) struct WebOp {
    id: u64,
    /// The view or popup it navigates.
    pub(super) browser: BrowserId,
    cancel: CancellationToken,
    /// The navigation it sent, whose pages it waits for.
    action: Option<ActionId>,
    landed: mpsc::UnboundedSender<Landing>,
    /// It covers the view now.
    pub(super) covering: bool,
}

impl WebOp {
    pub(super) fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// What starts an operation.
pub(super) enum Start {
    /// An App link to this target.
    Go(Url),
    /// The App's Reload on a sign-in page: it opens the visit's last page
    /// (`Visit::reopen`).
    Reload,
    /// The page on screen reached the sign-in page by itself; opens the same
    /// page as Reload.
    Landed,
}

struct Plan {
    id: u64,
    site: SchoolSite,
    browser: BrowserId,
    profile: SchoolAuthProfile,
    account: u64,
    storage: u64,
    deadline: Instant,
    /// Where it opens, with a GET, once the identity is written.
    target: Url,
    /// An App link: it opens the target first, and recovers only if that
    /// lands on the sign-in page.
    open_first: bool,
    /// Its cancel: a write it queued that has not reached the platform yet
    /// is dropped once it is cancelled.
    cancel: CancellationToken,
    /// The person's Reload on the sign-in page: it may go through a pause
    /// after a failure.
    past_pause: bool,
}

fn problem(stop: WebStop) -> Option<SchoolWebProblem> {
    match stop {
        WebStop::Unreachable => Some(SchoolWebProblem::Unreachable),
        WebStop::Rejected => Some(SchoolWebProblem::Rejected),
        WebStop::NeedsWeb | WebStop::NotConfigured => Some(SchoolWebProblem::StillSignIn),
        WebStop::Storage => Some(SchoolWebProblem::Storage),
        // Superseded, or nothing Core could write: nothing to say.
        WebStop::Stale | WebStop::Unsupported => None,
    }
}

impl ViewState {
    /// Whether Core keeps the view out of sight now: an operation signing it
    /// in, a write begun under a cover, or a write past its limit. An App
    /// link's own write leaves its page in view (`Plan::open_first`).
    pub(super) fn wants_cover(&self) -> bool {
        self.write_storing
            || (self.write_pending.is_some() && self.write_covers)
            || self.op.as_ref().is_some_and(|op| op.covering)
    }
}

impl Inner {
    /// Starts an operation for `site`'s view, superseding the one running.
    pub(super) fn start(
        self: &Arc<Self>,
        site: SchoolSite,
        browser: BrowserId,
        profile: SchoolAuthProfile,
        start: Start,
    ) {
        let id = self.next_action.fetch_add(1, Ordering::Relaxed);
        let cancel = CancellationToken::new();
        let (landed, receiver) = mpsc::unbounded_channel();
        let open_first = matches!(start, Start::Go(_));
        let past_pause = matches!(start, Start::Reload);
        // A recovery stays covered until its page arrives; an App link shows
        // its page unless that page asks for a sign-in.
        let covering = !open_first;
        let planned = self.state.with(|state| {
            let visit = state.visit_mut(browser)?;
            if !matches!(start, Start::Landed) {
                visit.allow();
            }
            if covering {
                visit.spend();
            }
            let target = match start {
                Start::Go(url) => {
                    visit.ask(&url);
                    url
                }
                Start::Reload | Start::Landed => visit.reopen(site)?,
            };
            let view = state.views.get_mut(&site)?;
            view.problem = None;
            view.deferred_navigation = None;
            let previous = view.op.replace(WebOp {
                id,
                browser,
                cancel: cancel.clone(),
                action: None,
                landed,
                covering,
            });
            Some((target, previous))
        });
        let Some((target, previous)) = planned else {
            return;
        };
        if let Some(previous) = previous {
            previous.cancel();
        }
        log::info!(target: "app_core::school_browser", "web operation {id} started site={site:?} open_first={open_first}");
        let plan = Plan {
            id,
            site,
            browser,
            profile,
            account: self.sessions.account_generation(profile),
            storage: self.sessions.web_storage(),
            deadline: Instant::now() + WEB_OPERATION_DEADLINE,
            target,
            open_first,
            past_pause,
            cancel: cancel.clone(),
        };
        self.sync_cover(site);
        self.publish_page(site);
        let inner = self.clone();
        let _entered = self.runtime.enter();
        let started = self.tasks.spawn(async move {
            inner.run(plan, receiver, cancel).await;
        });
        if !started {
            self.finish(site, id, None);
        }
    }

    async fn run(
        self: Arc<Self>,
        mut plan: Plan,
        mut landed: mpsc::UnboundedReceiver<Landing>,
        cancel: CancellationToken,
    ) {
        let mut write = None;
        let mut changed = self.state.with(|state| {
            state
                .views
                .get(&plan.site)
                .map(|view| view.write_changed.subscribe())
        });
        let ended = {
            let work = async {
                if let Some(changed) = changed.as_mut() {
                    loop {
                        let pending = *changed.borrow_and_update();
                        if !pending {
                            break;
                        }
                        if changed.changed().await.is_err() {
                            return Some(None);
                        }
                    }
                }
                plan.deadline = Instant::now() + WEB_OPERATION_DEADLINE;
                tokio::time::timeout_at(plan.deadline, self.operate(&plan, &mut landed, &mut write))
                    .await
                    .ok()
            };
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Some(None),
                _ = self.tasks.closed() => Some(None),
                ended = work => ended,
            }
        };
        let problem = match ended {
            Some(problem) => problem,
            // Out of time: a view still covered never got its sign-in.
            None => self
                .state
                .with(|state| {
                    state
                        .views
                        .get(&plan.site)
                        .and_then(|view| view.op.as_ref())
                        .is_some_and(|op| op.id == plan.id && op.covering)
                })
                .then_some(SchoolWebProblem::Unreachable),
        };
        // A write already handed to the platform ends before the view is
        // given back. Past `WEB_STORE_LIMIT` the view stays covered and says
        // the store has not finished, until the write really ended.
        if let Some(write) = write.as_mut() {
            if write.wait(WEB_STORE_LIMIT).await.is_none() {
                self.storing(&plan);
                // Only the view waits for as long as the write takes. Core's
                // exit does not (`SchoolBrowserEngine::shutdown` drains this
                // task before the sessions stop the queue), and the view
                // stays covered: its write never ended.
                tokio::select! {
                    biased;
                    _ = write.ended() => {}
                    _ = self.tasks.closed() => return,
                }
            }
        }
        self.end_write(&plan);
        self.finish(plan.site, plan.id, problem);
    }

    async fn operate(
        &self,
        plan: &Plan,
        landed: &mut mpsc::UnboundedReceiver<Landing>,
        write: &mut Option<WebWrite>,
    ) -> Option<SchoolWebProblem> {
        // ① An App link: its page, with the identity written first only when
        // that needs nothing from the school. Most pages end here.
        if plan.open_first {
            if let Some(identity) = self.sessions.fresh(plan.profile) {
                let written = self.write(plan, identity, write).await;
                if !self.still(plan) {
                    return None;
                }
                // The platform still has it: nothing opens over it.
                if written.is_err() && write.as_ref().is_some_and(WebWrite::pending) {
                    return Some(SchoolWebProblem::Storage);
                }
            }
            if self.open_and_land(plan, landed).await? != Landing::SignIn {
                return None;
            }
            // Nothing saved to sign in with: the page is the person's.
            let session = self.sessions.snapshot();
            if session.profile(plan.profile).state == SchoolSessionState::NotConfigured {
                return None;
            }
            self.state.with(|state| {
                if let Some(visit) = state.visit_mut(plan.browser) {
                    visit.spend();
                }
            });
            self.cover(plan, true);
        }
        // ② The identity: as it is, verified, or signed in (the one chance).
        let ready = match self
            .sessions
            .prepare_web(plan.profile, plan.past_pause)
            .await
        {
            Ok(ready) => ready,
            Err(stop) => return problem(stop),
        };
        if !self.still(plan) {
            return None;
        }
        // ③ Into the web view, through the one exit, and the target again.
        let identity = match self.write(plan, ready.identity, write).await {
            Ok(identity) => identity,
            Err(stop) => return problem(stop),
        };
        if !self.still(plan) {
            return None;
        }
        if self.open_and_land(plan, landed).await? != Landing::SignIn {
            return None;
        }
        // ④ The page asks for a sign-in although the identity was written.
        if ready.signed_in {
            return Some(SchoolWebProblem::StillSignIn);
        }
        log::info!(target: "app_core::school_browser", "web operation {} signing in: the page refused the verified identity", plan.id);
        let identity = match self
            .sessions
            .sign_in_web(plan.profile, identity, plan.past_pause)
            .await
        {
            Ok(identity) => identity,
            Err(stop) => return problem(stop),
        };
        if !self.still(plan) {
            return None;
        }
        if let Err(stop) = self.write(plan, identity, write).await {
            return problem(stop);
        }
        if !self.still(plan) {
            return None;
        }
        match self.open_and_land(plan, landed).await? {
            Landing::SignIn => Some(SchoolWebProblem::StillSignIn),
            Landing::Arrived | Landing::Failed => None,
        }
    }

    /// Opens the target and waits for where it arrives. `None`: it could not
    /// be sent. A landing still queued from before says nothing about this
    /// navigation.
    async fn open_and_land(
        &self,
        plan: &Plan,
        landed: &mut mpsc::UnboundedReceiver<Landing>,
    ) -> Option<Landing> {
        while landed.try_recv().is_ok() {}
        self.open_target(plan)?;
        Some(landed.recv().await.unwrap_or(Landing::Failed))
    }

    /// Writes `identity`, or the verified generation that replaced it while
    /// the write waited for its turn, and returns the generation written.
    /// An earlier write of the operation ends first. A queued write's handle
    /// is in `slot` before anything is awaited, so one that has not answered
    /// within `SEED_WAIT`, or whose operation was stopped meanwhile, is
    /// still waited for before the view is given back.
    async fn write(
        &self,
        plan: &Plan,
        mut identity: u64,
        slot: &mut Option<WebWrite>,
    ) -> Result<u64, WebStop> {
        if let Some(earlier) = slot.as_mut() {
            if earlier.wait(WEB_STORE_LIMIT).await.is_none() {
                return Err(WebStop::Storage);
            }
        }
        for _ in 0..2 {
            let started = self.state.with(|state| {
                let Some(view) = state.views.get_mut(&plan.site) else {
                    return false;
                };
                if !view.op.as_ref().is_some_and(|op| op.id == plan.id)
                    || view.write_pending.is_some()
                {
                    return false;
                }
                view.write_pending = Some(plan.id);
                view.write_covers = view.op.as_ref().is_some_and(|op| op.covering);
                view.write_changed.send_replace(true);
                true
            });
            if !started {
                return Err(WebStop::Stale);
            }
            self.sync_cover(plan.site);
            let answer = match self
                .sessions
                .seed(plan.profile, identity, plan.cancel.clone())
            {
                Ok(queued) => slot.insert(queued).wait(SEED_WAIT).await,
                Err(stop) => Some(Err(stop)),
            };
            if answer.is_some() {
                self.end_write(plan);
            }
            let answer = answer.unwrap_or(Err(WebStop::Storage));
            match answer {
                Ok(()) => return Ok(identity),
                Err(WebStop::Stale) => match self.sessions.fresh(plan.profile) {
                    Some(newer) if newer != identity => identity = newer,
                    _ => return Err(WebStop::Stale),
                },
                Err(stop) => return Err(stop),
            }
        }
        Err(WebStop::Stale)
    }

    /// Whether the operation still stands: its view kept it, and neither
    /// the account nor the store changed.
    fn still(&self, plan: &Plan) -> bool {
        !self.tasks.is_closed()
            && self.sessions.account_generation(plan.profile) == plan.account
            && self.sessions.web_storage() == plan.storage
            && self.state.with(|state| {
                state.alive(plan.browser)
                    && state
                        .views
                        .get(&plan.site)
                        .and_then(|view| view.op.as_ref())
                        .is_some_and(|op| op.id == plan.id)
            })
    }

    fn open_target(&self, plan: &Plan) -> Option<()> {
        let url = plan.target.clone();
        log::info!(target: "app_core::school_browser", "web operation {} opens host={}", plan.id, url.host_str().unwrap_or("-"));
        let action = self
            .dispatch_for(plan.browser, BrowserAction::Navigate(url), Some(plan.id))
            .ok()?;
        self.state.with(|state| {
            if let Some(op) = state
                .views
                .get_mut(&plan.site)
                .and_then(|view| view.op.as_mut())
                .filter(|op| op.id == plan.id)
            {
                op.action = Some(action);
            }
        });
        Some(())
    }

    fn cover(&self, plan: &Plan, covering: bool) {
        let changed = self.state.with(|state| {
            let op = state
                .views
                .get_mut(&plan.site)?
                .op
                .as_mut()
                .filter(|op| op.id == plan.id)?;
            (op.covering != covering).then(|| op.covering = covering)
        });
        if changed.is_some() {
            self.sync_cover(plan.site);
            self.publish_page(plan.site);
        }
    }

    /// Operation `plan` ended with a write the platform has not finished:
    /// its view is covered and says so until the write ends.
    fn storing(&self, plan: &Plan) {
        let changed = self.state.with(|state| {
            let view = state.views.get_mut(&plan.site)?;
            if view.write_pending != Some(plan.id) {
                return None;
            }
            view.write_storing = true;
            Some(())
        });
        if changed.is_some() {
            self.sync_cover(plan.site);
            self.publish_page(plan.site);
        }
    }

    fn end_write(&self, plan: &Plan) {
        let ended = self.state.with(|state| {
            let view = state.views.get_mut(&plan.site)?;
            if view.write_pending != Some(plan.id) {
                return None;
            }
            view.write_pending = None;
            view.write_storing = false;
            view.write_changed.send_replace(false);
            Some(view.deferred_navigation.take())
        });
        if let Some(deferred) = ended {
            self.sync_cover(plan.site);
            self.publish_page(plan.site);
            if let Some((browser, command)) = deferred.filter(|_| !self.tasks.is_closed()) {
                let _ = self.dispatch(browser, command);
            }
        }
    }

    /// Ends operation `id` of `site`'s view, leaving `problem` on the view.
    fn finish(&self, site: SchoolSite, id: u64, problem: Option<SchoolWebProblem>) {
        let ended = self.state.with(|state| {
            let view = state.views.get_mut(&site)?;
            if view.op.as_ref()?.id != id {
                return None;
            }
            view.op = None;
            view.problem = problem;
            Some(())
        });
        if ended.is_some() {
            log::info!(target: "app_core::school_browser", "web operation {id} ended site={site:?} problem={problem:?}");
            self.sync_cover(site);
            self.publish_page(site);
        }
    }

    /// Tells the host to cover or give back `site`'s view, as its operation
    /// wants it.
    pub(super) fn sync_cover(&self, site: SchoolSite) {
        let change = self.state.with(|state| {
            let view = state.views.get_mut(&site)?;
            let wanted = view.wants_cover();
            (view.covered != wanted).then(|| {
                view.covered = wanted;
                (view.id, wanted)
            })
        });
        if let Some((view, covered)) = change {
            if let Err(error) = self.browser.cover(view, covered) {
                log::warn!(target: "app_core::school_browser", "{:?} view not covered={covered}: {error}", view.site);
            }
        }
    }

    /// `browser` shows the sign-in page: whether saved credentials can sign
    /// it in, and, on screen and with no operation running, the automatic
    /// recovery or why there is none.
    pub(super) fn sign_in_page(self: &Arc<Self>, browser: BrowserId) {
        let site = browser.site;
        if site == SchoolSite::SciencePortal
            && self.state.with(|state| state.otp.as_ref().is_some_and(otp::Enrollment::active))
        {
            return;
        }
        let Some(profile) = site_profile(site).filter(|_| self.sessions.seeds_web()) else {
            return;
        };
        let session = self.sessions.snapshot().profile(profile).clone();
        let saved = !session.sign_out_pending;
        let signs_in = saved
            && matches!(
                session.state,
                SchoolSessionState::Normal | SchoolSessionState::SigningIn
            );
        let offer = signs_in || (saved && session.state == SchoolSessionState::Unavailable);
        let problem = match session.state {
            SchoolSessionState::Refused => Some(SchoolWebProblem::Rejected),
            SchoolSessionState::NeedsWeb => Some(SchoolWebProblem::StillSignIn),
            SchoolSessionState::Unavailable => Some(SchoolWebProblem::Unreachable),
            _ => None,
        };
        let shown = self.shown(browser);
        let recover = self.state.with(|state| {
            let idle = state.views.get(&site).is_some_and(|view| view.op.is_none());
            let visit = state.visit_mut(browser)?;
            visit.offer = offer;
            let recover = idle && shown && signs_in && visit.recovers();
            if idle {
                state.views.get_mut(&site)?.problem = problem;
            }
            Some(recover)
        });
        match recover {
            Some(true) => self.start(site, browser, profile, Start::Landed),
            Some(false) => self.publish_page(site),
            None => {}
        }
    }

    /// A page of `browser`'s navigation `cause` arrived, or its load failed:
    /// the operation waiting for it hears it.
    pub(super) fn landed(&self, browser: BrowserId, cause: Option<ActionId>, landing: Landing) {
        self.state.with(|state| {
            let Some(op) = state
                .views
                .get(&browser.site)
                .and_then(|view| view.op.as_ref())
            else {
                return;
            };
            // A page script's form (a SAML hand-off) has no cause; another
            // navigation already superseded the operation.
            if op.browser == browser
                && op.action.is_some()
                && (cause.is_none() || cause == op.action)
            {
                let _ = op.landed.send(landing);
            }
        });
    }
}

impl SchoolBrowserEngine {
    /// The person's "Cancel, I'll sign in myself" on a view Core is signing
    /// in. The view comes back once a write already handed to the platform
    /// has ended.
    pub fn cancel_sign_in(&self, site: SchoolSite) -> Result<bool, SchoolBrowserError> {
        let op = self.0.state.with(|state| {
            let (browser, cancel) = state
                .views
                .get(&site)
                .and_then(|view| view.op.as_ref())
                .map(|op| (op.browser, op.cancel.clone()))?;
            if let Some(visit) = state.visit_mut(browser) {
                visit.hold();
            }
            Some(cancel)
        });
        Ok(match op {
            Some(cancel) => {
                log::info!(target: "app_core::school_browser", "web operation cancelled site={site:?}");
                cancel.cancel();
                true
            }
            None => false,
        })
    }
}

impl AccountViews for Inner {
    /// The account's sign-out: its operations end and its views close.
    fn stop(&self) {
        let gone = self.state.with(|state| {
            if let Some(otp) = state.otp.as_mut() { otp.stop(); }
            std::mem::take(&mut state.views)
        });
        for view in gone.into_values() {
            if let Some(op) = &view.op {
                op.cancel();
            }
            log::info!(target: "app_core::school_browser", "{:?} view closed for a sign-out", view.id.site);
            let _ = self.browser.dispose(view.id);
            self.publish_page(view.id.site);
        }
    }
}
