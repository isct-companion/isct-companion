//! School sign-in and sessions, owned by Core.
//!
//! Two kinds of session. Each school account has one identity: Core signs in
//! to its identity provider over HTTP (Extic for Science Tokyo, GetAccess for
//! Tokyo Tech) and keeps the identity cookies the provider set, the
//! snapshot. Core never writes into a snapshot after the sign-in that made
//! it. Each website (the LMS, 教務Web in Japanese, 教務Web in English) has a
//! site session: an instance is a jar that starts as a copy of the current
//! snapshot and then takes every cookie the school sets, and SSO is the
//! site's own business. A request carries the instance it set out with, and
//! its answer is written back to that instance only, so a late answer can
//! never reach the jar a rebuild put in its place.
//!
//! Reads hand Core an operation (`site::Operation`), never a request: one
//! `run` rebuilds the requests on each attempt, recovers the session at most
//! once and runs the operation again at most once. Writes never run again.
//!
//! The web view gets an account's identity cookies, one way, before an
//! App-initiated navigation (`seed`); Core never reads its cookies, and a
//! session someone signs in to by hand on a web page stays the web page's.
//!
//! Every change of an account goes through its owner (`owner`): permits
//! stamped with the account generation, and the barrier that switching
//! accounts, signing out and resetting run.

mod identity;
mod jar;
mod login;
mod owner;
mod site;
mod store;
mod walk;
mod web;

pub(crate) use owner::Permit;
pub(crate) use site::{Attempt, OpError, Operation, RunError, Site};
pub use web::{WebCookie, WebCookieSeed};

use self::{
    identity::Identity, jar::Jar, owner::Owner, site::SiteSession, store::Saved, web::WebQueue,
};
use crate::{
    events::CoreEvent,
    net::{BoxFuture, Host, Http, Limits, NetError, NetRequest, Response},
    ports::EventSink,
    runtime_tasks::RuntimeTasks,
    school_auth::{
        CredentialAvailability, SchoolAuthProfile, SchoolAuthService, SchoolCredentials,
    },
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
    },
    time::Duration,
};
use url::Url;

/// One account sign-in, waiting for the one-time password window included.
/// A sign-in does not stop when the caller that started it stops waiting,
/// so it needs its own bound.
pub const SIGN_IN_DEADLINE: Duration = Duration::from_secs(60);
/// A one-time password window with less left than this is skipped for the
/// next one (Extic's clock runs a little ahead).
pub const OTP_MARGIN: Duration = Duration::from_secs(8);
/// After a network failure, the earliest the next sign-in may start.
pub const RETRY_NOT_BEFORE: Duration = Duration::from_secs(60);
/// The longest an App-initiated navigation waits for the shell to write the
/// identity cookies into the web view.
pub const SEED_WAIT: Duration = Duration::from_secs(10);

// A sync key's deadline (from the call's start) covers one sign-in and two
// page reads; every other limit comes from the caller's deadline.
const _: () = assert!(
    SIGN_IN_DEADLINE.as_secs() + 2 * Limits::page().deadline.as_secs()
        <= crate::sync_engine::KEY_DEADLINE.as_secs()
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSessionState {
    /// No credentials are saved.
    NotConfigured,
    /// Credentials, but no identity yet: the next read signs in.
    Idle,
    SigningIn,
    /// An identity snapshot is held.
    Active,
    /// The school turned these credentials down; waits for new ones or a
    /// retry.
    Rejected,
    /// The school wants a person on its own page (an emailed code, a new
    /// password, a page Core does not know).
    NeedsWeb,
    /// No verdict (network, timeout, server error, anomaly); Core tries
    /// again after `retry_at`.
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSignInProblemKind {
    InvalidCredentials,
    InvalidOtp,
    InvalidMatrix,
    EmailOtp,
    PasswordChange,
    UnknownPage,
    Network,
    /// The password went out and no answer came: counted as submitted.
    Unanswered,
    /// A fresh sign-in was sent straight back to the sign-in page.
    Anomaly,
}

impl SchoolSignInProblemKind {
    fn state(self) -> SchoolSessionState {
        match self {
            Self::InvalidCredentials | Self::InvalidOtp | Self::InvalidMatrix => {
                SchoolSessionState::Rejected
            }
            Self::EmailOtp | Self::PasswordChange | Self::UnknownPage => {
                SchoolSessionState::NeedsWeb
            }
            Self::Network | Self::Unanswered | Self::Anomaly => SchoolSessionState::Unavailable,
        }
    }
}

/// Why an account is not signed in. Kept with the credentials it is about
/// (`school_auth`'s candidate record), so a relaunch never sends a refused
/// password again.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSignInProblem {
    pub kind: SchoolSignInProblemKind,
    /// Core's short diagnostic, for logs; shells show text for `kind`.
    pub message: String,
}

/// A site the school paused for maintenance, while the pause lasts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolMaintenance {
    pub notice: Option<String>,
    pub until: DateTime<Utc>,
}

/// One account's session as shells render it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolProfileSession {
    pub state: SchoolSessionState,
    pub problem: Option<SchoolSignInProblem>,
    /// When Core tries again on its own (`unavailable` only).
    pub retry_at: Option<DateTime<Utc>>,
    /// The account's site (LMS, 教務Web) is paused for maintenance.
    pub maintenance: Option<SchoolMaintenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionProfiles {
    pub science: SchoolProfileSession,
    pub tokyo_tech: SchoolProfileSession,
}

/// Sent as `school-session` whenever it changes; a shell keeps the higher
/// revision of this and what it read.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolSessionSnapshot {
    pub revision: u64,
    pub profiles: SchoolSessionProfiles,
}

impl SchoolSessionSnapshot {
    pub fn profile(&self, profile: SchoolAuthProfile) -> &SchoolProfileSession {
        match profile {
            SchoolAuthProfile::Science => &self.profiles.science,
            SchoolAuthProfile::TokyoTech => &self.profiles.tokyo_tech,
        }
    }
}

/// What `Core::connect_school` ended with. `verified` says the school
/// accepted exactly these credentials, so they now sync to other devices.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchoolConnectResult {
    pub state: SchoolSessionState,
    pub problem: Option<SchoolSignInProblem>,
    pub verified: bool,
}

/// The account cannot reach its school now, and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Unavailable {
    pub(crate) profile: SchoolAuthProfile,
    pub(crate) state: SchoolSessionState,
}

/// The identity cookies of one sign-in, as the providers sent them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Snapshot {
    pub(crate) cookies: Vec<WebCookie>,
}

const HTTP_DATE: &str = "%a, %d %b %Y %H:%M:%S GMT";

impl Snapshot {
    /// The last value of each cookie the identity hosts set, in the order
    /// they set them, leaving out the deleted ones. A lifetime becomes an
    /// absolute `Expires`, so writing it later never extends it.
    pub(crate) fn from_responses(lines: Vec<WebCookie>, now: DateTime<Utc>) -> Self {
        let mut seen = HashSet::new();
        let mut kept = Vec::new();
        for cookie in lines.into_iter().rev() {
            let Ok(url) = Url::parse(&cookie.url) else {
                continue;
            };
            let Ok(parsed) = cookie_store::Cookie::parse(cookie.set_cookie.as_str(), &url) else {
                continue;
            };
            let key = (
                parsed.name().to_owned(),
                format!("{:?}", parsed.domain),
                String::from(&parsed.path),
            );
            if !seen.insert(key) || parsed.is_expired() || parsed.value().is_empty() {
                continue;
            }
            kept.push(WebCookie {
                set_cookie: absolute_expiry(&cookie.set_cookie, now),
                url: cookie.url,
            });
        }
        kept.reverse();
        Self { cookies: kept }
    }

    /// A new jar holding exactly these cookies.
    pub(crate) fn jar(&self) -> Jar {
        let mut jar = Jar::default();
        for cookie in &self.cookies {
            if let Ok(url) = Url::parse(&cookie.url) {
                jar.store(&url, std::slice::from_ref(&cookie.set_cookie));
            }
        }
        jar
    }
}

/// `line` with `Max-Age` or `Expires` replaced by one absolute `Expires`.
fn absolute_expiry(line: &str, now: DateTime<Utc>) -> String {
    let mut attributes = line.split(';');
    let mut kept = vec![attributes.next().unwrap_or("").trim().to_owned()];
    let (mut max_age, mut expires) = (None, None);
    for attribute in attributes {
        let (name, value) = attribute
            .split_once('=')
            .map(|(name, value)| (name.trim(), value.trim()))
            .unwrap_or((attribute.trim(), ""));
        match name.to_ascii_lowercase().as_str() {
            "max-age" => max_age = value.parse::<i64>().ok(),
            "expires" => expires = Some(value.to_owned()),
            "" => {}
            _ => kept.push(attribute.trim().to_owned()),
        }
    }
    let until = match (max_age, expires) {
        (Some(seconds), _) => Some(
            (now + chrono::Duration::seconds(seconds))
                .format(HTTP_DATE)
                .to_string(),
        ),
        (None, expires) => expires,
    };
    if let Some(until) = until {
        kept.push(format!("Expires={until}"));
    }
    kept.join("; ")
}

/// How Core reaches the school: `net::Http` in the app, a scripted school
/// in the session tests.
pub(crate) trait Transport: Send + Sync {
    fn send(&self, request: NetRequest) -> BoxFuture<'_, Result<Response, NetError>>;
    /// A maintenance page: pause the host now.
    fn report_maintenance(&self, host: Host);
    /// The end of the host's current pause, in Unix seconds.
    fn paused_until(&self, host: Host) -> Option<i64>;
}

impl Transport for Http {
    fn send(&self, request: NetRequest) -> BoxFuture<'_, Result<Response, NetError>> {
        Box::pin(Http::send(self, request))
    }

    fn report_maintenance(&self, host: Host) {
        Http::report_maintenance(self, host);
    }

    fn paused_until(&self, host: Host) -> Option<i64> {
        Http::paused_until(self, host)
    }
}

/// A transport that never reaches the school, for a Core built without one.
struct Offline;

impl Transport for Offline {
    fn send(&self, _request: NetRequest) -> BoxFuture<'_, Result<Response, NetError>> {
        Box::pin(async { Err(NetError::Offline) })
    }

    fn report_maintenance(&self, _host: Host) {}

    fn paused_until(&self, _host: Host) -> Option<i64> {
        None
    }
}

/// One school account: its owner, identity and sites.
struct Account {
    profile: SchoolAuthProfile,
    owner: Owner,
    identity: Identity,
    sites: Vec<SiteSession>,
    /// The vault item, read once, at the account's first use.
    restored: tokio::sync::OnceCell<()>,
}

struct Inner {
    transport: Arc<dyn Transport>,
    auth: Arc<SchoolAuthService>,
    events: OnceLock<Arc<dyn EventSink>>,
    accounts: [Account; 2],
    web: WebQueue,
    tasks: RuntimeTasks,
    revision: AtomicU64,
    published: Mutex<Option<SchoolSessionProfiles>>,
    /// The shell reports no network: a sign-in sends nothing.
    offline: AtomicBool,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The school sessions of both accounts. Cheap to clone.
#[derive(Clone)]
pub struct SchoolSessions(Arc<Inner>);

/// What writing an account's identity into the web view came to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Seeded {
    /// The shell committed the cookies of this identity generation.
    Written(u64),
    /// Core holds no identity for the account: nothing to write.
    Nothing,
    /// The write failed or took longer than `SEED_WAIT`.
    Failed,
    /// The shell has no web cookie port.
    Unsupported,
}

impl SchoolSessions {
    /// Sessions over `http`; without one (it could not be built) every
    /// request fails as offline.
    pub fn new(http: Option<Http>, auth: Arc<SchoolAuthService>) -> Self {
        let transport: Arc<dyn Transport> = match http {
            Some(http) => Arc::new(http),
            None => Arc::new(Offline),
        };
        Self::with_transport(transport, auth)
    }

    pub(crate) fn with_transport(
        transport: Arc<dyn Transport>,
        auth: Arc<SchoolAuthService>,
    ) -> Self {
        let configured = configured_profiles(&auth);
        let account = |profile: SchoolAuthProfile, sites: Vec<SiteSession>| Account {
            profile,
            owner: Owner::new(),
            identity: Identity::new(configured.contains(&profile)),
            sites,
            restored: tokio::sync::OnceCell::new(),
        };
        Self(Arc::new(Inner {
            transport,
            auth,
            events: OnceLock::new(),
            accounts: [
                account(
                    SchoolAuthProfile::Science,
                    vec![SiteSession::new(Site::Lms)],
                ),
                account(
                    SchoolAuthProfile::TokyoTech,
                    vec![
                        SiteSession::new(Site::Kyomu),
                        SiteSession::new(Site::KyomuEnglish),
                    ],
                ),
            ],
            web: WebQueue::default(),
            tasks: RuntimeTasks::default(),
            revision: AtomicU64::new(1),
            published: Mutex::new(None),
            offline: AtomicBool::new(false),
        }))
    }

    /// Sessions whose every site already holds an instance with an empty
    /// jar, for tests of what runs on a session.
    #[cfg(test)]
    pub(crate) fn ready(auth: Arc<SchoolAuthService>) -> Self {
        let sessions = Self::new(None, auth);
        for account in &sessions.0.accounts {
            for site in &account.sites {
                site.install_restored(Jar::default(), 0);
            }
        }
        sessions
    }

    /// Where `school-session` goes. Set once, by the facade.
    pub(crate) fn set_events(&self, events: Arc<dyn EventSink>) {
        let _ = self.0.events.set(events);
    }

    /// The shell's write-only web cookie port. Set once.
    pub fn attach_web(&self, seed: Arc<dyn WebCookieSeed>) -> bool {
        self.0.web.attach(seed)
    }

    /// Whether the shell gave Core its web cookie port.
    pub(crate) fn seeds_web(&self) -> bool {
        self.0.web.attached()
    }

    /// Both accounts as shells render them.
    pub fn snapshot(&self) -> SchoolSessionSnapshot {
        SchoolSessionSnapshot {
            revision: self.0.revision.load(Ordering::SeqCst),
            profiles: self.0.profiles(),
        }
    }

    /// Signs in with `credentials`, the person's own request. A refusal on
    /// record does not hold it back: entering credentials is the retry.
    pub async fn connect(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> SchoolConnectResult {
        let inner = &self.0;
        inner.restore(profile).await;
        // Another account under the same profile: the old one's session goes
        // first, behind the barrier.
        let switched = inner
            .stored_account(profile)
            .await
            .is_some_and(|account| account != credentials.username);
        if switched {
            inner.barrier(profile, true).await;
        }
        let account = inner.account(profile);
        account.identity.configured(true);
        let generation = account.identity.generation();
        let signed_in = inner
            .sign_in(
                profile,
                generation,
                identity::How::Connect(credentials.clone()),
            )
            .await;
        let session = inner.profile(profile);
        let verified = signed_in.is_ok() && {
            let auth = inner.auth.clone();
            tokio::task::spawn_blocking(move || auth.is_verified(profile, &credentials))
                .await
                .is_ok_and(|verified| verified.unwrap_or(false))
        };
        SchoolConnectResult {
            state: session.state,
            problem: session.problem,
            verified,
        }
    }

    /// One foreground attempt with the credentials saved now, through a
    /// refusal on record. Saved credentials are read again first, so a
    /// password changed on another device is found here.
    pub async fn retry(&self, profile: SchoolAuthProfile) -> SchoolSessionSnapshot {
        let inner = &self.0;
        inner.auth.refresh();
        inner.restore(profile).await;
        let account = inner.account(profile);
        account
            .identity
            .configured(configured_profiles(&inner.auth).contains(&profile));
        let generation = account.identity.generation();
        let _ = inner
            .sign_in(profile, generation, identity::How::Retry)
            .await;
        self.snapshot()
    }

    /// Saved credentials changed outside a connect (imported, synced,
    /// deleted). A refusal stays only while it is about the credentials
    /// saved now: the next sign-in reads them again.
    pub fn credentials_changed(&self, profile: SchoolAuthProfile) {
        self.0.auth.refresh();
        let configured = configured_profiles(&self.0.auth).contains(&profile);
        self.0.account(profile).identity.lift(configured);
        self.0.publish();
    }

    /// Signs the account out of this device: its sign-in stops, its reads
    /// end, Core's cookies and the LMS token go, and so do the school
    /// cookies of the web view.
    pub async fn sign_out(&self, profile: SchoolAuthProfile) {
        self.0.barrier(profile, true).await;
        self.0.account(profile).identity.configured(false);
        self.0.publish();
    }

    /// Clears the sessions of both accounts (a reset), the web view's school
    /// cookies too when `web`. Saved credentials stay.
    pub async fn reset(&self, web: bool) {
        // Both accounts stay closed until both are clean: one reopened early
        // would sign in while the other is still being cleared.
        let mut closed = Vec::new();
        for account in &self.0.accounts {
            closed.push(self.0.close(account.profile, web).await);
        }
        for account in &self.0.accounts {
            let configured = configured_profiles(&self.0.auth).contains(&account.profile);
            account.identity.configured(configured);
        }
        drop(closed);
        log::info!(target: "app_core::school_session", "sessions reset");
        self.0.publish();
    }

    /// The shell's periodic wake: a pause after a failure that has run out
    /// lifts, and the planner sees it.
    pub fn wake(&self) {
        let mut changed = false;
        for account in &self.0.accounts {
            changed |= account.identity.expire_pause(tokio::time::Instant::now());
        }
        if changed {
            self.0.publish();
        }
    }

    /// The network the shell reports (`Hint::Network`). While it is
    /// offline a sign-in ends as unavailable without sending anything.
    pub(crate) fn network(&self, network: crate::sync_engine::Network) {
        self.0.offline.store(
            network == crate::sync_engine::Network::Offline,
            Ordering::SeqCst,
        );
    }

    /// The system took the background time back: sign-ins stop.
    pub fn suspend(&self) {
        for account in &self.0.accounts {
            account.identity.cancel_flight();
        }
    }

    /// A permit for one operation on `profile`'s account.
    pub(crate) fn permit(&self, profile: SchoolAuthProfile) -> Option<Permit> {
        self.0.account(profile).owner.permit()
    }

    /// Reads with `operation` on `site` by `deadline`, counted from the
    /// call's start (`site::run`).
    pub(crate) async fn run<O: Operation>(
        &self,
        site: Site,
        permit: &Permit,
        deadline: tokio::time::Instant,
        operation: &mut O,
    ) -> Result<O::Output, RunError<O::Error>> {
        self.0.run(site, permit, deadline, operation).await
    }

    /// An attempt for a write: the site's session made ready (signing in if
    /// needed, nothing has been sent yet), never recovered or run again.
    #[cfg(feature = "agent")]
    pub(crate) async fn write_attempt(
        &self,
        site: Site,
        permit: &Permit,
        deadline: tokio::time::Instant,
    ) -> Result<Attempt, RunError<NetError>> {
        self.0.write_attempt(site, permit, deadline).await
    }

    /// Writes `profile`'s identity into the web view before an App-initiated
    /// navigation, waiting at most `SEED_WAIT`.
    pub(crate) async fn seed(&self, profile: SchoolAuthProfile) -> Seeded {
        self.0.seed(profile).await
    }

    /// A web page the App opened landed on the identity provider's sign-in
    /// page after `seeded` was written: signs in over HTTP (joining any
    /// sign-in already running) and writes the new identity. `true` when the
    /// web view now holds a newer identity than the one that failed.
    pub(crate) async fn web_sign_in(&self, profile: SchoolAuthProfile, seeded: u64) -> bool {
        self.0.web_sign_in(profile, seeded).await
    }

    /// The account generation of `profile`, which a web page's recovery
    /// checks did not move.
    pub(crate) fn account_generation(&self, profile: SchoolAuthProfile) -> u64 {
        self.0.account(profile).owner.generation()
    }

    /// Ends every sign-in and keeps the sites' jars for the next launch.
    pub async fn shutdown(&self) {
        self.suspend();
        for account in &self.0.accounts {
            self.0.persist(account.profile).await;
        }
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }
}

/// The profiles whose credentials the status cache says are saved. Only
/// metadata: reading it never unlocks the vault.
fn configured_profiles(auth: &SchoolAuthService) -> HashSet<SchoolAuthProfile> {
    let Ok(status) = auth.status() else {
        return HashSet::new();
    };
    [
        (SchoolAuthProfile::Science, status.science),
        (SchoolAuthProfile::TokyoTech, status.tokyo_tech),
    ]
    .into_iter()
    .filter(|(_, summary)| {
        summary.saved
            && !matches!(
                summary.availability,
                CredentialAvailability::NotConfigured | CredentialAvailability::Missing
            )
    })
    .map(|(profile, _)| profile)
    .collect()
}

impl Inner {
    fn account(&self, profile: SchoolAuthProfile) -> &Account {
        match profile {
            SchoolAuthProfile::Science => &self.accounts[0],
            SchoolAuthProfile::TokyoTech => &self.accounts[1],
        }
    }

    fn site(&self, site: Site) -> &SiteSession {
        let account = self.account(site.profile());
        account
            .sites
            .iter()
            .find(|session| session.site == site)
            .unwrap_or(&account.sites[0])
    }

    fn profile(&self, profile: SchoolAuthProfile) -> SchoolProfileSession {
        let account = self.account(profile);
        let (state, problem, retry_at) = account.identity.view();
        SchoolProfileSession {
            state,
            problem,
            retry_at,
            maintenance: self.maintenance(account),
        }
    }

    fn profiles(&self) -> SchoolSessionProfiles {
        SchoolSessionProfiles {
            science: self.profile(SchoolAuthProfile::Science),
            tokyo_tech: self.profile(SchoolAuthProfile::TokyoTech),
        }
    }

    /// The account's site paused for maintenance: by the school's notice, or
    /// by the host's breaker after a maintenance answer.
    fn maintenance(&self, account: &Account) -> Option<SchoolMaintenance> {
        let host = account.sites.first()?.site.host();
        let until = self.transport.paused_until(host)?;
        let notice = account
            .sites
            .iter()
            .find_map(|site| guard(&site.state).notice.clone());
        notice.as_ref()?;
        Some(SchoolMaintenance {
            notice,
            until: DateTime::from_timestamp(until, 0).unwrap_or_else(Utc::now),
        })
    }

    /// Sends `school-session` when what shells render changed.
    fn publish(&self) {
        let profiles = self.profiles();
        {
            let mut published = guard(&self.published);
            if published.as_ref() == Some(&profiles) {
                return;
            }
            *published = Some(profiles.clone());
        }
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(events) = self.events.get() {
            let _ = events.emit(CoreEvent::SchoolSession(SchoolSessionSnapshot {
                revision,
                profiles,
            }));
        }
    }

    /// Reads the account's vault item once: its identity and the sites'
    /// jars, if it belongs to the account the credentials name now.
    async fn restore(&self, profile: SchoolAuthProfile) {
        let account = self.account(profile);
        account
            .restored
            .get_or_init(|| async {
                let auth = self.auth.clone();
                let loaded = tokio::task::spawn_blocking(move || {
                    let username = auth.status().ok().map(|status| match profile {
                        SchoolAuthProfile::Science => status.science.username,
                        SchoolAuthProfile::TokyoTech => status.tokyo_tech.username,
                    });
                    let saved = auth.load_session(profile).ok().flatten()?;
                    let saved = Saved::decode(&saved)?;
                    (username.as_deref() == Some(saved.account.as_str())).then_some(saved)
                })
                .await
                .ok()
                .flatten();
                let Some(saved) = loaded else { return };
                let snapshot = Snapshot {
                    cookies: saved.identity.clone(),
                };
                let Some(generation) = account.identity.restore(snapshot, saved.account.clone())
                else {
                    return;
                };
                for site in &account.sites {
                    if let Some(jar) = saved.site(site.site).and_then(Jar::load) {
                        site.install_restored(jar, generation);
                    }
                }
                log::info!(target: "app_core::school_session", "restored the saved session profile={profile:?}");
            })
            .await;
        self.publish();
    }

    /// The username the account's session belongs to, read back if needed.
    async fn stored_account(&self, profile: SchoolAuthProfile) -> Option<String> {
        self.restore(profile).await;
        self.account(profile).identity.account()
    }

    /// Saves the identity and every site's jar of the account.
    async fn persist(&self, profile: SchoolAuthProfile) {
        let account = self.account(profile);
        let Some((snapshot, username)) = account.identity.persisted() else {
            return;
        };
        let mut saved = Saved::new(username, snapshot.cookies.clone());
        for site in &account.sites {
            saved = saved.with_site(site.site, site.saved_jar());
        }
        let Some(text) = saved.encode() else { return };
        let auth = self.auth.clone();
        let stored = tokio::task::spawn_blocking(move || auth.store_session(profile, &text)).await;
        if let Ok(Err(error)) = stored {
            log::warn!(target: "app_core::school_session", "session not saved profile={profile:?}: {error}");
        }
    }

    /// The barrier: stop, cancel and drain the account's permits, clean up
    /// in order (sign-in, sites, LMS token, vault item, web cookies), then
    /// open the next generation.
    async fn barrier(&self, profile: SchoolAuthProfile, web: bool) {
        let closed = self.close(profile, web).await;
        drop(closed);
        log::info!(target: "app_core::school_session", "account barrier done profile={profile:?}");
        self.publish();
    }

    /// Closes the account and clears its session: its sign-in stops, its
    /// reads drain, Core's cookies, the LMS token and (with `web`) the web
    /// view's school cookies go. It reopens, one generation on, when the
    /// guard drops.
    async fn close(&self, profile: SchoolAuthProfile, web: bool) -> owner::Closed<'_> {
        let account = self.account(profile);
        let closed = account.owner.close().await;
        account.identity.cancel_flight();
        account.identity.wait_flight().await;
        account.identity.forget();
        for site in &account.sites {
            site.forget();
        }
        let auth = self.auth.clone();
        let deleted = tokio::task::spawn_blocking(move || {
            let token = auth.delete_moodle_token(profile);
            let session = auth.delete_session(profile);
            token.and(session)
        })
        .await;
        if let Ok(Err(error)) = deleted {
            log::warn!(target: "app_core::school_session", "session not deleted profile={profile:?}: {error}");
        }
        if web {
            if let Err(error) = self.web.clear(&self.tasks).await {
                log::warn!(target: "app_core::school_session", "web cookies not cleared: {error}");
            }
        }
        closed
    }

    async fn seed(&self, profile: SchoolAuthProfile) -> Seeded {
        if !self.web.attached() {
            return Seeded::Unsupported;
        }
        self.restore(profile).await;
        let account = self.account(profile);
        let Some(_permit) = account.owner.permit() else {
            return Seeded::Failed;
        };
        let Some((snapshot, generation)) = account.identity.snapshot() else {
            return Seeded::Nothing;
        };
        let wait = self.web.write(&self.tasks, snapshot.cookies.clone());
        match tokio::time::timeout(SEED_WAIT, wait).await {
            Ok(Ok(Ok(()))) => Seeded::Written(generation),
            Ok(Ok(Err(error))) => {
                log::warn!(target: "app_core::school_session", "web cookies not written profile={profile:?}: {error}");
                Seeded::Failed
            }
            Ok(Err(_)) | Err(_) => {
                log::warn!(target: "app_core::school_session", "web cookies not written in time profile={profile:?}");
                Seeded::Failed
            }
        }
    }

    async fn web_sign_in(self: &Arc<Self>, profile: SchoolAuthProfile, seeded: u64) -> bool {
        if self
            .sign_in(profile, seeded, identity::How::Background)
            .await
            .is_err()
        {
            return false;
        }
        matches!(self.seed(profile).await, Seeded::Written(generation) if generation > seeded)
    }
}

#[cfg(test)]
#[path = "../tests/school_session.rs"]
mod tests;
