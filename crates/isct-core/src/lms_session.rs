//! School WebView cookies for Rust HTTP, independent of native webview
//! handles: which origins may be read, when a session exists, and the LMS
//! readiness wait that lets a hidden view finish the LMS sign-in.

use crate::net::{cookies, Host, NetError};
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const SCHOOL_TIMEOUT: Duration = Duration::from_secs(180);
const LMS_ATTEMPTS: usize = 2;
const LMS_POLLS_PER_ATTEMPT: usize = 100;
const RELOAD_DELAY: Duration = Duration::from_secs(5);
pub const LMS_HOME: &str = "https://lms.s.isct.ac.jp/2025/my/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Pending,
    Active,
    Failed,
}

/// The shell's school WebView store and the hidden LMS view.
pub trait SchoolCookiePort: Send + Sync {
    fn school_state(&self) -> Result<SessionState, NetError>;
    fn request_school_session(&self) -> Result<(), NetError>;
    fn ensure_lms_view(&self) -> Result<(), NetError>;
    /// The LMS view shows an LMS page and the store holds a MoodleSession.
    fn lms_ready(&self) -> Result<bool, NetError>;
    fn navigate_lms_home(&self) -> Result<(), NetError>;
    /// The name/value pairs the school store sends to `url`.
    fn cookies(&self, url: &Url) -> Result<Vec<(String, String)>, NetError>;
    /// Stores Set-Cookie values already vetted by [`cookies::storable`].
    fn set_cookies(&self, url: &Url, set_cookie: &[String]) -> Result<(), NetError>;
}

/// The cookie that marks a live session on `host`, when one is known. 教務Web
/// is ASP.NET; GetAccess has no single marker, so any cookie counts.
fn session_cookie(host: Host) -> Option<&'static str> {
    match host {
        Host::Kyomu => Some("ASP.NET_SessionId"),
        _ => None,
    }
}

fn session_header(host: Host, pairs: &[(String, String)]) -> Result<String, NetError> {
    let signed_in = match session_cookie(host) {
        Some(marker) => pairs
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case(marker) && !value.is_empty()),
        None => !pairs.is_empty(),
    };
    if signed_in {
        Ok(cookies::header(
            pairs
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str())),
        ))
    } else {
        Err(NetError::SignInRequired)
    }
}

/// Serializes the LMS readiness wait: one caller drives the hidden view at a
/// time. Other origins are read directly; their sign-in is the WebView SSO
/// that `net::SignInGate` starts.
#[derive(Default)]
pub struct LmsSession {
    gate: Mutex<()>,
}

impl LmsSession {
    pub async fn cookie_header(
        &self,
        port: &dyn SchoolCookiePort,
        url: &Url,
    ) -> Result<String, NetError> {
        match Host::of(url) {
            Some(Host::Moodle) => self.lms_cookie_header(port, url).await,
            Some(host @ (Host::Kyomu | Host::GetAccess)) => {
                session_header(host, &port.cookies(url)?)
            }
            _ => Err(NetError::Invalid("not a school sign-in origin".into())),
        }
    }

    pub fn store_set_cookies(
        &self,
        port: &dyn SchoolCookiePort,
        url: &Url,
        set_cookie: &[String],
    ) -> Result<(), NetError> {
        let storable = cookies::storable(url, set_cookie);
        if storable.is_empty() {
            return Ok(());
        }
        port.set_cookies(url, &storable)
    }

    async fn lms_cookie_header(
        &self,
        port: &dyn SchoolCookiePort,
        url: &Url,
    ) -> Result<String, NetError> {
        let _gate = self.gate.lock().await;
        self.wait_for_school(port).await?;
        port.ensure_lms_view()?;

        for attempt in 0..LMS_ATTEMPTS {
            for _ in 0..LMS_POLLS_PER_ATTEMPT {
                if port.lms_ready()? {
                    return session_header(Host::Moodle, &port.cookies(url)?);
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            if attempt == 0 {
                // The legacy recovery path treated this request as a hint; the
                // explicit navigation below remains the authoritative retry.
                let _ = port.request_school_session();
                tokio::time::sleep(RELOAD_DELAY).await;
                port.navigate_lms_home()?;
            }
        }
        Err(NetError::SignInRequired)
    }

    async fn wait_for_school(&self, port: &dyn SchoolCookiePort) -> Result<(), NetError> {
        if port.school_state()? == SessionState::Active {
            return Ok(());
        }
        port.request_school_session()?;
        tokio::time::timeout(SCHOOL_TIMEOUT, async {
            loop {
                match port.school_state()? {
                    SessionState::Active => return Ok(()),
                    SessionState::Failed => return Err(NetError::SignInRequired),
                    SessionState::Pending => tokio::time::sleep(POLL_INTERVAL).await,
                }
            }
        })
        .await
        .map_err(|_| NetError::SignInRequired)?
    }
}

#[cfg(test)]
#[path = "tests/lms_session.rs"]
mod tests;
