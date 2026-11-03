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
//! An attempt gets the site's instance only when it sends a request that
//! needs cookies: a request that carries its own credentials (the LMS
//! token REST service) goes out with no site session, so it never builds
//! one, never goes through SSO and never signs in.
//!
//! A top-level `run` has one recovery: the operation gets this attempt's
//! handle, never the site session, so it cannot start a nested run with a
//! recovery of its own; a second loss of the session ends the call. Across
//! calls the identity's pause bounds sign-ins.

use super::{
    guard,
    jar::{Cookies, Jar},
    login::{self, KYOMU_ENTRY, KYOMU_STUDENT, LMS_HOME},
    walk::{Page, Walk, WalkError},
    Ended, Inner, Permit, Snapshot, RETRY_NOT_BEFORE,
};
use crate::{
    net::{BoxFuture, Host, NetError, NetRequest, Response},
    school_auth::SchoolAuthProfile,
    sync_engine::AccountWait,
};
use adpt_isct::login::SciencePage;
use reqwest::header::COOKIE;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, PoisonError,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
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
    /// The account cannot reach its school now, and what lets its reads go
    /// on.
    Blocked(AccountWait),
    /// The site's SSO failed on the network or a paused host.
    Net(NetError),
    /// The account's barrier stopped the read.
    Cancelled,
    /// The caller's deadline passed.
    Deadline,
}

/// Why getting a site's instance ended a call.
#[derive(Clone, Debug)]
enum Stop {
    Blocked(AccountWait),
    Net(NetError),
    Cancelled,
    Deadline,
}

impl Stop {
    /// What a request that needed the instance answers instead; `run`
    /// returns the stop itself, whatever the operation made of it.
    fn net(&self) -> NetError {
        match self {
            Self::Blocked(AccountWait::Person) => NetError::SignInRequired,
            Self::Blocked(_) => NetError::Offline,
            Self::Net(error) => error.clone(),
            Self::Cancelled => NetError::Cancelled,
            Self::Deadline => NetError::Deadline,
        }
    }
}

impl<E> From<Stop> for RunError<E> {
    fn from(stop: Stop) -> Self {
        match stop {
            Stop::Blocked(wait) => Self::Blocked(wait),
            Stop::Net(error) => Self::Net(error),
            Stop::Cancelled => Self::Cancelled,
            Stop::Deadline => Self::Deadline,
        }
    }
}

/// One call on a site: its deadline, the cancel of the permit it runs
/// under, and its one sign-in.
struct Call {
    site: Site,
    sessions: Arc<Inner>,
    deadline: Instant,
    /// The account generation of the permit it runs under.
    account: u64,
    cancel: CancellationToken,
    signed_in: AtomicBool,
    /// Why getting the instance ended the call, once it did.
    stopped: Mutex<Option<Stop>>,
}

impl Call {
    fn stopped(&self) -> Option<Stop> {
        guard(&self.stopped).clone()
    }
}

/// One attempt's handle: the call, and the site instance it reads with
/// once a request needed one.
#[derive(Clone)]
pub(crate) struct Attempt(Arc<AttemptInner>);

struct AttemptInner {
    call: Arc<Call>,
    instance: tokio::sync::OnceCell<Arc<Instance>>,
}

impl Attempt {
    fn new(call: Arc<Call>) -> Self {
        Self(Arc::new(AttemptInner {
            call,
            instance: tokio::sync::OnceCell::new(),
        }))
    }

    pub(crate) fn site(&self) -> Site {
        self.0.call.site
    }

    /// The account generation it runs in; a cache of an account (the LMS
    /// token) is kept per generation.
    pub(crate) fn account_generation(&self) -> u64 {
        self.0.call.account
    }

    /// The site instance this attempt reads with: the current one, or a new
    /// one taken through SSO, signing in once if the SSO reaches the sign-in
    /// page. A failure ends the call (`run` returns it).
    async fn ready(&self) -> Result<&Arc<Instance>, NetError> {
        let call = &self.0.call;
        self.0
            .instance
            .get_or_try_init(|| async {
                if let Some(stop) = call.stopped() {
                    return Err(stop.net());
                }
                call.sessions.ready(call).await.map_err(|stop| {
                    let net = stop.net();
                    *guard(&call.stopped) = Some(stop);
                    net
                })
            })
            .await
    }

    /// A permit of this attempt's account generation for a vault write, to
    /// hold until the write lands (`owner`): `None` once a sign-out or reset
    /// closed the generation.
    pub(crate) fn write_permit(&self) -> Option<Permit> {
        let call = &self.0.call;
        call.sessions
            .account(call.site.profile())
            .owner
            .permit_of(call.account)
    }

    /// The instance this attempt reads with, got first if needed; a cache
    /// tied to an instance compares it.
    pub(crate) async fn instance(&self) -> Result<u64, NetError> {
        Ok(self.ready().await?.id)
    }

    /// `request` with this instance's cookies; what the school sets is kept
    /// in this instance only. Redirects are returned, never followed.
    pub(crate) async fn send(&self, mut request: NetRequest) -> Result<Response, NetError> {
        let instance = self.ready().await?.clone();
        if let Some(cookie) = instance.header(&request.url) {
            request.headers.push((COOKIE, cookie));
        }
        let url = request.url.clone();
        let response = self.bare(request).await?;
        let set_cookies = response.set_cookies();
        if !set_cookies.is_empty() {
            instance.store(&url, &set_cookies);
        }
        Ok(response)
    }

    /// `request` without cookies (the token REST service), within the
    /// call's deadline. It needs no site session.
    pub(crate) async fn bare(&self, request: NetRequest) -> Result<Response, NetError> {
        let call = &self.0.call;
        tokio::time::timeout_at(call.deadline, call.sessions.transport.send(request))
            .await
            .map_err(|_| NetError::Deadline)?
    }

    /// The `Cookie` header for a file request the caller streams itself.
    pub(crate) async fn cookie_header(&self, url: &Url) -> Result<Option<String>, NetError> {
        Ok(self.ready().await?.header(url))
    }

    /// Keeps the `Set-Cookie` values of a response the caller streamed.
    pub(crate) fn store_cookies(&self, url: &Url, set_cookie: &[String]) {
        if let Some(instance) = self.0.instance.get() {
            instance.store(url, set_cookie);
        }
    }

    /// The site showed a maintenance page: its host pauses, the notice is
    /// kept for the session snapshot, and the read ends as a paused host.
    pub(crate) fn maintenance(&self, notice: Option<String>) -> NetError {
        let call = &self.0.call;
        let host = call.site.host();
        call.sessions.transport.report_maintenance(host);
        if let Some(session) = call
            .sessions
            .account(call.site.profile())
            .sites
            .iter()
            .find(|session| session.site == call.site)
        {
            guard(&session.state).notice = notice.or_else(|| Some(String::new()));
        }
        call.sessions.publish();
        NetError::HostPaused {
            host,
            until_unix: call
                .sessions
                .transport
                .paused_until(host)
                .unwrap_or_else(|| chrono::Utc::now().timestamp() + 5 * 60),
        }
    }

    /// An attempt over `jar`, outside any session, for transport tests.
    #[cfg(test)]
    pub(crate) fn detached(sessions: &super::SchoolSessions, site: Site, jar: Jar) -> Self {
        let attempt = Self::new(Arc::new(Call {
            site,
            sessions: sessions.0.clone(),
            deadline: Instant::now() + std::time::Duration::from_secs(600),
            account: 0,
            cancel: CancellationToken::new(),
            signed_in: AtomicBool::new(false),
            stopped: Mutex::new(None),
        }));
        let _ = attempt.0.instance.set(Arc::new(Instance {
            id: 0,
            identity: 0,
            jar: Mutex::new(jar),
        }));
        attempt
    }
}

/// How a verification's SSO ended.
pub(super) enum Walked {
    /// It reached the site: the snapshot with what the school set on the way.
    Ready(Snapshot),
    /// It reached the identity provider's sign-in page.
    SignIn,
    Failed,
}

enum Sso {
    Ready,
    /// The SSO reached the identity provider's sign-in page.
    SignIn,
    Failed(NetError),
}

impl Inner {
    fn call(self: &Arc<Self>, site: Site, permit: &Permit, deadline: Instant) -> Arc<Call> {
        Arc::new(Call {
            site,
            sessions: self.clone(),
            deadline,
            account: permit.generation(),
            cancel: permit.cancel().clone(),
            signed_in: AtomicBool::new(false),
            stopped: Mutex::new(None),
        })
    }

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
        let call = self.call(site, permit, deadline);
        let _serial = match &session.serial {
            Some(serial) => Some(bounded(&call.cancel, deadline, serial.lock()).await?),
            None => None,
        };
        let mut recovered = false;
        loop {
            let attempt = Attempt::new(call.clone());
            let result =
                bounded(&call.cancel, deadline, operation.attempt(attempt.clone())).await?;
            // The instance could not be had: that ends the call, whatever
            // the operation made of it.
            if let Some(stop) = call.stopped() {
                return Err(stop.into());
            }
            match result {
                Ok(value) => {
                    self.recovered(site);
                    return Ok(value);
                }
                Err(error) if error.signed_out() => {
                    let instance = attempt.0.instance.get();
                    if let Some(instance) = instance {
                        session.retire(instance);
                    }
                    if recovered || call.signed_in.load(Ordering::SeqCst) {
                        log::warn!(target: "app_core::school_session", "{site:?} refused a recovered session");
                        let identity = &self.account(profile).identity;
                        identity.anomaly(
                            instance.map_or_else(
                                || identity.generation(),
                                |instance| instance.identity,
                            ),
                        );
                        self.publish();
                        return Err(RunError::Blocked(self.paused(profile)));
                    }
                    log::info!(target: "app_core::school_session", "{site:?} session expired; rebuilding it");
                    recovered = true;
                }
                Err(error) => return Err(RunError::Op(error)),
            }
        }
    }

    /// An attempt for a write: nothing has been sent yet, and the site's
    /// session is got only if a request needs it. Never recovered or run
    /// again.
    #[cfg(feature = "agent")]
    pub(super) async fn write_attempt(
        self: &Arc<Self>,
        site: Site,
        permit: &Permit,
        deadline: Instant,
    ) -> Attempt {
        self.restore(site.profile()).await;
        Attempt::new(self.call(site, permit, deadline))
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

    /// The site's ready instance for `call`: the current one, or a new one
    /// built from the current snapshot and taken through SSO, signing in
    /// once per call if the SSO reaches the sign-in page.
    async fn ready(self: &Arc<Self>, call: &Call) -> Result<Arc<Instance>, Stop> {
        let site = call.site;
        let profile = site.profile();
        let session = self.site(site);
        let (cancel, deadline) = (&call.cancel, call.deadline);
        loop {
            if let Some(instance) = session.current() {
                return Ok(instance);
            }
            let _sso = bounded(cancel, deadline, session.sso.lock()).await?;
            // Another call finished the SSO while this one waited.
            if let Some(instance) = session.current() {
                return Ok(instance);
            }
            let identity = &self.account(profile).identity;
            let Some((snapshot, generation)) = identity.snapshot() else {
                if call.signed_in.swap(true, Ordering::SeqCst) {
                    identity.anomaly(identity.generation());
                    self.publish();
                    return Err(Stop::Blocked(self.paused(profile)));
                }
                let seen = identity.generation();
                bounded(
                    cancel,
                    deadline,
                    self.sign_in(profile, seen, super::identity::How::Background),
                )
                .await?
                .map_err(|ended| self.blocked(profile, ended))?;
                continue;
            };
            let instance = session.build(snapshot.jar(), generation);
            let outcome = bounded(cancel, deadline, self.sso(site, &instance)).await?;
            match outcome {
                Sso::Ready => {
                    log::info!(target: "app_core::school_session", "{site:?} session ready instance={}", instance.id);
                    session.install(instance.clone());
                    self.persist(profile).await;
                    return Ok(instance);
                }
                Sso::SignIn => {
                    if call.signed_in.swap(true, Ordering::SeqCst) {
                        log::warn!(target: "app_core::school_session", "{site:?} sent a fresh sign-in back to the sign-in page");
                        identity.anomaly(generation);
                        self.publish();
                        return Err(Stop::Blocked(self.paused(profile)));
                    }
                    identity.invalidate(generation);
                    bounded(
                        cancel,
                        deadline,
                        self.sign_in(profile, generation, super::identity::How::Background),
                    )
                    .await?
                    .map_err(|ended| self.blocked(profile, ended))?;
                }
                Sso::Failed(error) => return Err(Stop::Net(error)),
            }
        }
    }

    /// What lets `profile`'s reads go on after a sign-in ended without an
    /// identity.
    fn blocked(&self, profile: SchoolAuthProfile, ended: Ended) -> Stop {
        match ended {
            Ended::NotConfigured | Ended::Refused(_) => Stop::Blocked(AccountWait::Person),
            Ended::Offline => Stop::Blocked(AccountWait::Network),
            Ended::Unavailable(_) => Stop::Blocked(self.paused(profile)),
            Ended::Stopped => Stop::Cancelled,
        }
    }

    /// The end of `profile`'s pause, as a wait.
    fn paused(&self, profile: SchoolAuthProfile) -> AccountWait {
        let left = self
            .account(profile)
            .identity
            .pause_until()
            .map_or(RETRY_NOT_BEFORE, |until| {
                until.saturating_duration_since(Instant::now())
            });
        AccountWait::Paused {
            until: chrono::Utc::now() + chrono::Duration::from_std(left).unwrap_or_default(),
        }
    }

    /// One SSO of `profile`'s account from a new jar holding `snapshot`
    /// (the LMS for Science Tokyo, 教務Web for Tokyo Tech), keeping the
    /// identity cookies the school sets on the way: a verification
    /// (`identity::verify`). Nothing of it is installed as a site instance.
    pub(super) async fn check_sso(
        &self,
        profile: SchoolAuthProfile,
        snapshot: &Snapshot,
    ) -> Walked {
        let jar = Mutex::new(snapshot.jar());
        let captured = Mutex::new(snapshot.cookies.clone());
        let hosts: &[Host] = match profile {
            SchoolAuthProfile::Science => &[Host::ScienceAccount],
            SchoolAuthProfile::TokyoTech => &[Host::GetAccess, Host::GetAccessRouter],
        };
        let walk = Walk::new(self.transport.as_ref(), &jar).capturing(hosts, &captured);
        let ended = match profile {
            SchoolAuthProfile::Science => lms_sso(&walk).await,
            SchoolAuthProfile::TokyoTech => kyomu_sso(&walk, false).await,
        };
        match ended {
            Ok(None) => {
                let lines = captured
                    .into_inner()
                    .unwrap_or_else(PoisonError::into_inner);
                Walked::Ready(Snapshot::from_responses(lines, chrono::Utc::now()))
            }
            Err(SsoError::SignIn) => Walked::SignIn,
            // A maintenance notice, the network, an unknown page: no verdict.
            Ok(Some(_)) | Err(SsoError::Walk(_)) => Walked::Failed,
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
async fn bounded<T>(
    cancel: &CancellationToken,
    deadline: Instant,
    work: impl std::future::Future<Output = T>,
) -> Result<T, Stop> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(Stop::Cancelled),
        done = tokio::time::timeout_at(deadline, work) => done.map_err(|_| Stop::Deadline),
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
