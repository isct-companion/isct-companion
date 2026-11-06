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
//! The web view gets an account's identity cookies, one way, from a web
//! operation (`seed`, the one exit): only a verified generation, only while
//! the account is usable. Core never reads them back, and a session someone
//! signs in to by hand on a web page stays the web page's. Signing either account out clears every school website of the
//! web view, both accounts' alike, in a fixed order (`clear_web`): the
//! account still signed in has its identity written again by its next web
//! operation, which opens a new view.
//!
//! Every change of an account goes through its owner (`owner`): permits
//! stamped with the account generation, and the barrier that signing out
//! and resetting run, which drains every operation of the account before it
//! deletes anything.
//!
//! Sync reads only what these sessions hold (`account_facts`, the LMS
//! token's `MoodleClient::token_fact`) and is told when any of it changes
//! (`CredentialSignal`); it keeps none of it itself.

mod identity;
mod jar;
mod login;
mod owner;
mod site;
mod store;
mod vpn;
mod walk;
mod web;

pub(crate) use identity::{Ended, SignInAdmission};
pub(crate) use owner::Permit;
pub(crate) use site::{Attempt, Blocked, OpError, Operation, RunError, Site};
pub use vpn::{CampusVpnError, CampusVpnLogin, APM_ENTRY};
pub use web::{is_school_domain, WebCookie, WebCookieSeed};

use self::{
    identity::{Checked, Identity},
    jar::Jar,
    owner::Owner,
    site::SiteSession,
    store::Saved,
    web::{WebError, WebQueue},
};
use crate::{
    events::CoreEvent,
    net::{BoxFuture, Host, Http, Limits, NetError, NetRequest, Response},
    ports::EventSink,
    runtime_tasks::RuntimeTasks,
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak,
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
/// The longest a web operation waits for the shell to write the identity
/// cookies into the web view. Only the caller stops waiting: the write stays
/// queued, and nothing behind it overtakes it.
pub const SEED_WAIT: Duration = Duration::from_secs(10);
/// A web cookie call the platform has not answered after this makes the
/// store unusable until it does: no write is admitted meanwhile. Within it,
/// shutdown waits for the answer.
pub const WEB_STORE_LIMIT: Duration = Duration::from_secs(10);
/// One verification of an identity, everything included.
pub const VERIFY_DEADLINE: Duration = Duration::from_secs(30);
/// A web operation uses an identity verified less long ago as it is.
pub const VERIFIED_FRESH: Duration = Duration::from_secs(5 * 60);
/// One web operation, everything included: getting the identity ready (a
/// verification, a sign-in), its writes and its page loads.
pub const WEB_OPERATION_DEADLINE: Duration = Duration::from_secs(150);

const _: () = assert!(SEED_WAIT.as_secs() <= WEB_STORE_LIMIT.as_secs());
const _: () = assert!(
    VERIFY_DEADLINE.as_secs() + SIGN_IN_DEADLINE.as_secs() + 2 * SEED_WAIT.as_secs() + 30
        <= WEB_OPERATION_DEADLINE.as_secs()
);

// A sync key's deadline (from the call's start) covers one sign-in and two
// page reads; every other limit comes from the caller's deadline.
const _: () = assert!(
    SIGN_IN_DEADLINE.as_secs() + 2 * Limits::page().deadline.as_secs()
        <= crate::plan::KEY_DEADLINE.as_secs()
);

/// One account as shells show it, derived from what Core keeps about it
/// (`identity::Identity::view`).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSessionState {
    /// No credentials are saved.
    NotConfigured,
    /// Nothing stands in the way: Core signs in when it needs to.
    Normal,
    SigningIn,
    /// The school refused the saved credentials: Core does not sign in with
    /// them on its own until they change or the person retries.
    Refused,
    /// The school wants the person on its own page (an emailed code, a new
    /// password, a page Core does not know).
    NeedsWeb,
    /// No verdict (network, an unanswered sign-in, an anomaly); Core tries
    /// again after `retry_at`.
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SchoolSignInProblemKind {
    InvalidCredentials,
    InvalidOtp,
    /// The school asks for the App one-time password and the Connect
    /// entered no OTP setup.
    OtpNeeded,
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
    /// The school wants the person on its own page, not other credentials.
    pub(crate) fn needs_web(self) -> bool {
        matches!(
            self,
            Self::EmailOtp | Self::PasswordChange | Self::UnknownPage
        )
    }
}

/// Why an account is not signed in. A refusal is kept with the credentials
/// it is about (`school_auth`'s refusal record), so a relaunch never sends
/// a refused password again.
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
    pub sign_out_pending: bool,
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

/// How `Core::connect_school` ended, for the Connect form. Only `Connected`
/// saved anything: the entered credentials are the verified ones now and
/// sync to the person's other devices. Every other outcome keeps the
/// form's input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SchoolConnectOutcome {
    Connected,
    /// The school refused these credentials: the ID or password, the
    /// one-time password or the Matrix (`reason`).
    Refused {
        reason: SchoolSignInProblemKind,
    },
    /// The school wants the person on its own page: an emailed code, a new
    /// password, a page Core does not know (`reason`).
    NeedsWeb {
        reason: SchoolSignInProblemKind,
    },
    /// No verdict: offline, the network, out of time.
    Unreachable,
    /// The school accepted the credentials, and the system credential vault
    /// did not save them: the saved credentials are as they were.
    NotSaved,
    /// A newer Connect, a sign-out or a reset stopped it.
    Stopped,
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

/// Moves whenever a credential fact a school read rests on changes: a site
/// instance installed or retired, the identity snapshot made, restored or
/// found invalid, a refusal recorded or cleared, a pause set or over, the
/// account admitted or not, the LMS token got, refused or found unable to
/// serve a function. Sync compares its `version` and is called on every
/// move; unlike `publish`, it never depends on what shells render.
#[derive(Default)]
pub(crate) struct CredentialSignal {
    version: AtomicU64,
    listener: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl CredentialSignal {
    pub(crate) fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// A fact changed. Callers hold none of the sessions' locks.
    pub(crate) fn changed(&self) {
        self.version.fetch_add(1, Ordering::SeqCst);
        let listener = guard(&self.listener).clone();
        if let Some(listener) = listener {
            listener();
        }
    }

    /// The one listener (the sync runner). Set once.
    pub(crate) fn listen(&self, listener: Arc<dyn Fn() + Send + Sync>) {
        *guard(&self.listener) = Some(listener);
    }
}

/// What one account's credentials allow now, as sync reads it. Before the
/// account's saved session is read back (`Inner::restore`), its jars and
/// identity count as present: unchecked, never as gone.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AccountFacts {
    /// Saved credentials, no sign-out pending, no barrier running: the
    /// admission every read of the account passes first
    /// (`Attempt::authorize`, `Owner::permit`).
    pub(crate) admitted: bool,
    /// An identity snapshot the school has not found invalid, and no
    /// anomaly pause running: a site's SSO can take a new instance from it
    /// without a password.
    pub(crate) identity: bool,
    /// Whether a sign-in on Core's own may start (`Inner::admission`).
    pub(crate) sign_in: SignInAdmission,
    /// The sites holding an instance.
    pub(crate) sites: Vec<Site>,
}

impl Default for AccountFacts {
    /// No sessions at all: nothing is admitted.
    fn default() -> Self {
        Self {
            admitted: false,
            identity: false,
            sign_in: SignInAdmission::Person,
            sites: Vec::new(),
        }
    }
}

/// How Core reaches the school: `net::Http`, or nothing (`Offline`).
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
    /// Order the identity verdict and all its vault writes as one commit.
    commit: tokio::sync::Mutex<()>,
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
    /// The web views a sign-out stops (the school browser engine).
    views: Mutex<Option<Weak<dyn AccountViews>>>,
    tasks: RuntimeTasks,
    revision: AtomicU64,
    published: Mutex<Option<SchoolSessionProfiles>>,
    /// The shell reports no network: a sign-in sends nothing.
    offline: AtomicBool,
    credentials: Arc<CredentialSignal>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The school sessions of both accounts. Cheap to clone.
#[derive(Clone)]
pub struct SchoolSessions(Arc<Inner>);

/// The web views of the accounts, as a sign-out stops them.
pub(crate) trait AccountViews: Send + Sync {
    /// All school views share the store, including public sites and popups.
    fn stop(&self);
}

/// Why a web operation could not get an account's identity into the web
/// view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WebStop {
    /// No verdict: offline, the network, maintenance, a pause after a
    /// failure, out of time. No password was sent for it.
    Unreachable,
    /// The school refused the saved credentials.
    Rejected,
    /// The school wants the person on its own page.
    NeedsWeb,
    NotConfigured,
    /// The web cookie store failed or is unusable.
    Storage,
    /// A sign-out or reset, or a newer identity, made it stale.
    Stale,
    /// The shell has no web cookie port.
    Unsupported,
}

impl WebStop {
    fn of(ended: &Ended) -> Self {
        match ended {
            Ended::Refused(problem) if problem.kind.needs_web() => Self::NeedsWeb,
            Ended::Refused(_) => Self::Rejected,
            Ended::NotConfigured => Self::NotConfigured,
            // Only a Connect ends so; a web operation's sign-in joins it as
            // the identity it made (`identity::Inner::sign_in`).
            Ended::NotSaved { .. } => Self::Storage,
            Ended::Unavailable(_) | Ended::Offline | Ended::Stopped => Self::Unreachable,
        }
    }
}

/// An identity a web operation got ready.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Prepared {
    /// The verified generation to write.
    pub(crate) identity: u64,
    /// Getting it used the operation's one sign-in with credentials.
    pub(crate) signed_in: bool,
}

/// A write of an identity into the web view, queued.
pub(crate) struct WebWrite {
    wait: tokio::sync::oneshot::Receiver<Result<(), WebError>>,
    answer: Option<Result<(), WebStop>>,
}

impl WebWrite {
    /// The platform's answer, waited for at most `within`; `None` while it
    /// has not come.
    pub(crate) async fn wait(&mut self, within: Duration) -> Option<Result<(), WebStop>> {
        if let Some(answer) = self.answer {
            return Some(answer);
        }
        let answer = match tokio::time::timeout(within, &mut self.wait).await {
            Err(_) => return None,
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(WebError::Stale))) => Err(WebStop::Stale),
            Ok(Ok(Err(WebError::Unsupported))) => Err(WebStop::Unsupported),
            Ok(Ok(Err(error))) => {
                log::warn!(target: "app_core::school_session", "web cookies not written: {error}");
                Err(WebStop::Storage)
            }
            Ok(Err(_)) => Err(WebStop::Storage),
        };
        self.answer = Some(answer);
        Some(answer)
    }
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
        let configured = PROFILES.map(|profile| auth.configured(profile));
        let pending = PROFILES.map(|profile| auth.sign_out_pending(profile));
        let account = |profile: SchoolAuthProfile, sites: Vec<SiteSession>| Account {
            profile,
            owner: {
                let owner = Owner::new();
                owner.set_sign_out_pending(pending[slot(profile)]);
                owner
            },
            commit: tokio::sync::Mutex::new(()),
            identity: Identity::new(configured[slot(profile)]),
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
            views: Mutex::new(None),
            tasks: RuntimeTasks::default(),
            revision: AtomicU64::new(1),
            published: Mutex::new(None),
            offline: AtomicBool::new(false),
            credentials: Arc::new(CredentialSignal::default()),
        }))
    }

    /// Sessions whose accounts are admitted and whose every site already
    /// holds an instance with an empty jar, for tests of what runs on a
    /// session (`http` for the live probes that send real requests).
    #[cfg(test)]
    pub(crate) fn ready(http: Option<Http>, auth: Arc<SchoolAuthService>) -> Self {
        let sessions = Self::new(http, auth);
        for account in &sessions.0.accounts {
            account.identity.set_configured(true);
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

    /// Lets a test act as if credentials were saved, with none to send.
    #[cfg(test)]
    pub(crate) fn assume_configured(&self, profile: SchoolAuthProfile) {
        self.0.account(profile).identity.set_configured(true);
    }

    /// The shell's write-only web cookie port. Set once.
    pub fn attach_web(&self, seed: Arc<dyn WebCookieSeed>) -> bool {
        self.0.web.attach(seed)
    }

    /// Whether the shell gave Core its web cookie port.
    pub(crate) fn seeds_web(&self) -> bool {
        self.0.web.attached()
    }

    /// The school browser engine, whose views a sign-out stops.
    pub(crate) fn attach_views(&self, views: Weak<dyn AccountViews>) {
        *guard(&self.0.views) = Some(views);
    }

    /// Both accounts as shells render them.
    pub fn snapshot(&self) -> SchoolSessionSnapshot {
        SchoolSessionSnapshot {
            revision: self.0.revision.load(Ordering::SeqCst),
            profiles: self.0.profiles(),
        }
    }

    /// Signs in with `typed`, the person's authoritative input, its blank
    /// fields filled from the saved credentials as the vault holds them
    /// now: it replaces a sign-in that is running, goes through a refusal or
    /// a pause, and only an acceptance saves anything
    /// (`identity::Inner::finish_sign_in`). The Connect takes the account's
    /// permit before it reads the vault and holds it until its save landed,
    /// so a sign-out or reset that begins meanwhile waits for it and stops
    /// it (`Stopped`), and nothing it read is saved after one.
    pub async fn connect(
        &self,
        profile: SchoolAuthProfile,
        typed: SchoolCredentials,
    ) -> Result<SchoolConnectOutcome, SchoolAuthError> {
        let inner = &self.0;
        let Some(permit) = inner.account(profile).owner.permit() else {
            return Ok(SchoolConnectOutcome::Stopped);
        };
        let intent = inner.account(profile).identity.begin_connect();
        let auth = inner.auth.clone();
        let credentials = tokio::task::spawn_blocking(move || {
            auth.refresh();
            auth.resolve(profile, typed)
        })
        .await
        .map_err(|_| {
            SchoolAuthError::Storage("The saved credentials could not be read.".into())
        })??;
        if permit.is_cancelled() {
            return Ok(SchoolConnectOutcome::Stopped);
        }
        inner.restore(profile).await;
        let seen = inner.account(profile).identity.generation();
        let how = identity::How::Connect {
            intent,
            credentials,
            account: permit.generation(),
        };
        let ended = inner.sign_in(profile, seen, how).await;
        inner.changed();
        Ok(match ended {
            Ok(_) => SchoolConnectOutcome::Connected,
            Err(Ended::Refused(problem)) if problem.kind.needs_web() => {
                SchoolConnectOutcome::NeedsWeb {
                    reason: problem.kind,
                }
            }
            Err(Ended::Refused(problem)) => SchoolConnectOutcome::Refused {
                reason: problem.kind,
            },
            Err(Ended::NotSaved { .. }) => SchoolConnectOutcome::NotSaved,
            Err(Ended::Stopped) => SchoolConnectOutcome::Stopped,
            Err(Ended::Unavailable(_) | Ended::Offline | Ended::NotConfigured) => {
                SchoolConnectOutcome::Unreachable
            }
        })
    }

    /// One foreground attempt with the credentials saved now, through a
    /// refusal or a pause. Saved credentials are read again first, so a
    /// password changed on another device is found here. Like a Connect it
    /// holds the account's permit from that read to its end: a sign-out or
    /// reset running or begun meanwhile stops it.
    pub async fn retry(&self, profile: SchoolAuthProfile) -> SchoolSessionSnapshot {
        let inner = &self.0;
        let account = inner.account(profile);
        let Some(permit) = account.owner.permit() else {
            return self.snapshot();
        };
        inner.auth.refresh();
        inner.restore(profile).await;
        account
            .identity
            .set_configured(inner.auth.configured(profile));
        let generation = account.identity.generation();
        let how = identity::How::Retry {
            account: permit.generation(),
        };
        let _ = inner.sign_in(profile, generation, how).await;
        drop(permit);
        inner.changed();
        self.snapshot()
    }

    /// Capture before fetching the remote backup, without holding up a sign-out.
    pub(crate) fn credential_generations(&self) -> [u64; 2] {
        PROFILES.map(|profile| self.account_generation(profile))
    }

    /// Drain both captured generations in reset order before replacing the bundle.
    /// Dropping the caller does not detach the vault write from these guards.
    pub(crate) async fn import_credentials(
        &self,
        payload: String,
        generations: [u64; 2],
    ) -> Result<usize, SchoolAuthError> {
        let sessions = self.clone();
        self.0
            .tasks
            .run(async move {
                let mut closed = Vec::new();
                for profile in PROFILES {
                    let account = sessions.0.account(profile);
                    closed.push(
                        account
                            .owner
                            .close_generation(generations[slot(profile)])
                            .await
                            .ok_or_else(|| {
                                SchoolAuthError::Storage(
                                    "The account changed during backup retrieval.".into(),
                                )
                            })?,
                    );
                }
                let auth = sessions.0.auth.clone();
                let present =
                    tokio::task::spawn_blocking(move || auth.import_sync_payload(&payload))
                        .await
                        .map_err(|_| {
                            SchoolAuthError::Storage("Credential import stopped.".into())
                        })??;
                for profile in PROFILES {
                    let account = sessions.0.account(profile);
                    let saved = present.contains(&profile);
                    account.identity.set_configured(saved);
                    if !saved {
                        account.identity.forget();
                        for site in &account.sites {
                            site.forget();
                        }
                    }
                }
                drop(closed);
                sessions.0.changed();
                sessions.0.publish();
                Ok(present.len())
            })
            .await
            .map_err(|error| SchoolAuthError::Storage(error.to_string()))?
    }

    /// The verified credentials as the synchronized bundle, for an
    /// encrypted credential backup. Never crosses the WebView IPC boundary.
    pub fn export_credentials(&self) -> Result<String, SchoolAuthError> {
        self.0.auth.export_sync_payload()
    }

    /// Forgets the account's credentials and signs it out of this device,
    /// behind the account barrier: admission closes, its sign-in and reads
    /// are stopped and waited for (a Connect the school already accepted
    /// included, until its vault write landed), and only then do Core's
    /// cookies, the LMS token, the saved session and every school website of
    /// the web view (both accounts', `clear_web`) go, and last the
    /// credentials. It succeeds only when every step did; otherwise the
    /// credentials stay saved, the error names what is left, and signing out
    /// again redoes every step.
    pub async fn sign_out(&self, profile: SchoolAuthProfile) -> Result<bool, SchoolAuthError> {
        let sessions = self.clone();
        self.0
            .tasks
            .run(async move { sessions.sign_out_owned(profile).await })
            .await
            .map_err(|error| SchoolAuthError::SignOutIncomplete(error.to_string()))?
    }

    async fn sign_out_owned(&self, profile: SchoolAuthProfile) -> Result<bool, SchoolAuthError> {
        // No web write is admitted from here on: the clear covers both
        // accounts.
        self.0.web.close_admission(&PROFILES);
        let (closed, mut unfinished) = self.0.close(profile, true).await;
        if !self.0.clear_web().await {
            unfinished.push("the school web sign-ins");
        }
        let forgotten = self.0.forget(&[profile], unfinished).await;
        drop(closed);
        log::info!(target: "app_core::school_session", "account barrier done profile={profile:?} {forgotten:?}");
        self.0
            .account(profile)
            .identity
            .set_configured(self.0.auth.configured(profile));
        self.0.changed();
        self.0.publish();
        forgotten.map(|()| true)
    }

    /// Clears the sessions of both accounts (a reset), the web view's school
    /// websites too when `web`, and the saved credentials too when
    /// `credentials`, each deleted only once both accounts' operations have
    /// ended, the credentials last and only when everything before them
    /// went: an error names what is left, and resetting again redoes it.
    pub async fn reset(&self, web: bool, credentials: bool) -> Result<(), SchoolAuthError> {
        if web {
            self.0.web.close_admission(&PROFILES);
        }
        // Both accounts stay closed until both are clean: one reopened early
        // would sign in while the other is still being cleared.
        let mut closed = Vec::new();
        let mut unfinished = Vec::new();
        for account in &self.0.accounts {
            let (guard, left) = self.0.close(account.profile, credentials).await;
            closed.push(guard);
            unfinished.extend(left);
        }
        if web && !self.0.clear_web().await {
            unfinished.push("the school web sign-ins");
        }
        let forgotten = if credentials {
            self.0.forget(&PROFILES, unfinished).await
        } else if unfinished.is_empty() {
            Ok(())
        } else {
            Err(SchoolAuthError::SignOutIncomplete(unfinished.join(", ")))
        };
        for account in &self.0.accounts {
            account
                .identity
                .set_configured(self.0.auth.configured(account.profile));
        }
        drop(closed);
        log::info!(target: "app_core::school_session", "sessions reset {forgotten:?}");
        self.0.changed();
        self.0.publish();
        forgotten
    }

    /// When the earliest pause after a sign-in without a verdict ends; the
    /// `PlanEngine` calls [`Self::lift_pauses`] then.
    pub(crate) fn next_pause_end(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let now = tokio::time::Instant::now();
        self.0
            .accounts
            .iter()
            .filter_map(|account| account.identity.pause_end())
            .min()
            .map(|until| {
                chrono::Utc::now()
                    + chrono::Duration::from_std(until.saturating_duration_since(now))
                        .unwrap_or_default()
            })
    }

    /// A pause that has run out goes, and shells see it.
    pub(crate) fn lift_pauses(&self) {
        let mut changed = false;
        for account in &self.0.accounts {
            changed |= account.identity.expire_pause(tokio::time::Instant::now());
        }
        if changed {
            self.0.changed();
            self.0.publish();
        }
    }

    /// The network the shell reports (`Hint::Network`). While it is
    /// offline a sign-in ends without sending anything.
    pub(crate) fn network(&self, network: crate::plan::Network) {
        self.0
            .offline
            .store(network == crate::plan::Network::Offline, Ordering::SeqCst);
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

    /// The signal sync listens to, shared with the LMS token's holder.
    pub(crate) fn credential_signal(&self) -> Arc<CredentialSignal> {
        self.0.credentials.clone()
    }

    /// What `profile`'s credentials allow now. Reads only memory.
    pub(crate) fn account_facts(&self, profile: SchoolAuthProfile) -> AccountFacts {
        let account = self.0.account(profile);
        let restored = account.restored.initialized();
        let (identity, sign_in) = self.0.identity_facts(profile);
        AccountFacts {
            admitted: account.owner.admits() && account.identity.configured(),
            identity: identity || !restored,
            sign_in,
            sites: account
                .sites
                .iter()
                .filter(|site| !restored || site.present())
                .map(|site| site.site)
                .collect(),
        }
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

    /// An attempt for a write: nothing has been sent yet, and the site's
    /// session is got (signing in if needed) only when a request needs
    /// cookies. Never recovered or run again.
    #[cfg(feature = "agent")]
    pub(crate) async fn write_attempt(
        &self,
        site: Site,
        permit: &Permit,
        deadline: tokio::time::Instant,
    ) -> Attempt {
        self.0.write_attempt(site, permit, deadline).await
    }

    /// The identity a web operation can write at once: verified less than
    /// `VERIFIED_FRESH` ago, the account usable. `None` when getting one
    /// needs the school.
    pub(crate) fn fresh(&self, profile: SchoolAuthProfile) -> Option<u64> {
        self.0.fresh(profile)
    }

    /// Gets `profile`'s identity ready for a web operation: a fresh one as
    /// it is, an unverified or aging one verified, and only when there is
    /// none (or the school found it invalid) a sign-in with the saved
    /// credentials. A refusal on record always stops it; a pause after a
    /// failure stops it unless `past_pause` (the person's Reload on the
    /// sign-in page).
    pub(crate) async fn prepare_web(
        &self,
        profile: SchoolAuthProfile,
        past_pause: bool,
    ) -> Result<Prepared, WebStop> {
        self.0.prepare_web(profile, past_pause).await
    }

    /// The account name of `profile`'s verified identity, got ready as for
    /// a web operation (`prepare_web`, never past a pause). It is read with
    /// the account's commit held, so it is the name of the credentials that
    /// made that identity: a sign-in applies its identity and saves its
    /// credentials under that lock as one commit.
    #[cfg(feature = "campus-printer")]
    pub(crate) async fn verified_account_name(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<String, WebStop> {
        let inner = &self.0;
        let prepared = inner.prepare_web(profile, false).await?;
        let account = inner.account(profile);
        let _commit = account.commit.lock().await;
        let Some(permit) = account.owner.permit() else {
            return Err(WebStop::Stale);
        };
        if inner.writable(profile, prepared.identity).is_none() {
            return Err(WebStop::Stale);
        }
        let auth = inner.auth.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            auth.load(profile)
        })
        .await;
        match loaded {
            Ok(Ok(Some(credentials))) => Ok(credentials.username),
            Ok(Ok(None)) => Err(WebStop::NotConfigured),
            Ok(Err(_)) | Err(_) => Err(WebStop::Storage),
        }
    }

    /// A web operation's one sign-in with the saved credentials: the web
    /// page refused the verified identity `seen`. A newer identity that
    /// appeared meanwhile is used instead.
    pub(crate) async fn sign_in_web(
        &self,
        profile: SchoolAuthProfile,
        seen: u64,
        past_pause: bool,
    ) -> Result<u64, WebStop> {
        self.0
            .sign_in(profile, seen, identity::How::Web { past_pause })
            .await
            .map_err(|ended| WebStop::of(&ended))
    }

    /// The one exit of an identity into the web view: queues the cookies of
    /// `profile`'s verified generation `identity`, while the account is
    /// usable. The queue drops them if, by their turn, the account was signed
    /// out or reset or that generation is no longer the one to write.
    pub(crate) fn seed(
        &self,
        profile: SchoolAuthProfile,
        identity: u64,
    ) -> Result<WebWrite, WebStop> {
        self.0.seed(profile, identity)
    }

    pub(crate) fn sign_out_pending(&self, profile: SchoolAuthProfile) -> bool {
        self.0.account(profile).owner.sign_out_pending()
    }

    pub(crate) fn web_cleanup_pending(&self) -> bool {
        PROFILES
            .iter()
            .any(|profile| self.sign_out_pending(*profile))
    }

    /// The account generation of `profile`, which a web operation checks
    /// did not move.
    pub(crate) fn account_generation(&self, profile: SchoolAuthProfile) -> u64 {
        self.0.account(profile).owner.generation()
    }

    /// Grows with every clear of the web view's store the platform
    /// confirmed, which a web operation checks did not move.
    pub(crate) fn web_storage(&self) -> u64 {
        self.0.web.storage()
    }

    /// Ends every sign-in and keeps the sites' jars for the next launch.
    pub async fn shutdown(&self) {
        self.suspend();
        for account in &self.0.accounts {
            self.0
                .persist(account.profile, account.owner.generation())
                .await;
        }
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }
}

const PROFILES: [SchoolAuthProfile; 2] = [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech];

fn slot(profile: SchoolAuthProfile) -> usize {
    match profile {
        SchoolAuthProfile::Science => 0,
        SchoolAuthProfile::TokyoTech => 1,
    }
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

    /// A credential fact changed (`CredentialSignal`).
    fn changed(&self) {
        self.credentials.changed();
    }

    fn profile(&self, profile: SchoolAuthProfile) -> SchoolProfileSession {
        let account = self.account(profile);
        let (state, problem, retry_at) = account.identity.view(self.auth.refusal(profile));
        SchoolProfileSession {
            sign_out_pending: account.owner.sign_out_pending(),
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

    /// Reads the account's vault items once: its identity, the sites' jars
    /// and the refusals kept on this device. The read holds a permit until
    /// what it read is installed, so a barrier forgets and deletes only
    /// after it.
    async fn restore(&self, profile: SchoolAuthProfile) {
        let account = self.account(profile);
        let first = AtomicBool::new(false);
        account
            .restored
            .get_or_init(|| async {
                first.store(true, Ordering::SeqCst);
                // A barrier is running: it deletes the item anyway.
                let Some(_permit) = account.owner.permit() else {
                    return;
                };
                let auth = self.auth.clone();
                let loaded = tokio::task::spawn_blocking(move || {
                    if let Err(error) = auth.load_refusals() {
                        log::warn!(target: "app_core::school_session", "refusals not read: {error}");
                    }
                    let saved = auth.load_session(profile).ok().flatten()?;
                    Saved::decode(&saved)
                })
                .await
                .ok()
                .flatten();
                let Some(saved) = loaded else { return };
                let snapshot = Snapshot {
                    cookies: saved.identity.clone(),
                };
                let Some(generation) = account.identity.restore(snapshot) else {
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
        // The jars and refusals are known now, no longer unchecked.
        if first.load(Ordering::SeqCst) {
            self.changed();
        }
        self.publish();
    }

    /// Saves the identity and every site's jar of the account, for an
    /// operation of account generation `generation`. The write holds a
    /// permit of that generation until it lands, however the caller ends: a
    /// barrier waits for it, so it never lands after the barrier deleted
    /// the item, and an operation of an older generation writes nothing.
    async fn persist(&self, profile: SchoolAuthProfile, generation: u64) {
        let account = self.account(profile);
        let Some(permit) = account.owner.permit_of(generation) else {
            return;
        };
        let Some(snapshot) = account.identity.persisted() else {
            return;
        };
        let mut saved = Saved::new(snapshot.cookies.clone());
        for site in &account.sites {
            saved = saved.with_site(site.site, site.saved_jar());
        }
        let Some(text) = saved.encode() else { return };
        let auth = self.auth.clone();
        let stored = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            auth.store_session(profile, &text)
        })
        .await;
        if let Ok(Err(error)) = stored {
            log::warn!(target: "app_core::school_session", "session not saved profile={profile:?}: {error}");
        }
    }

    /// The barrier. It closes admission first, then cancels the account's
    /// sign-in and reads and waits for every operation of it (writes run on
    /// to their end), and only then, still closed, deletes Core's identity
    /// and sites, the LMS token and the saved session. It reopens, one
    /// generation on, when the guard drops. The result names what could not
    /// be deleted; the caller deletes the credentials only once nothing is
    /// left (`forget`), so a sign-out that did not finish keeps the account
    /// saved and the person can sign out again.
    async fn close(
        &self,
        profile: SchoolAuthProfile,
        forget: bool,
    ) -> (owner::Closed<'_>, Vec<&'static str>) {
        let account = self.account(profile);
        let closed = account.owner.close().await;
        // Closed: the account's reads are not admitted until it reopens.
        self.changed();
        if forget {
            account.owner.set_sign_out_pending(true);
            if let Err(error) = self.auth.mark_sign_out(profile, true) {
                log::warn!("Sign-out intent could not be saved: {error}");
                return (closed, vec!["the local sign-out record"]);
            }
        }
        account.identity.cancel_flight();
        account.identity.wait_flight().await;
        account.identity.forget();
        for site in &account.sites {
            site.forget();
        }
        self.changed();
        let auth = self.auth.clone();
        let deleted = tokio::task::spawn_blocking(move || {
            let token = auth.delete_moodle_token(profile);
            let session = auth.delete_session(profile);
            (token, session)
        })
        .await;
        let mut unfinished = Vec::new();
        match deleted {
            Ok((token, session)) => {
                if let Err(error) = token {
                    log::warn!(target: "app_core::school_session", "LMS token not deleted profile={profile:?}: {error}");
                    unfinished.push("the LMS token");
                }
                if let Err(error) = session {
                    log::warn!(target: "app_core::school_session", "session not deleted profile={profile:?}: {error}");
                    unfinished.push("the saved school session");
                }
            }
            Err(_) => unfinished.push("the saved school session"),
        }
        (closed, unfinished)
    }

    /// Deletes the saved credentials of `profiles`, last of a sign-out or a
    /// reset and only when nothing else is `unfinished`: otherwise they stay,
    /// with the account's sign-out, and the error names what is left.
    async fn forget(
        &self,
        profiles: &[SchoolAuthProfile],
        unfinished: Vec<&'static str>,
    ) -> Result<(), SchoolAuthError> {
        if !unfinished.is_empty() {
            return Err(SchoolAuthError::SignOutIncomplete(unfinished.join(", ")));
        }
        let auth = self.auth.clone();
        let batch = profiles.to_vec();
        tokio::task::spawn_blocking(move || {
            for profile in &batch {
                auth.delete(*profile)?;
            }
            for profile in batch {
                auth.mark_sign_out(profile, false)?;
            }
            Ok::<_, SchoolAuthError>(())
        })
        .await
        .map_err(|_| SchoolAuthError::SignOutIncomplete("the saved credentials".into()))?
        .map_err(|error| SchoolAuthError::SignOutIncomplete(error.to_string()))?;
        for profile in profiles {
            self.account(*profile).owner.set_sign_out_pending(false);
        }
        Ok(())
    }

    /// Clears every school website from the web view, both accounts', in
    /// order: close both accounts' admission, stop both accounts' views, let
    /// the writes already handed to the platform end (the queue runs them
    /// first), clear, and wait for the platform's confirmation, after which
    /// the store moves one generation on: a web operation begun before it
    /// stops, and the account still signed in has its identity written by
    /// its next one. The caller waits at most `WEB_STORE_LIMIT`; a clear
    /// still queued then runs once the store answers, and the queue admits
    /// no write meanwhile. Whether the platform confirmed it (true without a
    /// web cookie port: there is nothing to clear).
    async fn clear_web(&self) -> bool {
        if !self.web.attached() {
            return true;
        }
        self.web.close_admission(&PROFILES);
        let views = guard(&self.views).as_ref().and_then(Weak::upgrade);
        if let Some(views) = views {
            views.stop();
        }
        let wait = self.web.clear(&self.tasks);
        match tokio::time::timeout(WEB_STORE_LIMIT, wait).await {
            Ok(Ok(Ok(()))) => {
                log::info!(target: "app_core::school_session", "school websites cleared");
                true
            }
            Ok(Ok(Err(error))) => {
                log::warn!(target: "app_core::school_session", "school websites not cleared: {error}");
                false
            }
            Ok(Err(_)) => {
                log::warn!(target: "app_core::school_session", "school websites not cleared: the queue stopped");
                false
            }
            Err(_) => {
                log::warn!(target: "app_core::school_session", "school websites not cleared yet; the clear runs once the store answers");
                false
            }
        }
    }

    /// Whether nothing stands in the way of the account's identity: its
    /// derived state is normal.
    fn usable(&self, profile: SchoolAuthProfile) -> bool {
        !self.account(profile).owner.sign_out_pending()
            && self.profile(profile).state == SchoolSessionState::Normal
    }

    fn fresh(&self, profile: SchoolAuthProfile) -> Option<u64> {
        self.account(profile)
            .identity
            .fresh()
            .filter(|_| self.usable(profile))
    }

    /// The snapshot a web view may be given: `generation` verified, the
    /// account usable.
    fn writable(&self, profile: SchoolAuthProfile, generation: u64) -> Option<Arc<Snapshot>> {
        self.account(profile)
            .identity
            .verified(generation)
            .filter(|_| self.usable(profile))
    }

    async fn prepare_web(
        self: &Arc<Self>,
        profile: SchoolAuthProfile,
        past_pause: bool,
    ) -> Result<Prepared, WebStop> {
        self.restore(profile).await;
        match self.profile(profile).state {
            SchoolSessionState::NotConfigured => return Err(WebStop::NotConfigured),
            SchoolSessionState::Refused => return Err(WebStop::Rejected),
            SchoolSessionState::NeedsWeb => return Err(WebStop::NeedsWeb),
            SchoolSessionState::Unavailable if !past_pause => return Err(WebStop::Unreachable),
            _ => {}
        }
        if let Some(generation) = self.fresh(profile) {
            return Ok(Prepared {
                identity: generation,
                signed_in: false,
            });
        }
        match self.verify(profile).await {
            Checked::Verified(generation) if self.writable(profile, generation).is_some() => {
                return Ok(Prepared {
                    identity: generation,
                    signed_in: false,
                })
            }
            Checked::Verified(_) | Checked::Unconfirmed => return Err(WebStop::Unreachable),
            Checked::Expired => {}
        }
        let seen = self.account(profile).identity.generation();
        let generation = self
            .sign_in(profile, seen, identity::How::Web { past_pause })
            .await
            .map_err(|ended| WebStop::of(&ended))?;
        Ok(Prepared {
            identity: generation,
            signed_in: true,
        })
    }

    fn seed(
        self: &Arc<Self>,
        profile: SchoolAuthProfile,
        identity: u64,
    ) -> Result<WebWrite, WebStop> {
        if !self.web.attached() {
            return Err(WebStop::Unsupported);
        }
        let account = self.account(profile);
        let Some(_permit) = account.owner.permit() else {
            return Err(WebStop::Stale);
        };
        let Some(snapshot) = self.writable(profile, identity) else {
            return Err(WebStop::Stale);
        };
        if !self.web.usable() {
            return Err(WebStop::Storage);
        }
        let sessions = Arc::downgrade(self);
        let valid = Box::new(move || {
            sessions
                .upgrade()
                .is_some_and(|sessions| sessions.writable(profile, identity).is_some())
        });
        Ok(WebWrite {
            wait: self
                .web
                .write(&self.tasks, profile, snapshot.cookies.clone(), valid),
            answer: None,
        })
    }
}

#[cfg(test)]
#[path = "../tests/school_session.rs"]
mod tests;
