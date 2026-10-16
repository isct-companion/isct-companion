use crate::school_auth::SchoolAuthProfile;
use url::Url;

/// Every official host a school page may navigate to.
///
/// The list is grouped by the campus that publishes the system, following the
/// student site's 学修関連システム page. An origin in this list may be loaded in a
/// school WebView; membership does not imply an adapter, a start page or a
/// `SchoolSite` mapping.
pub const SCHOOL_HOSTS: &[&str] = &[
    // 理工学系 (Science and engineering): portal, LMS, syllabus, 教務, library,
    // TSUBAME, mail and the former Tokyo Tech site that still serves 学修案内.
    "isct.ex-tic.com",
    "portal.isct.ac.jp",
    "portal.titech.ac.jp",
    "lms.s.isct.ac.jp",
    "t2schola.titech.ac.jp",
    "syllabus.s.isct.ac.jp",
    "portal.nap.gsic.titech.ac.jp",
    "rp.nap.gsic.titech.ac.jp",
    "mailv4.m.titech.ac.jp",
    "kyomu0.gakumu.titech.ac.jp",
    "request.libra.titech.ac.jp",
    "portal.t4.gsic.titech.ac.jp",
    "www.titech.ac.jp",
    // 医歯学系 (medicine and dentistry): Dream Campus and Web Class. Allowlisted
    // so a school page can hand off to them, but NOT verified: no adapter, no
    // `SchoolSite` mapping and no live account has been exercised. Do not add
    // extraction or start-page behaviour without that verification.
    "yushima2.tmd.ac.jp",
    "lib02.tmd.ac.jp",
];

pub fn sanitize_school_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if !url
        .host_str()
        .is_some_and(|host| SCHOOL_HOSTS.contains(&host))
    {
        return None;
    }
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, value)| {
            let identifier = value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            (matches!(key.as_ref(), "id" | "course" | "page" | "section") && identifier)
                || (key == "jwc"
                    && value.len() == 9
                    && value.bytes().all(|byte| byte.is_ascii_digit()))
                || (key == "lang" && matches!(value.as_ref(), "ja" | "en"))
                || (key == "plang" && matches!(value.as_ref(), "jpn" | "eng"))
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !kept.is_empty() {
        url.query_pairs_mut().extend_pairs(kept);
    }
    Some(url.to_string())
}

/// Whether a loaded or fetched URL is a school sign-in page: an identity
/// provider's credential step, or a service's own login or SSO hand-off route
/// such as the LMS `/login/` and `/auth/` pages. This is the single sign-in
/// detector shared by the browser engine and the LMS downloaders.
pub fn is_login_url(url: &Url) -> bool {
    is_identity_login(url)
        || url.path_segments().is_some_and(|mut segments| {
            segments.any(|segment| segment == "login" || segment == "auth")
        })
}

/// An identity-provider page where saved credentials can be entered.
pub(crate) fn is_identity_login(url: &Url) -> bool {
    url.host_str()
        .and_then(SchoolAuthProfile::for_auth_host)
        .is_some()
        && is_login_step(url)
}

pub(crate) fn is_login_step(url: &Url) -> bool {
    let path = url.path().trim_end_matches('/');
    match url.host_str() {
        Some("isct.ex-tic.com") => matches!(
            path,
            "/auth" | "/auth/session" | "/auth/session/second_factor"
        ),
        Some("portal.nap.gsic.titech.ac.jp" | "portal.titech.ac.jp") => {
            matches!(path, "" | "/portal.pl" | "/GetAccess/Login")
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "../tests/school_browser__policy.rs"]
mod tests;
