//! Site sessions and `run`: the one way reads reach the LMS and 教務Web.
//!
//! A site always has at most one instance: a jar that began as a copy of
//! the identity snapshot, plus its id. A request carries the instance it
//! set out with and its answer is written to that instance only. A session
//! that has expired retires the whole instance; the next one is built from
//! the current snapshot and goes through the site's SSO again, one SSO per
//! site at a time. Only an SSO that reaches the identity provider's sign-in
//! page asks the identity to sign in.
//!
//! A top-level `run` has one recovery: the operation gets this attempt's
//! handle, never the site session, so it cannot start a nested run with a
//! recovery of its own; a second loss of the session ends the call. Across
//! calls the identity's cooldown bounds sign-ins.

use super::{
    guard,
    jar::{Cookies, Jar},
    login::{self, KYOMU_ENTRY, KYOMU_STUDENT, LMS_HOME},
    walk::{Page, Walk, WalkError},
    Inner, Permit,
};
use crate::{
    net::{BoxFuture, Host, NetError, NetRequest, Response},
    school_auth::SchoolAuthProfile,
};
use adpt_isct::login::SciencePage;
use reqwest::header::COOKIE;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;
use url::Url;

/// A website with its own session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Site {
    Lms,
    /// 教務Web in Japanese.
    Kyomu,
    /// 教務Web in English: its own server session, switched to English when
    /// it is set up, so the Japanese one is never touched.
    KyomuEnglish,
}

impl Site {
    pub(crate) fn profile(self) -> SchoolAuthProfile {
        match self {
            Self::Lms => SchoolAuthProfile::Science,
            Self::Kyomu | Self::KyomuEnglish => SchoolAuthProfile::TokyoTech,
        }
    }

    pub(crate) fn host(self) -> Host {
        match self {
            Self::Lms => Host::Moodle,
            Self::Kyomu | Self::KyomuEnglish => Host::Kyomu,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Lms => "lms",
            Self::Kyomu => "kyomu",
            Self::KyomuEnglish => "kyomu-en",
        }
    }

    /// The site whose session reads `url`'s files, if any.
    pub(crate) fn for_url(url: &Url) -> Option<Self> {
        match Host::of(url)? {
            Host::Moodle => Some(Self::Lms),
            Host::Kyomu => Some(Self::Kyomu),
            _ => None,
        }
    }
}

/// One instance of a site session.
pub(crate) struct Instance {
    id: u64,
    /// The identity generation its jar was copied from.
    identity: u64,
    jar: Mutex<Jar>,
}

impl Cookies for Instance {
    fn header(&self, url: &Url) -> Option<String> {
        self.jar.header(url)
    }

    fn store(&self, url: &Url, set_cookie: &[String]) {
        self.jar.store(url, set_cookie);
    }
}

pub(super) struct SiteState {
    instance: Option<Arc<Instance>>,
    next: u64,
    /// The school's maintenance notice, until a read succeeds again.
    pub(super) notice: Option<String>,
}

pub(super) struct SiteSession {
    pub(super) site: Site,
    pub(super) state: Mutex<SiteState>,
    /// One SSO per site at a time.
    sso: tokio::sync::Mutex<()>,
    /// 教務Web keeps each page's ViewState on the server, so an instance runs
    /// one operation at a time. LMS reads are independent.
    serial: Option<tokio::sync::Mutex<()>>,
}

impl SiteSession {
    pub(super) fn new(site: Site) -> Self {
        Self {
            site,
            state: Mutex::new(SiteState {
                instance: None,
                next: 1,
                notice: None,
            }),
            sso: tokio::sync::Mutex::new(()),
            serial: (site != Site::Lms).then(|| tokio::sync::Mutex::new(())),
        }
    }

    fn current(&self) -> Option<Arc<Instance>> {
        guard(&self.state).instance.clone()
    }

    fn build(&self, jar: Jar, identity: u64) -> Arc<Instance> {
        let mut state = guard(&self.state);
        let id = state.next;
        state.next += 1;
        Arc::new(Instance {
            id,
            identity,
            jar: Mutex::new(jar),
        })
    }

    fn install(&self, instance: Arc<Instance>) {
        guard(&self.state).instance = Some(instance);
    }

    /// A jar read back from the vault, taken as ready until a request finds
    /// otherwise.
    pub(super) fn install_restored(&self, jar: Jar, identity: u64) {
        let instance = self.build(jar, identity);
        let mut state = guard(&self.state);
        if state.instance.is_none() {
            state.instance = Some(instance);
        }
    }

    /// Retires `instance` unless a rebuild already replaced it.
    fn retire(&self, instance: &Instance) {
        let mut state = guard(&self.state);
        if state
            .instance
            .as_ref()
            .is_some_and(|current| current.id == instance.id)
        {
            state.instance = None;
        }
    }

    pub(super) fn forget(&self) {
        let mut state = guard(&self.state);
        state.instance = None;
        state.notice = None;
    }

    pub(super) fn saved_jar(&self) -> Option<String> {
        self.current()
            .and_then(|instance| instance.jar.lock().ok().and_then(|jar| jar.save()))
    }
}

/// What a read operation's error says about the session.
pub(crate) trait OpError {
    /// The school answered with its sign-in instead of the data.
    fn signed_out(&self) -> bool;
}

impl OpError for NetError {
    fn signed_out(&self) -> bool {
        matches!(self, NetError::SignInRequired)
    }
}

/// A read: run once, and once more after a recovered session. Each attempt
/// builds its requests again (sesskeys, ViewState and tokens are fetched
/// anew), and nothing is handed out before the operation ends, so a rerun
/// starts clean.
pub(crate) trait Operation: Send {
    type Output: Send;
    type Error: OpError + Send;

    fn attempt(&mut self, attempt: Attempt) -> BoxFuture<'_, Result<Self::Output, Self::Error>>;
}

/// Why a `run` ended without its data.
#[derive(Debug)]
pub(crate) enum RunError<E> {
    /// The operation's own error.
    Op(E),
    /// The account cannot reach its school now; its state is in the snapshot.
    Session,
    /// The site's SSO failed on the network or a paused host.
    Net(NetError),
    /// The account's barrier stopped the read.
    Cancelled,
    /// The caller's deadline passed.
    Deadline,
}

/// One attempt's handle: the site instance it set out with.
#[derive(Clone)]
pub(crate) struct Attempt(Arc<AttemptInner>);

struct AttemptInner {
    site: Site,
    instance: Arc<Instance>,
    sessions: Arc<Inner>,
    deadline: Instant,
    /// The account generation of the permit it runs under.
    account: u64,
}

impl Attempt {
    pub(crate) fn site(&self) -> Site {
        self.0.site
    }

    /// The account generation it runs in; a cache of an account (the LMS
    /// token) is kept per generation.
    pub(crate) fn account_generation(&self) -> u64 {
        self.0.account
    }

    /// The instance this attempt reads with; a cache tied to an instance
    /// compares it.
    pub(crate) fn instance(&self) -> u64 {
        self.0.instance.id
    }

    /// `request` with this instance's cookies; what the school sets is kept
    /// in this instance only. Redirects are returned, never followed.
    pub(crate) async fn send(&self, mut request: NetRequest) -> Result<Response, NetError> {
        if let Some(cookie) = self.0.instance.header(&request.url) {
            request.headers.push((COOKIE, cookie));
        }
        let url = request.url.clone();
        let response = self.bare(request).await?;
        let set_cookies = response.set_cookies();
        if !set_cookies.is_empty() {
            self.0.instance.store(&url, &set_cookies);
        }
        Ok(response)
    }

    /// `request` without cookies (the token REST service), within the
    /// call's deadline.
    pub(crate) async fn bare(&self, request: NetRequest) -> Result<Response, NetError> {
        tokio::time::timeout_at(self.0.deadline, self.0.sessions.transport.send(request))
            .await
            .map_err(|_| NetError::Deadline)?
    }

    /// The `Cookie` header for a file request the caller streams itself.
    pub(crate) fn cookie_header(&self, url: &Url) -> Option<String> {
        self.0.instance.header(url)
    }

    /// Keeps the `Set-Cookie` values of a response the caller streamed.
    pub(crate) fn store_cookies(&self, url: &Url, set_cookie: &[String]) {
        self.0.instance.store(url, set_cookie);
    }

    /// The site showed a maintenance page: its host pauses, the notice is
    /// kept for the session snapshot, and the read ends as a paused host.
    pub(crate) fn maintenance(&self, notice: Option<String>) -> NetError {
        let host = self.0.site.host();
        self.0.sessions.transport.report_maintenance(host);
        if let Some(session) = self
            .0
            .sessions
            .account(self.0.site.profile())
            .sites
            .iter()
            .find(|session| session.site == self.0.site)
        {
            guard(&session.state).notice = notice.or_else(|| Some(String::new()));
        }
        self.0.sessions.publish();
        NetError::HostPaused {
            host,
            until_unix: self
                .0
                .sessions
                .transport
                .paused_until(host)
                .unwrap_or_else(|| chrono::Utc::now().timestamp() + 5 * 60),
        }
    }

    /// An attempt over `jar`, outside any session, for transport tests.
    #[cfg(test)]
    pub(crate) fn detached(sessions: &super::SchoolSessions, site: Site, jar: Jar) -> Self {
        Self(Arc::new(AttemptInner {
            site,
            instance: Arc::new(Instance {
                id: 0,
                identity: 0,
                jar: Mutex::new(jar),
            }),
            sessions: sessions.0.clone(),
            deadline: Instant::now() + std::time::Duration::from_secs(600),
            account: 0,
        }))
    }
}

/// What one call has used of its recovery.
#[derive(Default)]
struct Budget {
    recovered: bool,
    signed_in: bool,
}

enum Sso {
    Ready,
    /// The SSO reached the identity provider's sign-in page.
    SignIn,
    Failed(NetError),
}

impl Inner {
    pub(super) async fn run<O: Operation>(
        self: &Arc<Self>,
        site: Site,
        permit: &Permit,
        deadline: Instant,
        operation: &mut O,
    ) -> Result<O::Output, RunError<O::Error>> {
        let profile = site.profile();
        self.restore(profile).await;
        let session = self.site(site);
        let _serial = match &session.serial {
            Some(serial) => Some(bounded(permit, deadline, serial.lock()).await?),
            None => None,
        };
        let mut budget = Budget::default();
        loop {
            let instance = self.ready(site, permit, deadline, &mut budget).await?;
            let attempt = Attempt(Arc::new(AttemptInner {
                site,
                instance: instance.clone(),
                sessions: self.clone(),
                deadline,
                account: permit.generation(),
            }));
            let result = bounded(permit, deadline, operation.attempt(attempt)).await?;
            match result {
                Ok(value) => {
                    self.recovered(site);
                    return Ok(value);
                }
                Err(error) if error.signed_out() => {
                    session.retire(&instance);
                    if budget.recovered || budget.signed_in {
                        log::warn!(target: "app_core::school_session", "{site:?} refused a recovered session");
                        self.account(profile).identity.anomaly(instance.identity);
                        self.publish();
                        return Err(RunError::Session);
                    }
                    log::info!(target: "app_core::school_session", "{site:?} session expired; rebuilding it");
                    budget.recovered = true;
                }
                Err(error) => return Err(RunError::Op(error)),
            }
        }
    }

    #[cfg(feature = "agent")]
    pub(super) async fn write_attempt(
        self: &Arc<Self>,
        site: Site,
        permit: &Permit,
        deadline: Instant,
    ) -> Result<Attempt, RunError<NetError>> {
        self.restore(site.profile()).await;
        let instance = self
            .ready(site, permit, deadline, &mut Budget::default())
            .await?;
        Ok(Attempt(Arc::new(AttemptInner {
            site,
            instance,
            sessions: self.clone(),
            deadline,
            account: permit.generation(),
        })))
    }

    /// A read succeeded: a maintenance notice the site showed is over.
    fn recovered(&self, site: Site) {
        let cleared = self
            .account(site.profile())
            .sites
            .iter()
            .filter(|session| session.site == site)
            .any(|session| guard(&session.state).notice.take().is_some());
        if cleared {
            self.publish();
        }
    }

    /// The site's ready instance: the current one, or a new one built from
    /// the current snapshot and taken through SSO, signing in once if the
    /// SSO reaches the sign-in page.
    async fn ready<E>(
        self: &Arc<Self>,
        site: Site,
        permit: &Permit,
        deadline: Instant,
        budget: &mut Budget,
    ) -> Result<Arc<Instance>, RunError<E>> {
        let profile = site.profile();
        let session = self.site(site);
        loop {
            if let Some(instance) = session.current() {
                return Ok(instance);
            }
            let _sso = bounded(permit, deadline, session.sso.lock()).await?;
            // Another call finished the SSO while this one waited.
            if let Some(instance) = session.current() {
                return Ok(instance);
            }
            let identity = &self.account(profile).identity;
            let Some((snapshot, generation)) = identity.snapshot() else {
                if budget.signed_in {
                    return Err(RunError::Session);
                }
                budget.signed_in = true;
                let seen = identity.generation();
                bounded(
                    permit,
                    deadline,
                    self.sign_in(profile, seen, super::identity::How::Background),
                )
                .await?
                .map_err(|_| RunError::Session)?;
                continue;
            };
            let instance = session.build(snapshot.jar(), generation);
            let outcome = bounded(permit, deadline, self.sso(site, &instance)).await?;
            match outcome {
                Sso::Ready => {
                    log::info!(target: "app_core::school_session", "{site:?} session ready instance={}", instance.id);
                    session.install(instance.clone());
                    self.persist(profile).await;
                    return Ok(instance);
                }
                Sso::SignIn => {
                    if budget.signed_in {
                        log::warn!(target: "app_core::school_session", "{site:?} sent a fresh sign-in back to the sign-in page");
                        identity.anomaly(generation);
                        self.publish();
                        return Err(RunError::Session);
                    }
                    budget.signed_in = true;
                    bounded(
                        permit,
                        deadline,
                        self.sign_in(profile, generation, super::identity::How::Background),
                    )
                    .await?
                    .map_err(|_| RunError::Session)?;
                }
                Sso::Failed(error) => return Err(RunError::Net(error)),
            }
        }
    }

    async fn sso(&self, site: Site, instance: &Arc<Instance>) -> Sso {
        let walk = Walk::new(self.transport.as_ref(), instance.as_ref());
        let ended = match site {
            Site::Lms => lms_sso(&walk).await,
            Site::Kyomu => kyomu_sso(&walk, false).await,
            Site::KyomuEnglish => kyomu_sso(&walk, true).await,
        };
        match ended {
            Ok(Some(notice)) => {
                // 教務Web's maintenance page: the host pauses, the notice
                // shows until a read gets through.
                self.transport.report_maintenance(site.host());
                if let Some(session) = self
                    .account(site.profile())
                    .sites
                    .iter()
                    .find(|session| session.site == site)
                {
                    guard(&session.state).notice = Some(notice);
                }
                self.publish();
                Sso::Failed(NetError::HostPaused {
                    host: site.host(),
                    until_unix: self
                        .transport
                        .paused_until(site.host())
                        .unwrap_or_else(|| chrono::Utc::now().timestamp() + 5 * 60),
                })
            }
            Ok(None) => Sso::Ready,
            Err(SsoError::SignIn) => Sso::SignIn,
            Err(SsoError::Walk(WalkError::Net(error))) => Sso::Failed(error),
            Err(SsoError::Walk(WalkError::Http(status))) => Sso::Failed(NetError::Http(status)),
            Err(SsoError::Walk(WalkError::Off(what))) => {
                log::warn!(target: "app_core::school_session", "{site:?} SSO: {what}");
                Sso::Failed(NetError::Invalid(what.into()))
            }
        }
    }
}

/// Races `work` against the barrier's cancel and the call's deadline.
async fn bounded<T, E>(
    permit: &Permit,
    deadline: Instant,
    work: impl std::future::Future<Output = T>,
) -> Result<T, RunError<E>> {
    tokio::select! {
        biased;
        _ = permit.cancel().cancelled() => Err(RunError::Cancelled),
        done = tokio::time::timeout_at(deadline, work) => done.map_err(|_| RunError::Deadline),
    }
}

enum SsoError {
    SignIn,
    Walk(WalkError),
}

impl From<WalkError> for SsoError {
    fn from(error: WalkError) -> Self {
        Self::Walk(error)
    }
}

/// The LMS's SSO: the dashboard, through Extic and the SAML hand-off. Only
/// the dashboard with the log-out action proves the session: the first LMS
/// answer already sets a guest `MoodleSession`.
async fn lms_sso(walk: &Walk<'_>) -> Result<Option<String>, SsoError> {
    let home = Url::parse(LMS_HOME).map_err(|_| WalkError::Off("bad LMS address"))?;
    let mut page = walk.get(home, None).await?;
    for _ in 0..4 {
        match adpt_isct::login::science(&page.html, page.url.as_str()) {
            SciencePage::Saml(form) => page = login::saml(walk, &form, &page).await?,
            SciencePage::Credentials(_) | SciencePage::Totp(_) | SciencePage::EmailOtp => {
                return Err(SsoError::SignIn)
            }
            SciencePage::Other => break,
        }
    }
    if Host::of(&page.url) == Some(Host::Moodle) && page.html.contains("login/logout.php") {
        return Ok(None);
    }
    if Host::of(&page.url) == Some(Host::ScienceAccount) {
        return Err(SsoError::SignIn);
    }
    Err(WalkError::Off("the LMS did not open its dashboard").into())
}

/// 教務Web's SSO: its sign-in entry, through GetAccess, to the student top.
/// The English instance then posts the header's switch to English, so every
/// later page of that server session is English. `Some` is a maintenance
/// notice.
async fn kyomu_sso(walk: &Walk<'_>, english: bool) -> Result<Option<String>, SsoError> {
    let entry = Url::parse(KYOMU_ENTRY).map_err(|_| WalkError::Off("bad 教務Web address"))?;
    let page = walk.get(entry, None).await?;
    if Host::of(&page.url) == Some(Host::Kyomu) {
        let payload = adpt_isct::kyomu::extract_page(page.url.as_str(), &page.html);
        if payload.status == "maintenance" {
            return Ok(Some(payload.message));
        }
    }
    if Host::of(&page.url) != Some(Host::Kyomu) || !page.url.path().starts_with(KYOMU_STUDENT) {
        return match adpt_isct::login::tokyo(&page.html, page.url.as_str()) {
            adpt_isct::login::TokyoPage::Other
                if matches!(
                    Host::of(&page.url),
                    Some(Host::Kyomu | Host::GetAccess | Host::GetAccessRouter)
                ) && !page.url.as_str().contains("ResourceList") =>
            {
                Err(WalkError::Off("教務Web did not open the student top").into())
            }
            _ => Err(SsoError::SignIn),
        };
    }
    if english {
        to_english(walk, &page).await?;
    }
    Ok(None)
}

/// Posts the course search page's language link, the switch the English
/// catalog reads were proven on. The student top offers the same link, but
/// switching there left the search page in Japanese (every English
/// department read refused its columns).
async fn to_english(walk: &Walk<'_>, page: &Page) -> Result<(), SsoError> {
    let search = page
        .url
        .join(crate::catalog::KYOMU_SEARCH_PATH)
        .map_err(|_| WalkError::Off("bad 教務Web search address"))?;
    let page = walk.get(search, Some(page.url.clone())).await?;
    let (url, html) = (page.url, page.html);
    let fields =
        adpt_isct::kyomu::search::language_postback(&html, adpt_isct::kyomu::search::TO_ENGLISH)
            .ok_or(WalkError::Off("the 教務Web language links changed"))?;
    log::info!(target: "app_core::school_session", "step: 教務Web in English");
    match walk.post(&url, &fields, &url, false).await? {
        super::walk::Step::Page(_) => Ok(()),
        super::walk::Step::Redirect(to) if Host::of(&to) == Some(Host::Kyomu) => {
            walk.get(to, Some(url)).await?;
            Ok(())
        }
        super::walk::Step::Redirect(_) => Err(SsoError::SignIn),
    }
}
