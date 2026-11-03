//! Web operations: the one way Core signs a school view in.
//!
//! An operation starts only from the person's explicit action: an App link
//! that opens a new target (`Go`, a `Return` taken as one, a view's first
//! page), or the App's Reload while the page in front of the view is a
//! known sign-in page. Resuming a view, a surface mounting again, or a page
//! that moves to the sign-in page by itself start nothing, and nothing
//! watches a page once its operation ended.
//!
//! A Reload's operation, like the App's Reload anywhere else, opens the last
//! school page of the visit with a GET (`Visit`), on every platform: no
//! form is ever sent again.
//!
//! An operation carries its id, its view (or the popup in front, for a
//! Reload), the account and storage generations it began in, and one
//! deadline. A new navigation of the view, closing it, or a sign-out or
//! reset supersede it; repeated Reloads join it. It sends the password at most
//! once: getting the identity ready may sign in, and then it never signs in
//! again; only when a verified identity was written and the page still
//! lands on the sign-in page does it sign in, write and open the target
//! once more. The network, maintenance and the store's failures never send
//! a password. While it signs in the view is covered and the person may
//! cancel; the view comes back only once a write already handed to the
//! platform has ended.

use super::*;
use crate::school_session::{
    Prepared, WebStop, WebWrite, SEED_WAIT, WEB_OPERATION_DEADLINE, WEB_STORE_LIMIT,
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
}

fn on_provider(url: &Url) -> bool {
    url.host_str()
        .and_then(SchoolAuthProfile::for_auth_host)
        .is_some()
}

impl Visit {
    /// A main-frame page arrived. The identity provider's pages and sign-in
    /// routes leave the visit as it was.
    pub(super) fn arrived(&mut self, url: &Url) {
        // The identity provider also hosts these two ordinary portal pages.
        let portal = [SchoolSite::SciencePortal, SchoolSite::TokyoPortal]
            .iter()
            .filter_map(|site| Url::parse(site.start_url()).ok())
            .any(|entry| entry.origin() == url.origin() && entry.path() == url.path());
        if (on_provider(url) && !portal) || is_login_url(url) {
            return;
        }
        if let Some(last) = reopen_url(url.as_str()) {
            self.last = Some(last);
        }
    }

    /// Where the App's Reload opens: the last page recorded. While none is
    /// (the view opened on the sign-in page, or only on pages that are no
    /// school site's), it is `site`'s start page: the one address
    /// known to open with a GET, where the engine's own reload might send a
    /// form again.
    pub(super) fn reopen(&self, site: SchoolSite) -> Option<Url> {
        self.last
            .clone()
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
    /// The person's Reload on the sign-in page: it may go through a pause
    /// after a failure, and it stops at once when the identity cannot be
    /// made ready.
    reload: bool,
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
        let reload = matches!(start, Start::Reload);
        // A Reload stays covered until its page arrives; an App link only
        // while the identity needs the school.
        let covering = reload || self.sessions.fresh(profile).is_none();
        let planned = self.state.with(|state| {
            let target = match start {
                Start::Go(url) => url,
                Start::Reload => state
                    .visit(browser)
                    .cloned()
                    .unwrap_or_default()
                    .reopen(site)?,
            };
            let view = state.views.get_mut(&site)?;
            view.problem = None;
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
        log::info!(target: "app_core::school_browser", "web operation {id} started site={site:?} reload={reload}");
        let plan = Plan {
            id,
            site,
            browser,
            profile,
            account: self.sessions.account_generation(profile),
            storage: self.sessions.web_storage(),
            deadline: Instant::now() + WEB_OPERATION_DEADLINE,
            target,
            reload,
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
        plan: Plan,
        mut landed: mpsc::UnboundedReceiver<Landing>,
        cancel: CancellationToken,
    ) {
        let mut write = None;
        let ended = {
            let work = self.operate(&plan, &mut landed, &mut write);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Some(None),
                _ = self.tasks.closed() => Some(None),
                ended = tokio::time::timeout_at(plan.deadline, work) => ended.ok(),
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
        // given back.
        if let Some(write) = write.as_mut() {
            let _ = write.wait(WEB_STORE_LIMIT).await;
        }
        self.finish(plan.site, plan.id, problem);
    }

    async fn operate(
        &self,
        plan: &Plan,
        landed: &mut mpsc::UnboundedReceiver<Landing>,
        write: &mut Option<WebWrite>,
    ) -> Option<SchoolWebProblem> {
        // ① The identity: as it is, verified, or signed in (the one chance).
        let (mut prepared, mut stop) =
            match self.sessions.prepare_web(plan.profile, plan.reload).await {
                Ok(prepared) => (Some(prepared), None),
                Err(stop) => (None, Some(stop)),
            };
        if !self.still(plan) {
            return None;
        }
        // ② Into the web view, through the one exit.
        if let Some(ready) = prepared {
            match self.write(plan, ready.identity, write).await {
                Ok(identity) => {
                    prepared = Some(Prepared { identity, ..ready });
                }
                Err(failed) => {
                    stop = Some(failed);
                    prepared = None;
                }
            }
        }
        // A Reload on the sign-in page has nothing to open without it; an
        // App link opens its target anyway, and says why only if the page
        // asks for a sign-in.
        if let (true, Some(stop)) = (plan.reload, stop) {
            return problem(stop);
        }
        if !self.still(plan) {
            return None;
        }
        // ③ The target.
        if !plan.reload {
            self.cover(plan, false);
        }
        self.open_target(plan)?;
        match landed.recv().await.unwrap_or(Landing::Failed) {
            Landing::Arrived | Landing::Failed => return None,
            Landing::SignIn => {}
        }
        // ④ The page asks for a sign-in although the identity was written.
        if let Some(stop) = stop {
            return problem(stop);
        }
        let ready = prepared?;
        if ready.signed_in {
            return Some(SchoolWebProblem::StillSignIn);
        }
        self.cover(plan, true);
        log::info!(target: "app_core::school_browser", "web operation {} signing in: the page refused the verified identity", plan.id);
        let identity = match self
            .sessions
            .sign_in_web(plan.profile, ready.identity, plan.reload)
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
        self.open_target(plan)?;
        match landed.recv().await.unwrap_or(Landing::Failed) {
            Landing::SignIn => Some(SchoolWebProblem::StillSignIn),
            Landing::Arrived | Landing::Failed => None,
        }
    }

    /// Writes `identity`, or the verified generation that replaced it while
    /// the write waited for its turn, and returns the generation written.
    /// An earlier write of the operation ends first; one that has not
    /// answered within `SEED_WAIT` stays queued and is waited for before the
    /// view is given back.
    async fn write(
        &self,
        plan: &Plan,
        mut identity: u64,
        slot: &mut Option<WebWrite>,
    ) -> Result<u64, WebStop> {
        if let Some(earlier) = slot.as_mut() {
            let _ = earlier.wait(WEB_STORE_LIMIT).await;
        }
        for _ in 0..2 {
            let answer = match self.sessions.seed(plan.profile, identity) {
                Ok(mut queued) => {
                    let answer = queued.wait(SEED_WAIT).await;
                    *slot = Some(queued);
                    answer.unwrap_or(Err(WebStop::Storage))
                }
                Err(stop) => Err(stop),
            };
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
            let wanted = view.op.as_ref().is_some_and(|op| op.covering);
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
            state
                .views
                .get(&site)
                .and_then(|view| view.op.as_ref())
                .map(|op| op.cancel.clone())
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
        let gone = self.state.with(|state| std::mem::take(&mut state.views));
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
