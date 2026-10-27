//! The school WebView's cookie store as seen by Rust HTTP: the one port every
//! signed-in request (LMS, 教務Web, course files) reads its session from.

use super::{BoxFuture, Host, NetError};
use url::Url;

pub trait SessionCookies: Send + Sync {
    /// Cookie header for `url` from the SCHOOL WebView data store
    /// (desktop SCHOOL_DATA_STORE / iOS default store / Android CookieManager).
    /// Err(SignInRequired) when no session cookie exists.
    fn cookie_header<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<String, NetError>>;
    /// Write Set-Cookie values from a Rust response back to the same store.
    fn store_set_cookies<'a>(
        &'a self,
        url: &'a Url,
        set_cookie: &'a [String],
    ) -> BoxFuture<'a, Result<(), NetError>>;
}

/// The cookies a school response from `url` may write into the WebView store.
/// A cookie without `Domain` is host-only and always kept. A `Domain` must
/// cover the response host and name at least a site (`isct.ac.jp`), never a
/// suffix shared by unrelated sites (`ac.jp`). Anything else, and every cookie
/// from a non-school host, is dropped: Rust responses must not plant cookies
/// the browser itself would refuse.
pub fn storable(url: &Url, set_cookie: &[String]) -> Vec<String> {
    let Some(host) = Host::of(url)
        .filter(|host| {
            matches!(
                host,
                Host::Moodle | Host::Kyomu | Host::GetAccess | Host::ScienceAccount
            )
        })
        .map(Host::name)
    else {
        return Vec::new();
    };
    set_cookie
        .iter()
        .filter(|value| {
            let Some((name, _)) = value
                .split(';')
                .next()
                .and_then(|pair| pair.split_once('='))
            else {
                return false;
            };
            if name.trim().is_empty() {
                return false;
            }
            match domain_attribute(value) {
                None => true,
                Some(domain) => {
                    domain.split('.').filter(|label| !label.is_empty()).count() >= 3
                        && (host == domain || host.ends_with(&format!(".{domain}")))
                }
            }
        })
        .cloned()
        .collect()
}

/// The last `Domain` attribute, lowercased and without its leading dot.
fn domain_attribute(set_cookie: &str) -> Option<String> {
    set_cookie
        .split(';')
        .skip(1)
        .filter_map(|attribute| attribute.split_once('='))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("domain"))
        .map(|(_, value)| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .last()
        .filter(|domain| !domain.is_empty())
}

/// `name=value; …` for a request, from the pairs a store holds for its URL.
pub fn header<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}
