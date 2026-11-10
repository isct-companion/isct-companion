//! One cookie jar with a browser's rules (domain, path, expiry), over
//! `cookie_store`. A sign-in attempt, each site instance and nothing else
//! has one; they never share.

use crate::net::cookies::storable;
use cookie_store::CookieStore;
use std::sync::{Mutex, PoisonError};
use url::Url;

pub(crate) struct Jar(CookieStore);

impl Default for Jar {
    fn default() -> Self {
        Self(CookieStore::new())
    }
}

impl Jar {
    /// The `Cookie` header a browser would send to `url`.
    pub(crate) fn header(&self, url: &Url) -> Option<String> {
        let pairs: Vec<String> = self
            .0
            .get_request_values(url)
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }

    /// Takes the `Set-Cookie` values of a response from `url`, only those a
    /// school response may set (`net::cookies::storable`).
    pub(crate) fn store(&mut self, url: &Url, set_cookie: &[String]) {
        for line in storable(url, set_cookie) {
            // A value the store refuses (a foreign domain, a bad path) is
            // what a browser would drop as well.
            let _ = self.0.parse(&line, url);
        }
    }

    /// Whether a non-empty `name` would be sent to `url`.
    pub(crate) fn has(&self, url: &Url, name: &str) -> bool {
        self.0
            .get_request_values(url)
            .any(|(found, value)| found == name && !value.is_empty())
    }

    /// Every cookie, session cookies included: a school session is mostly
    /// session cookies, which `cookie_store`'s plain `save` leaves out.
    pub(crate) fn save(&self) -> Option<String> {
        let mut out = Vec::new();
        cookie_store::serde::json::save_incl_expired_and_nonpersistent(&self.0, &mut out).ok()?;
        String::from_utf8(out).ok()
    }

    /// A saved jar, without what expired since it was saved.
    pub(crate) fn load(saved: &str) -> Option<Self> {
        cookie_store::serde::json::load(saved.as_bytes())
            .ok()
            .map(Self)
    }
}

/// Where a walk through the school's pages reads and writes its cookies.
pub(crate) trait Cookies: Send + Sync {
    fn header(&self, url: &Url) -> Option<String>;
    fn store(&self, url: &Url, set_cookie: &[String]);
}

impl Cookies for Mutex<Jar> {
    fn header(&self, url: &Url) -> Option<String> {
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .header(url)
    }

    fn store(&self, url: &Url, set_cookie: &[String]) {
        self.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .store(url, set_cookie);
    }
}
