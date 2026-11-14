//! Wire format between IsctCompanion and the campus VPN privileged helper.
//!
//! One JSON object per line over the helper's local socket: a Unix socket on
//! macOS, a named pipe on Windows. Both the app-core client side and the
//! privileged daemon depend on this crate alone, so the two ends cannot drift
//! apart and the daemon never links the app's domain code.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::Ipv4Addr;

/// The helper this app bundles. An installed helper that reports less (or
/// nothing, as the first ones did) is replaced before the next connect.
pub const HELPER_REVISION: u32 = 5;

/// Socket the daemon listens on; the installer creates it as root:staff 0660.
#[cfg(unix)]
pub const SOCKET_PATH: &str = "/var/run/app.isctcompanion.vpnhelper.sock";

/// Pipe the Windows service listens on; its DACL admits the interactive user
/// and refuses remote clients.
#[cfg(windows)]
pub const PIPE_PATH: &str = r"\\.\pipe\app.isctcompanion.vpnhelper";

/// The Windows service the installer registers; the app asks the service
/// manager for it to tell a missing helper from a stopped one.
#[cfg(windows)]
pub const SERVICE_NAME: &str = "IsctCompanionVpnHelper";

/// The `Cookie:` header openconnect's F5 mode takes, e.g.
/// `MRHSession=…; F5_ST=…`. It is a live login: it is never logged, never put
/// in argv, and `Debug` prints only a placeholder.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SessionCookie(String);

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CookieError {
    #[error("session cookie is empty")]
    Empty,
    /// Line breaks or NUL would split the request on the wire or the helper's
    /// stdin, so they are refused rather than escaped.
    #[error("session cookie contains a control character")]
    ControlCharacter,
}

impl SessionCookie {
    pub fn new(value: impl Into<String>) -> Result<Self, CookieError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(CookieError::Empty);
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(CookieError::ControlCharacter);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SessionCookie {
    type Error = CookieError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SessionCookie> for String {
    fn from(cookie: SessionCookie) -> Self {
        cookie.0
    }
}

impl fmt::Debug for SessionCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionCookie(<redacted>)")
    }
}

/// The only operations the helper accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    /// Starts a tunnel, which the helper ends itself once it carries no
    /// traffic for a while: the school asks for connecting on demand and
    /// disconnecting as soon as possible.
    Start {
        cookie: SessionCookie,
    },
    Stop,
    Status,
    /// Holds the campus routes on the helper's standby interface for as long
    /// as this connection stays open, and reports on it as `StandbyEvent`
    /// lines. The connection closing, the app quitting or crashing with it,
    /// takes the routes away. `bypass` are campus addresses the sign-in and
    /// the app itself reach directly, which standby must not capture.
    Standby {
        #[serde(default)]
        bypass: Vec<Ipv4Addr>,
    },
}

impl Request {
    /// Parse one request line. An unknown `op` or a malformed cookie is an
    /// error, never a default.
    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line.trim())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HelperState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub state: HelperState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ifname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// When the tunnel's session ends (Unix seconds), as openconnect reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
    /// The answering helper's `HELPER_REVISION`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    /// Why the helper ended the last tunnel itself, until the next start or
    /// stop. `None` when nothing has, or the person stopped it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ending>,
}

/// A tunnel the helper ended on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ending {
    /// No traffic and no open connection through it for the idle time.
    Idle,
    /// No real traffic for half an hour, whatever connections it still held:
    /// a forgotten session holding one of the school's few VPN ports.
    Unused,
}

impl Reply {
    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            state: HelperState::Error,
            detail: Some(detail.into()),
            address: None,
            ifname: None,
            pid: None,
            expires: None,
            revision: Some(HELPER_REVISION),
            ended: None,
        }
    }

    pub fn disconnected() -> Self {
        Self {
            ok: true,
            state: HelperState::Disconnected,
            detail: None,
            address: None,
            ifname: None,
            pid: None,
            expires: None,
            revision: Some(HELPER_REVISION),
            ended: None,
        }
    }
}

/// One line the helper writes on a standby connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum StandbyEvent {
    /// Standby is up, sent every `STANDBY_HEARTBEAT_SECS` too: a write that
    /// fails is how the helper learns the app has gone. `tunnel` is whether a
    /// tunnel carries the campus routes instead of standby.
    Standby { tunnel: bool },
    /// Something used the campus network while no tunnel was up.
    Demand,
    /// Standby could not be set up or ended; the connection closes after it.
    Failed { detail: String },
}

impl StandbyEvent {
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// How often the helper writes on a standby connection. The app treats a
/// connection silent for several of these as a helper that went away.
pub const STANDBY_HEARTBEAT_SECS: u64 = 2;

#[cfg(test)]
#[path = "tests/protocol.rs"]
mod tests;
