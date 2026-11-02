//! Which `Set-Cookie` values a school response may plant in one of Core's
//! cookie jars (`crate::school_session`).

use super::Host;
use url::Url;

/// The cookies a school response from `url` may store. A cookie without
/// `Domain` is host-only and always kept. A `Domain` must cover the response
/// host and name at least a site (`isct.ac.jp`), never a suffix shared by
/// unrelated sites (`ac.jp`). Anything else, and every cookie from a
/// non-school host, is dropped: a jar must not take cookies a browser would
/// refuse.
pub fn storable(url: &Url, set_cookie: &[String]) -> Vec<String> {
    let Some(host) = Host::of(url)
        .filter(|host| *host != Host::IsctData)
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
