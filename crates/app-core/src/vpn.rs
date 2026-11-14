//! Campus VPN decisions: what state the person sees, what a connect or a
//! disconnect asks of the helper, and how the helper's report becomes that
//! state. The helper socket and the desktop shell live behind `VpnPort`;
//! nothing here does IO.

pub mod f5;

use crate::school_session::CampusVpnError;
use chrono::Duration;
use chrono::{DateTime, Utc};
use std::net::IpAddr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};
use thiserror::Error;
use vpn_protocol::{Ending, HelperState, SessionCookie, HELPER_REVISION};

/// Failures the person can act on. `code` is the stable name the UI keys its
/// guidance on, e.g. "helper_not_installed" leads to the install step.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VpnError {
    #[error("the campus VPN helper is not installed")]
    HelperNotInstalled,
    /// The installed helper is older than the one this app bundles.
    #[error("the campus VPN helper needs an update")]
    HelperOutdated,
    #[error("the campus VPN helper is unreachable: {0}")]
    HelperUnreachable(String),
    #[error("the campus VPN helper did not answer in time")]
    Timeout,
    #[error("the campus VPN helper sent an unexpected reply: {0}")]
    Protocol(String),
    #[error("the campus VPN tunnel failed: {0}")]
    Tunnel(String),
    /// Signing in to the APM failed; the helper was not asked.
    #[error("{0}")]
    Login(crate::school_session::CampusVpnError),
}

impl VpnError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::HelperNotInstalled => "helper_not_installed",
            Self::HelperOutdated => "helper_outdated",
            Self::HelperUnreachable(_) => "helper_unreachable",
            Self::Timeout => "helper_timeout",
            Self::Protocol(_) => "helper_protocol",
            Self::Tunnel(_) => "tunnel_failed",
            Self::Login(error) => error.code(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VpnState {
    Disconnected,
    Connecting,
    Connected {
        address: Option<String>,
        /// When the APM session behind the tunnel ends, if known.
        expires_at: Option<DateTime<Utc>>,
    },
    Error(VpnError),
}

/// The helper's answer, already parsed from its wire reply by the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperReport {
    pub state: HelperState,
    pub address: Option<String>,
    pub detail: Option<String>,
    /// When the tunnel's session ends, as openconnect reported it.
    pub expires_at: Option<DateTime<Utc>>,
    /// The answering helper's revision; `None` from the first helpers.
    pub revision: Option<u32>,
    /// Why the helper ended the last tunnel itself.
    pub ended: Option<Ending>,
}

/// What the person is told without asking: once before the session's end,
/// once when the tunnel ends without a disconnect, when the helper ended one
/// left unused with connections still open, and when on-demand connecting
/// gives up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VpnNotice {
    EndingSoon {
        minutes: i64,
    },
    Ended,
    /// Half an hour without use, ended though connections (a forgotten SSH)
    /// were still open through it. Told even with on-demand on: those
    /// connections are cut.
    Unused,
    /// Signing in again cannot help; `code` is `VpnError::code`.
    DemandStopped {
        code: &'static str,
    },
}

/// One periodic check's outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VpnCheck {
    pub state: VpnState,
    pub notice: Option<VpnNotice>,
}

/// A tunnel up this long proves the sign-in works; the counts restart. It
/// must stay shorter than the helper's idle disconnect (5 minutes), or every
/// tunnel ended for being unused would count as one that did not last.
const STABLE_MINUTES: i64 = 3;
/// Logins the school accepted whose tunnel did not last, in a row, before
/// on-demand connecting gives up rather than keep posting the password.
const MAX_SPENT: u32 = 6;
/// Sign-ins on-demand connecting starts by itself within an hour, at most.
/// Something that touches the campus network every few minutes would
/// otherwise sign in each time the idle tunnel ends, which the school would
/// see as a login storm. A press is never held back.
const HOURLY_LOGINS: usize = 6;

/// The sign-ins on-demand connecting made on its own,
/// which must not keep posting a password that cannot work.
#[derive(Clone, Debug, Default)]
struct Attempts {
    /// Failed or short-lived reconnects in a row, for the wait between them.
    failures: u32,
    /// Of those, the ones that posted the password and got a session.
    spent: u32,
    retry_at: Option<DateTime<Utc>>,
    up_since: Option<DateTime<Utc>>,
    /// When the sign-ins of the last hour started.
    logins: Vec<DateTime<Utc>>,
}

impl Attempts {
    /// The next attempt waits longer the more have failed: 1, 2, then 5 min.
    fn back_off(&mut self, now: DateTime<Utc>) {
        self.failures += 1;
        let minutes = match self.failures {
            1 => 1,
            2 => 2,
            _ => 5,
        };
        self.retry_at = Some(now + Duration::minutes(minutes));
    }

    fn due(&self, now: DateTime<Utc>) -> bool {
        self.retry_at.is_none_or(|at| now >= at) && self.paused_until(now).is_none()
    }

    /// While the hour's sign-ins are spent, when the next one may start.
    fn paused_until(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let hour = Duration::hours(1);
        let recent: Vec<_> = self.logins.iter().filter(|at| now - **at < hour).collect();
        (recent.len() >= HOURLY_LOGINS).then(|| *recent[recent.len() - HOURLY_LOGINS] + hour)
    }

    fn login(&mut self, now: DateTime<Utc>) {
        self.logins.retain(|at| now - *at < Duration::hours(1));
        self.logins.push(now);
    }

    /// A tunnel seen up at `now`: one up this long proves the sign-in works.
    fn up(&mut self, now: DateTime<Utc>) {
        let since = *self.up_since.get_or_insert(now);
        if now - since >= Duration::minutes(STABLE_MINUTES) {
            *self = Self {
                up_since: Some(since),
                logins: std::mem::take(&mut self.logins),
                ..Self::default()
            };
        }
    }

    /// A sign-in of its own finished as `state`. A tunnel that came up still
    /// counts until it lasts: one that dies at once must not loop on the
    /// password. `Err` is the code to stop with.
    fn finished(&mut self, state: &VpnState, now: DateTime<Utc>) -> Result<(), &'static str> {
        match state {
            VpnState::Error(error) if hopeless(error) => return Err(error.code()),
            VpnState::Connected { .. } | VpnState::Error(VpnError::Tunnel(_)) => {
                self.spent += 1;
                if self.spent >= MAX_SPENT {
                    return Err(VpnError::Tunnel(String::new()).code());
                }
            }
            // Offline or the school not answering: no password was accepted,
            // so this only waits.
            _ => {}
        }
        if matches!(state, VpnState::Connected { .. }) {
            self.up_since = Some(now);
        }
        self.back_off(now);
        Ok(())
    }
}

/// Reconnecting cannot fix these: a refused password must not be sent again,
/// and the rest need the person.
fn hopeless(error: &VpnError) -> bool {
    matches!(
        error,
        VpnError::HelperNotInstalled
            | VpnError::HelperOutdated
            | VpnError::Login(
                CampusVpnError::NeedsAccount
                    | CampusVpnError::Rejected
                    | CampusVpnError::NeedsWeb
                    | CampusVpnError::Denied
                    | CampusVpnError::Changed(_)
                    | CampusVpnError::SignIn
            )
    )
}

/// Held while one sign-in to the APM runs: the sessions are one-time, so a
/// press and a reconnect must not each spend one.
pub struct LoginGuard(Arc<AtomicBool>);

impl Drop for LoginGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// How long before the session's end the reminder comes.
pub const ENDING_NOTICE_MINUTES: i64 = 10;

/// The helper's socket. Implementations block; the shell calls them off the
/// UI thread.
pub trait VpnPort: Send + Sync {
    fn status(&self) -> Result<HelperReport, VpnError>;
    fn start(&self, cookie: &SessionCookie) -> Result<HelperReport, VpnError>;
    fn stop(&self) -> Result<HelperReport, VpnError>;
}

/// The state a helper report means to the person. The helper keeps the last
/// tunnel failure until the next start or stop, so a disconnected helper
/// that still carries a detail has failed rather than stopped cleanly. An
/// outdated helper keeps a tunnel it has up; with none up it is updated
/// before the next connect.
pub fn project(report: HelperReport) -> VpnState {
    if report.state != HelperState::Connected && report.revision < Some(HELPER_REVISION) {
        return VpnState::Error(VpnError::HelperOutdated);
    }
    match report.state {
        HelperState::Connected => VpnState::Connected {
            address: report.address,
            expires_at: report.expires_at,
        },
        HelperState::Connecting => VpnState::Connecting,
        HelperState::Disconnected => match report.detail {
            Some(detail) => VpnState::Error(VpnError::Tunnel(detail)),
            None => VpnState::Disconnected,
        },
        HelperState::Error => VpnState::Error(VpnError::Tunnel(
            report.detail.unwrap_or_else(|| "unknown".into()),
        )),
    }
}

pub struct VpnService {
    port: Arc<dyn VpnPort>,
    state: Mutex<VpnState>,
    /// The end the ending reminder was given for, so it comes once.
    warned: Mutex<Option<DateTime<Utc>>>,
    /// On while on-demand connecting is: a standby demand may sign in.
    on_demand: Mutex<Option<Attempts>>,
    logging_in: Arc<AtomicBool>,
    /// On the campus network, as `network` last read it: the VPN is not
    /// needed there.
    on_campus: AtomicBool,
}

impl VpnService {
    pub fn new(port: Arc<dyn VpnPort>) -> Self {
        Self {
            port,
            state: Mutex::new(VpnState::Disconnected),
            warned: Mutex::new(None),
            on_demand: Mutex::new(None),
            logging_in: Arc::new(AtomicBool::new(false)),
            on_campus: AtomicBool::new(false),
        }
    }

    /// The one sign-in in progress, or `None` when another holds it.
    pub fn begin_login(&self) -> Option<LoginGuard> {
        (!self.logging_in.swap(true, Ordering::SeqCst)).then(|| LoginGuard(self.logging_in.clone()))
    }

    pub fn state(&self) -> VpnState {
        self.lock().clone()
    }

    /// Ask the helper what it is doing, for example after the app starts.
    pub fn refresh(&self) -> VpnState {
        let next = Self::settle(self.port.status());
        self.store(next)
    }

    /// Whether a connect would start a tunnel: not while one is up, so a
    /// second press spends no new login and starts no second openconnect.
    pub fn needs_login(&self) -> bool {
        !matches!(self.state(), VpnState::Connected { .. })
    }

    /// Connect with a fresh session cookie. The helper ends any tunnel, this
    /// one included, once it carries nothing for a while.
    pub fn connect(&self, cookie: &SessionCookie) -> VpnState {
        if !self.needs_login() {
            return self.state();
        }
        self.store(VpnState::Connecting);
        let next = Self::settle(self.port.start(cookie));
        self.store(next)
    }

    /// Turns on-demand connecting on or off. While on, the shell keeps the
    /// helper's standby open and passes its demands to `demanded`.
    pub fn set_on_demand(&self, on: bool) {
        let mut current = lock(&self.on_demand);
        match (on, current.is_some()) {
            (true, false) => *current = Some(Attempts::default()),
            (false, _) => *current = None,
            (true, true) => {}
        }
    }

    pub fn on_demand(&self) -> bool {
        lock(&self.on_demand).is_some()
    }

    /// The device's addresses now; whether it is on the campus network.
    /// While a sign-in runs or a tunnel is up the last answer stands: the
    /// tunnel's own campus address appears before Core knows it, and would
    /// read as the campus network and tear standby down mid-sign-in.
    pub fn network(&self, addresses: impl IntoIterator<Item = IpAddr>) -> bool {
        if self.logging_in.load(Ordering::SeqCst)
            || matches!(
                self.state(),
                VpnState::Connected { .. } | VpnState::Connecting
            )
        {
            return self.on_campus();
        }
        let on = crate::campus_network::on_campus(addresses, None);
        self.on_campus.store(on, Ordering::SeqCst);
        on
    }

    pub fn on_campus(&self) -> bool {
        self.on_campus.load(Ordering::SeqCst)
    }

    /// Whether the helper's standby should hold the campus routes: on-demand
    /// is on and campus hosts are not reached directly anyway.
    pub fn wants_standby(&self) -> bool {
        self.on_demand() && !self.on_campus()
    }

    /// Whether a standby demand at `now` should sign in: standby is wanted,
    /// no tunnel is up, the wait after a failed attempt is over, and the
    /// hour's sign-ins are not spent. A yes counts as one of them.
    pub fn demanded(&self, now: DateTime<Utc>) -> bool {
        if !self.needs_login() || self.on_campus() {
            return false;
        }
        let mut current = lock(&self.on_demand);
        let Some(attempts) = current.as_mut().filter(|attempts| attempts.due(now)) else {
            return false;
        };
        attempts.login(now);
        true
    }

    /// While on-demand connecting has spent the hour's sign-ins, when it may
    /// sign in again; the card says so.
    pub fn demand_paused_until(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        lock(&self.on_demand).as_ref()?.paused_until(now)
    }

    /// A demand's sign-in finished as `state`. What signing in again cannot
    /// fix turns on-demand off.
    pub fn demand_finished(&self, state: &VpnState, now: DateTime<Utc>) -> Option<VpnNotice> {
        let mut current = lock(&self.on_demand);
        let code = current.as_mut()?.finished(state, now).err()?;
        *current = None;
        Some(VpnNotice::DemandStopped { code })
    }

    /// Campus hosts standby must leave to the physical network: the hosts
    /// Core itself signs in to and reads, so neither the sign-in a demand
    /// starts nor the app's own refreshes wait on a tunnel.
    pub fn standby_bypass_hosts() -> impl Iterator<Item = &'static str> {
        crate::net::Host::ALL
            .into_iter()
            .map(crate::net::Host::name)
    }

    /// The periodic check while a tunnel is up: the helper's state and the
    /// reminder due at `now`.
    pub fn watch(&self, now: DateTime<Utc>) -> VpnCheck {
        let before = self.state();
        let report = self.port.status();
        let ended = report.as_ref().ok().and_then(|report| report.ended);
        let state = self.store(Self::settle(report));
        if matches!(state, VpnState::Connected { .. }) {
            if let Some(attempts) = lock(&self.on_demand).as_mut() {
                attempts.up(now);
            }
        }
        // With on-demand on, an ending is not news: the next use brings the
        // tunnel back, and an unused one is ended on purpose.
        let notice = self
            .notice(&before, &state, now)
            .map(|notice| match (notice, ended) {
                (VpnNotice::Ended, Some(Ending::Unused)) => VpnNotice::Unused,
                (notice, _) => notice,
            })
            .filter(|notice| *notice != VpnNotice::Ended || !self.on_demand());
        VpnCheck { state, notice }
    }

    /// A tunnel that was up and now is not, without a disconnect (which
    /// stores `Disconnected` before any check sees it), ended on its own. A
    /// helper that does not answer is not an ending.
    fn notice(&self, before: &VpnState, after: &VpnState, now: DateTime<Utc>) -> Option<VpnNotice> {
        match (before, after) {
            (
                VpnState::Connected { .. },
                VpnState::Disconnected | VpnState::Error(VpnError::Tunnel(_)),
            ) => Some(VpnNotice::Ended),
            (
                _,
                VpnState::Connected {
                    expires_at: Some(end),
                    ..
                },
            ) => self.ending(*end, now),
            _ => None,
        }
    }

    /// The reminder for `end`, once, in the last minutes before it.
    fn ending(&self, end: DateTime<Utc>, now: DateTime<Utc>) -> Option<VpnNotice> {
        let left = (end - now).num_seconds();
        let mut warned = lock(&self.warned);
        if left <= 0 || left > ENDING_NOTICE_MINUTES * 60 || *warned == Some(end) {
            return None;
        }
        *warned = Some(end);
        Some(VpnNotice::EndingSoon {
            minutes: (left + 59) / 60,
        })
    }

    /// The login step failed before the helper was asked: the error is the
    /// state the person sees.
    pub fn failed(&self, error: VpnError) -> VpnState {
        self.store(VpnState::Error(error))
    }

    /// The person's disconnect.
    pub fn disconnect(&self) -> VpnState {
        let next = Self::settle(self.port.stop());
        self.store(next)
    }

    fn settle(result: Result<HelperReport, VpnError>) -> VpnState {
        match result {
            Ok(report) => project(report),
            Err(error) => VpnState::Error(error),
        }
    }

    fn store(&self, next: VpnState) -> VpnState {
        *self.lock() = next.clone();
        next
    }

    fn lock(&self) -> MutexGuard<'_, VpnState> {
        lock(&self.state)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "tests/vpn.rs"]
mod tests;
