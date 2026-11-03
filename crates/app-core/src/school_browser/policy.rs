use super::{NavigationKind, NavigationRequest, NewWindowRequest};
use crate::{lifecycle::SchoolSite, school_auth::SchoolAuthProfile};
use url::Url;

/// The registrable domains of both accounts' school websites
/// (`adpt_isct::urls`). Every cookie under them belongs to a school session.
pub(crate) fn school_domains() -> impl Iterator<Item = &'static str> {
    adpt_isct::urls::SCIENCE_DOMAINS
        .iter()
        .chain(adpt_isct::urls::TOKYO_TECH_DOMAINS)
        .copied()
}

/// Whether a cookie or website-data domain (`.lms.s.isct.ac.jp`,
/// `isct.ac.jp`) is `registrable` or under it.
pub(crate) fn domain_under(domain: &str, registrable: &str) -> bool {
    let domain = domain.trim_start_matches('.').to_ascii_lowercase();
    domain == registrable || domain.ends_with(&format!(".{registrable}"))
}

/// A URL school automation may load: HTTPS to a site with an adapter
/// (`SchoolSite::for_url`) or a school identity provider, without embedded
/// credentials. Core's own school requests and page scripts are bounded by
/// this; what the person browses is not.
pub fn automation_target(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && (SchoolSite::for_url(raw).is_some() || SchoolAuthProfile::for_auth_host(host).is_some()))
    .then_some(url)
}

/// A URL Core fetches a school file from: HTTPS on the default port to a
/// host under the school domains, without embedded credentials. Wider than
/// `automation_target`: a page the person reads may link a file on
/// `www.isct.ac.jp`, which has no adapter but is still a school host, and
/// school hosts are reached only through a WebView or Core
/// (`constraint/school-network-two-stacks`). Every redirect hop is checked.
pub fn school_file_url(raw: &str) -> Option<Url> {
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            !host.starts_with('.')
                && school_domains().any(|school| host.ends_with(&format!(".{school}")))
        }))
    .then_some(url)
}

/// An automation target in the form that may be written to disk or compared
/// as a page identity: no fragment, and only bounded identifying query
/// parameters, so SSO tokens and session state never reach a restore record.
pub fn restore_record(raw: &str) -> Option<String> {
    let mut url = automation_target(raw)?;
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

/// Query parameters a Reload never sends again: what makes a GET act
/// (Moodle's `sesskey` and its `confirm`) and what carries a secret (tokens,
/// tickets, SAML messages). Matched without case.
const NOT_REOPENED: &[&str] = &[
    "sesskey",
    "confirm",
    "wstoken",
    "token",
    "privatetoken",
    "passport",
    "access_token",
    "refresh_token",
    "id_token",
    "code",
    "client_secret",
    "authorization_code",
    "logintoken",
    "ticket",
    "samlrequest",
    "samlresponse",
    "relaystate",
    "password",
];

/// The address the App's Reload opens again for a page the person visited:
/// the school page as it was, its fragment and the `NOT_REOPENED`
/// parameters dropped, so the page (a forum discussion's `d`, a search's
/// filters) is the same and nothing acts or leaks. Held in memory only;
/// what reaches the disk is `restore_record`.
pub(crate) fn reopen_url(raw: &str) -> Option<Url> {
    let mut url = automation_target(raw)?;
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| {
            !NOT_REOPENED
                .iter()
                .any(|denied| key.eq_ignore_ascii_case(denied))
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.len() != url.query_pairs().count() {
        url.set_query(None);
        if !kept.is_empty() {
            url.query_pairs_mut().extend_pairs(kept);
        }
    }
    Some(url)
}

/// An address the person may type or open: HTTP or HTTPS without embedded
/// credentials, query and fragment kept. This grants no automation authority
/// (`automation_target`), and loading it still goes through the rule table.
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

/// Why a navigation or new window was refused. Every refusal is logged.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DenyReason {
    /// The view (or its opener) is gone or was replaced.
    Closed,
    /// Plain HTTP the person did not ask for. Nothing relaxes ATS or
    /// cleartext rules; a link the person follows goes to the system browser.
    Cleartext,
    /// User name or password inside the URL.
    Credentials,
    /// A scheme no school view loads (`file:`, `javascript:`, top-level `data:`).
    Scheme,
    /// The engine was about to send an earlier form again.
    Resubmission,
    /// The opener already has as many popups as a view may hold.
    TooManyPopups,
}

/// The rule table's answer before the engine applies it to a view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Verdict {
    Allow,
    /// New windows only: a child of the opener.
    Popup,
    External,
    Deny(DenyReason),
    /// Cancel a back/forward resubmission and step past its history entry.
    SkipEntry,
    /// Cancel a reload resubmission and load the same URL with GET. Only a
    /// page's own `location.reload()` or the engine's (iOS reloads a view
    /// whose content process ended) gets here, never the App's Reload.
    ReloadAsGet,
}

enum Target {
    /// Documents that never reach the network and inherit their opener's
    /// origin: `about:blank`, `about:srcdoc` (editor frames, `window.open('')`),
    /// `blob:` and, below the main frame, `data:`. The one place such
    /// exceptions are listed for every shell.
    Local {
        data: bool,
    },
    Https,
    Cleartext,
    Credentials,
    Unsupported,
}

fn target(url: &url::Url) -> Target {
    match url.scheme() {
        "about" if matches!(url.path(), "blank" | "srcdoc") => Target::Local { data: false },
        "blob" => Target::Local { data: false },
        "data" => Target::Local { data: true },
        "https" | "http" if !url.username().is_empty() || url.password().is_some() => {
            Target::Credentials
        }
        "https" if url.host_str().is_some() => Target::Https,
        "http" if url.host_str().is_some() => Target::Cleartext,
        _ => Target::Unsupported,
    }
}

/// Whether the person asked for this load: a gesture, or a link or form the
/// engine attributes to them. `Unknown` (hosts that cannot tell) counts, so
/// an HTTP file opens in the browser instead of doing nothing.
fn user_action(kind: NavigationKind, gesture: bool) -> bool {
    gesture
        || matches!(
            kind,
            NavigationKind::Link | NavigationKind::FormSubmit | NavigationKind::Unknown
        )
}

/// The rule table for a frame navigation. Every school view is the
/// person's: Core never drives one on its own.
pub(crate) fn navigation_rule(request: &NavigationRequest) -> Verdict {
    if let NavigationKind::FormResubmit { from_history } = request.kind {
        return match (request.main_frame, from_history) {
            (true, true) => Verdict::SkipEntry,
            (true, false) => Verdict::ReloadAsGet,
            (false, _) => Verdict::Deny(DenyReason::Resubmission),
        };
    }
    match target(&request.url) {
        Target::Local { data: true } if request.main_frame => Verdict::Deny(DenyReason::Scheme),
        Target::Local { .. } | Target::Https => Verdict::Allow,
        Target::Credentials => Verdict::Deny(DenyReason::Credentials),
        Target::Unsupported => Verdict::Deny(DenyReason::Scheme),
        Target::Cleartext => {
            if request.main_frame && user_action(request.kind, request.user_gesture) {
                Verdict::External
            } else {
                Verdict::Deny(DenyReason::Cleartext)
            }
        }
    }
}

/// The rule table for a window a page opens (`window.open`, `target=_blank`).
pub(crate) fn new_window_rule(request: &NewWindowRequest) -> Verdict {
    match target(&request.url) {
        Target::Local { data: true } => Verdict::Deny(DenyReason::Scheme),
        Target::Local { .. } | Target::Https => Verdict::Popup,
        Target::Credentials => Verdict::Deny(DenyReason::Credentials),
        Target::Unsupported => Verdict::Deny(DenyReason::Scheme),
        Target::Cleartext if user_action(request.kind, request.user_gesture) => Verdict::External,
        Target::Cleartext => Verdict::Deny(DenyReason::Cleartext),
    }
}

/// Whether a school view may save a file: anything the person can browse,
/// files a page builds itself (`blob:`, `data:`) included.
pub fn download_permitted(raw: &str) -> bool {
    browsing_url(raw).is_some()
        || Url::parse(raw).is_ok_and(|url| matches!(url.scheme(), "blob" | "data"))
}

/// Whether a school attachment (`Content-Disposition: attachment`) of
/// `mime` is shown in its view instead of saved: a PDF, a raster image,
/// audio or video, none of which runs script. Everything else is saved.
/// HTML, SVG and XML shown in place would run as a page of the school's
/// origin, which the attachment header exists to prevent; notebooks, JSON
/// and text read badly as a page anyway.
pub fn attachment_shown_in_place(mime: &str) -> bool {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    mime == "application/pdf"
        || (mime.starts_with("image/") && !mime.contains("svg") && !mime.contains("xml"))
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
}

#[cfg(test)]
#[path = "../tests/school_browser__policy.rs"]
mod tests;
