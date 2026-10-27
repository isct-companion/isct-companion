//! School WebView cookies for Rust HTTP, independent of native webview
//! handles: which origins may be read and when the store holds a session.
//! Getting a session when it is missing is the school browser's job; a read
//! here answers `SignInRequired` and the caller's transport asks for one.

use crate::net::{cookies, Host, NetError};
use url::Url;

pub const LMS_HOME: &str = "https://lms.s.isct.ac.jp/2025/my/";

/// The shell's school WebView cookie store.
pub trait SchoolCookiePort: Send + Sync {
    /// The name/value pairs the school store sends to `url`.
    fn cookies(&self, url: &Url) -> Result<Vec<(String, String)>, NetError>;
    /// Stores Set-Cookie values already vetted by [`cookies::storable`].
    fn set_cookies(&self, url: &Url, set_cookie: &[String]) -> Result<(), NetError>;
}

/// The cookie that marks a live session on `host`, when one is known. Moodle
/// and 教務Web set their own; GetAccess has no single marker, so any cookie
/// counts.
fn session_cookie(host: Host) -> Option<&'static str> {
    match host {
        Host::Moodle => Some("MoodleSession"),
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

/// The `Cookie` header for a school request, or `SignInRequired` when the
/// store holds no session for its host.
pub fn cookie_header(port: &dyn SchoolCookiePort, url: &Url) -> Result<String, NetError> {
    match Host::of(url) {
        Some(host @ (Host::Moodle | Host::Kyomu | Host::GetAccess | Host::ScienceAccount)) => {
            session_header(host, &port.cookies(url)?)
        }
        _ => Err(NetError::Invalid("not a school sign-in origin".into())),
    }
}

pub fn store_set_cookies(
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

#[cfg(test)]
#[path = "tests/lms_session.rs"]
mod tests;
