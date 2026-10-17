use crate::school_auth::SchoolAuthProfile;
use url::Url;

/// Official origins eligible for school automation and persisted restore URLs.
/// Interactive WebView browsing is governed separately by `browsing_url`.
///
/// The list is grouped by the campus that publishes the system, following the
/// student site's 学修関連システム page. An origin in this list may be loaded in a
/// school automation flow; membership does not imply an adapter, a start page or a
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
    // Services linked from the Tokyo Tech Portal resource menu, which open
    // in the portal's own view.
    "portfolio2.gakumu.titech.ac.jp",
    "healdb.gakumu.titech.ac.jp",
    "acc1.dx.titech.ac.jp",
    "vlmgr-42.edu2024.gsic.titech.ac.jp",
    "lsci.rpd.titech.ac.jp",
    "apm.nap.gsic.titech.ac.jp",
    "search-inside.star.titech.ac.jp",
    "signin.noc.titech.ac.jp",
    "idp-gakunin.nap.gsic.titech.ac.jp",
    "idp-saas.nap.gsic.titech.ac.jp",
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
            (url.host_str() == Some("kyomu0.gakumu.titech.ac.jp")
                && url.path().eq_ignore_ascii_case(crate::catalog::KYOMU_SEARCH_PATH)
                && match key.as_ref() {
                    "Quarters" | "Youbi" | "KaisiJigenS" | "KamokuCodeSeries" | "HisshuKbn" => value.len()<=128 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b','),
                    "Lang_kj" | "Lang_ke" => value == "True",
                    "TermUnit" => value.len() == 5 && value.bytes().all(|b| b.is_ascii_digit()) && matches!(value.as_bytes().last(),Some(b'1'|b'2')),
                    "_search" | "ZengakuSenmonAll" | "Suisho" | "Zengaku" | "Senmon" | "SE" | "MD" => matches!(value.as_ref(),"True"|"False"),
                    "ShinkokuNoS" | "Name" | "KyouinName" => value.chars().count() <= 200 && !value.chars().any(char::is_control),
                    "back" => value == "0",
                    _ => false,
                }) ||
            (matches!(key.as_ref(), "id" | "course" | "page" | "section") && identifier)
                // Keep only the draft editor action on the LMS assignment route.
                || (key == "action"
                    && value == "editsubmission"
                    && url.host_str() == Some("lms.s.isct.ac.jp")
                    && url.path().ends_with("/mod/assign/view.php"))
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

/// Interactive browsing preserves query/fragment state. This grants no login
/// or extraction authority; those continue to use `sanitize_school_url`.
pub fn browsing_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

pub fn browsing_target(
    raw: &str,
    fallback: crate::lifecycle::SchoolSite,
) -> Result<super::SchoolWebTarget, super::SchoolBrowserError> {
    let url = browsing_url(raw).ok_or_else(|| {
        super::SchoolBrowserError::InvalidInput(
            "Enter an HTTP or HTTPS URL without embedded credentials.".into(),
        )
    })?;
    Ok(super::SchoolWebTarget {
        site: crate::lifecycle::SchoolSite::for_url(&url).unwrap_or(fallback),
        url,
    })
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
