//! What the campus VPN takes from the F5 APM session: the cookie openconnect
//! is given. The APM session itself is signed in by `school_session` over
//! HTTP; when it ends, openconnect reports through the helper.

use vpn_protocol::{CookieError, SessionCookie};

pub const CAMPUS_APM_HOST: &str = "apm.nap.gsic.titech.ac.jp";

/// The helper's cookie from the APM session cookies. `MRHSession` is the
/// session; `F5_ST` (its timers) goes along when the APM set it.
pub fn session_cookie(
    mrh_session: &str,
    f5_st: Option<&str>,
) -> Result<SessionCookie, CookieError> {
    match f5_st.map(str::trim).filter(|value| !value.is_empty()) {
        Some(f5_st) => SessionCookie::new(format!("MRHSession={mrh_session}; F5_ST={f5_st}")),
        None => SessionCookie::new(format!("MRHSession={mrh_session}")),
    }
}

#[cfg(test)]
#[path = "../tests/vpn__f5.rs"]
mod tests;
