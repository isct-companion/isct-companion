//! Campus VPN decisions: what state the person sees, what a connect or a
//! disconnect asks of the helper, and how the helper's report becomes that
//! state. The helper socket and the desktop shell live behind `VpnPort`;
//! nothing here does IO.

pub mod f5;

use crate::school_session::CampusVpnError;
use chrono::Duration;
use chrono::{DateTime, Utc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard,
};
use thiserror::Error;
use vpn_protocol::{HelperState, SessionCookie, HELPER_REVISION};

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
}

/// What the person is told without asking: once before the session's or the
/// auto-continue's end, once when the tunnel ends without a disconnect, and
/// when auto-continue finishes or gives up.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VpnNotice {
    EndingSoon {
        minutes: i64,
    },
    Ended,
    /// Its time is up; the tunnel is being disconnected.
    KeepEnded,
    /// Reconnecting cannot help; `code` is `VpnError::code`.
    KeepStopped {
        code: &'static str,
    },
}

/// What a check asks the shell to do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VpnAction {
    /// Sign in again and connect (`begin_login`, `connect`, then `reconnected`).
    Reconnect,
    Disconnect,
}

/// One periodic check's outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VpnCheck {
    pub state: VpnState,
    pub notice: Option<VpnNotice>,
    pub action: Option<VpnAction>,
}

/// Auto-continue always ends: it needs a duration of at most this long.
pub const KEEP_MAX_HOURS: u32 = 24;
/// A tunnel up this long proves the reconnecting works; the counts restart.
const KEEP_STABLE_MINUTES: i64 = 5;
/// Logins the school accepted whose tunnel did not last, in a row, before
/// auto-continue gives up rather than keep posting the password.
const KEEP_MAX_SPENT: u32 = 6;

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum VpnKeepError {
    #[error("auto-continue needs a duration of 1 to {KEEP_MAX_HOURS} hours")]
    InvalidDuration,
    #[error("auto-continue starts from a connected tunnel")]
    NotConnected,
}

#[derive(Clone, Copy, Debug)]
struct Keep {
    until: DateTime<Utc>,
    /// Failed or short-lived reconnects in a row, for the wait between them.
    failures: u32,
    /// Of those, the ones that posted the password and got a session.
    spent: u32,
    retry_at: Option<DateTime<Utc>>,
    up_since: Option<DateTime<Utc>>,
}

impl Keep {
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
    keep: Mutex<Option<Keep>>,
    logging_in: Arc<AtomicBool>,
}

impl VpnService {
    pub fn new(port: Arc<dyn VpnPort>) -> Self {
        Self {
            port,
            state: Mutex::new(VpnState::Disconnected),
            warned: Mutex::new(None),
            keep: Mutex::new(None),
            logging_in: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The one sign-in in progress, or `None` when another holds it.
    pub fn begin_login(&self) -> Option<LoginGuard> {
        (!self.logging_in.swap(true, Ordering::SeqCst)).then(|| LoginGuard(self.logging_in.clone()))
    }

    /// Reconnects whenever the tunnel ends, for `hours` from `now`, then
    /// disconnects. There is no open-ended form. `None` turns it off and
    /// leaves the tunnel up.
    pub fn set_keep(
        &self,
        hours: Option<u32>,
        now: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, VpnKeepError> {
        let Some(hours) = hours else {
            *lock(&self.keep) = None;
            return Ok(None);
        };
        if hours == 0 || hours > KEEP_MAX_HOURS {
            return Err(VpnKeepError::InvalidDuration);
        }
        if self.needs_login() {
            return Err(VpnKeepError::NotConnected);
        }
        let until = now + Duration::hours(i64::from(hours));
        *lock(&self.keep) = Some(Keep {
            until,
            failures: 0,
            spent: 0,
            retry_at: None,
            up_since: Some(now),
        });
        Ok(Some(until))
    }

    pub fn keep_until(&self) -> Option<DateTime<Utc>> {
        lock(&self.keep).map(|keep| keep.until)
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

    /// Connect with a fresh session cookie.
    pub fn connect(&self, cookie: &SessionCookie) -> VpnState {
        if !self.needs_login() {
            return self.state();
        }
        self.store(VpnState::Connecting);
        let next = Self::settle(self.port.start(cookie));
        self.store(next)
    }

    /// The periodic check while a tunnel is up or auto-continue is on: the
    /// helper's state, the reminder due at `now`, and what to do about it.
    pub fn watch(&self, now: DateTime<Utc>) -> VpnCheck {
        let before = self.state();
        let state = self.refresh();
        let keep = *lock(&self.keep);
        let (notice, action) = match keep {
            Some(keep) => self.keeping(keep, &state, now),
            None => (self.notice(&before, &state, now), None),
        };
        VpnCheck {
            state,
            notice,
            action,
        }
    }

    /// Auto-continue's decision. Its own end replaces the session's: a session
    /// ending before it just reconnects, so only its end is announced.
    fn keeping(
        &self,
        mut keep: Keep,
        state: &VpnState,
        now: DateTime<Utc>,
    ) -> (Option<VpnNotice>, Option<VpnAction>) {
        let up = matches!(state, VpnState::Connected { .. });
        if now >= keep.until {
            *lock(&self.keep) = None;
            return (
                Some(VpnNotice::KeepEnded),
                up.then_some(VpnAction::Disconnect),
            );
        }
        let mut decision = (None, None);
        match state {
            VpnState::Connected { .. } => {
                let since = *keep.up_since.get_or_insert(now);
                if now - since >= Duration::minutes(KEEP_STABLE_MINUTES) {
                    keep.failures = 0;
                    keep.spent = 0;
                    keep.retry_at = None;
                }
                decision.0 = self.ending(keep.until, now);
            }
            VpnState::Disconnected | VpnState::Error(VpnError::Tunnel(_)) => {
                keep.up_since = None;
                if keep.retry_at.is_none_or(|at| now >= at) {
                    decision.1 = Some(VpnAction::Reconnect);
                }
            }
            VpnState::Error(error) if hopeless(error) => {
                *lock(&self.keep) = None;
                return (Some(VpnNotice::KeepStopped { code: error.code() }), None);
            }
            // A helper not answering says nothing about the tunnel.
            VpnState::Connecting | VpnState::Error(_) => {}
        }
        let mut current = lock(&self.keep);
        if current.is_some() {
            *current = Some(keep);
        }
        decision
    }

    /// Whether a reconnect's sign-in may still go on to start a tunnel: a
    /// disconnect meanwhile turned auto-continue off.
    pub fn keeps(&self) -> bool {
        lock(&self.keep).is_some()
    }

    /// A reconnect finished as `state`. A tunnel that came up still counts
    /// until it lasts: one that dies at once must not loop on the password.
    pub fn reconnected(&self, state: &VpnState, now: DateTime<Utc>) -> Option<VpnNotice> {
        let mut current = lock(&self.keep);
        let keep = current.as_mut()?;
        match state {
            VpnState::Error(error) if hopeless(error) => {
                let code = error.code();
                *current = None;
                return Some(VpnNotice::KeepStopped { code });
            }
            VpnState::Connected { .. } | VpnState::Error(VpnError::Tunnel(_)) => {
                keep.spent += 1;
                if keep.spent >= KEEP_MAX_SPENT {
                    *current = None;
                    return Some(VpnNotice::KeepStopped {
                        code: VpnError::Tunnel(String::new()).code(),
                    });
                }
            }
            // Offline or the school not answering: no password was accepted,
            // so this only waits, until the duration ends.
            _ => {}
        }
        if matches!(state, VpnState::Connected { .. }) {
            keep.up_since = Some(now);
        }
        keep.back_off(now);
        None
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

    /// The person's disconnect, or auto-continue's end: nothing reconnects.
    pub fn disconnect(&self) -> VpnState {
        *lock(&self.keep) = None;
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
