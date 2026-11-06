//! Walking the school's pages the way a browser does: GET with redirects
//! followed inside the school hosts, form posts, cookies from one jar. The
//! sign-in flows and every site's SSO run on it.
//!
//! Logs carry the host, the path, the status and page markers only: never a
//! query string, a form body, a cookie or a SAML parameter.

use super::{jar::Cookies, Transport, WebCookie};
use crate::net::{Body, Host, Limits, NetError, NetRequest, Response};
use reqwest::header::{ACCEPT, COOKIE, ORIGIN, REFERER, USER_AGENT};
use reqwest::Method;
use std::sync::Mutex;
use url::Url;

/// Redirects one GET may follow. The SAML chain between LMS and Extic is five
/// or six hops; 教務Web's is three.
const MAX_HOPS: usize = 20;

/// A page a walk arrived at.
#[derive(Clone, Debug)]
pub(super) struct Page {
    pub(super) url: Url,
    pub(super) html: String,
}

/// What a form post answered: a page, or the redirect it sent, which the
/// caller may judge (a rejection) before following it.
pub(super) enum Step {
    Page(Page),
    Redirect(Url),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WalkError {
    /// A status the step did not expect.
    Http(u16),
    /// The school sent the walk somewhere it does not go.
    Off(&'static str),
    Net(NetError),
}

impl From<NetError> for WalkError {
    fn from(error: NetError) -> Self {
        Self::Net(error)
    }
}

impl std::fmt::Display for WalkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(status) => write!(formatter, "the school answered HTTP {status}"),
            Self::Off(what) => formatter.write_str(what),
            Self::Net(error) => write!(formatter, "{error}"),
        }
    }
}

pub(super) struct Walk<'a> {
    transport: &'a dyn Transport,
    cookies: &'a dyn Cookies,
    /// Identity hosts whose `Set-Cookie` values are kept as they arrive,
    /// with the URL each came from: the identity snapshot.
    capture: Option<(&'a [Host], &'a Mutex<Vec<WebCookie>>)>,
    /// Sent instead of Core's own agent, for a site that admits browsers
    /// only.
    agent: Option<&'static str>,
}

impl<'a> Walk<'a> {
    pub(super) fn new(transport: &'a dyn Transport, cookies: &'a dyn Cookies) -> Self {
        Self {
            transport,
            cookies,
            capture: None,
            agent: None,
        }
    }

    pub(super) fn browser(mut self) -> Self {
        self.agent = Some(crate::net::BROWSER_USER_AGENT);
        self
    }

    pub(super) fn capturing(mut self, hosts: &'a [Host], into: &'a Mutex<Vec<WebCookie>>) -> Self {
        self.capture = Some((hosts, into));
        self
    }

    /// GET `url`, following redirects inside the school hosts.
    pub(super) async fn get(
        &self,
        mut url: Url,
        mut referer: Option<Url>,
    ) -> Result<Page, WalkError> {
        for _ in 0..MAX_HOPS {
            let response = self
                .send(Method::GET, &url, referer.as_ref(), None, false)
                .await?;
            match response.status {
                200..=299 => {
                    let html = response.text()?;
                    log::info!(
                        target: "app_core::school_session",
                        "  page {} markers={}",
                        loggable(&url),
                        markers_of(&html)
                    );
                    return Ok(Page { url, html });
                }
                300..=399 => {
                    let next = next_hop(&url, response.location().as_deref())?;
                    referer = Some(url);
                    url = next;
                }
                // A maintenance page may come with an error status: it is the
                // page, for the caller's maintenance check.
                status => {
                    return match response.text() {
                        Ok(html) if adpt_isct::maintenance_notice(&url, &html).is_some() => {
                            Ok(Page { url, html })
                        }
                        _ => Err(WalkError::Http(status)),
                    };
                }
            }
        }
        Err(WalkError::Off("too many redirects"))
    }

    /// POSTs `fields` to `action`; `xhr` for Extic's remote forms.
    pub(super) async fn post(
        &self,
        action: &Url,
        fields: &[(String, String)],
        referer: &Url,
        xhr: bool,
    ) -> Result<Step, WalkError> {
        let response = self
            .send(Method::POST, action, Some(referer), Some(fields), xhr)
            .await?;
        match response.status {
            200..=299 => Ok(Step::Page(Page {
                url: action.clone(),
                html: response.text()?,
            })),
            300..=399 => Ok(Step::Redirect(next_hop(
                action,
                response.location().as_deref(),
            )?)),
            status => match response.text() {
                Ok(html) if adpt_isct::maintenance_notice(action, &html).is_some() => {
                    Ok(Step::Page(Page {
                        url: action.clone(),
                        html,
                    }))
                }
                _ => Err(WalkError::Http(status)),
            },
        }
    }

    /// The page a step stands for: a redirect is followed.
    pub(super) async fn follow(&self, step: Step, referer: &Url) -> Result<Page, WalkError> {
        match step {
            Step::Page(page) => Ok(page),
            Step::Redirect(to) => self.get(to, Some(referer.clone())).await,
        }
    }

    async fn send(
        &self,
        method: Method,
        url: &Url,
        referer: Option<&Url>,
        form: Option<&[(String, String)]>,
        xhr: bool,
    ) -> Result<Response, WalkError> {
        let host = Host::of(url)
            .filter(|host| *host != Host::IsctData)
            .ok_or(WalkError::Off("the walk left the school hosts"))?;
        let mut headers = Vec::new();
        if let Some(cookie) = self.cookies.header(url) {
            headers.push((COOKIE, cookie));
        }
        if let Some(referer) = referer {
            headers.push((REFERER, referer.to_string()));
        }
        if let Some(agent) = self.agent {
            headers.push((USER_AGENT, agent.to_owned()));
        }
        if method == Method::POST {
            headers.push((
                ORIGIN,
                format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default()),
            ));
        }
        if xhr {
            // Extic's sign-in forms answer through Rails' remote form path.
            headers.push((
                reqwest::header::HeaderName::from_static("x-requested-with"),
                "XMLHttpRequest".to_owned(),
            ));
            headers.push((
                ACCEPT,
                "text/javascript, application/javascript, */*; q=0.01".to_owned(),
            ));
        }
        let body = match form {
            Some(fields) => Body::Form(fields.to_vec()),
            None => Body::None,
        };
        let response = self
            .transport
            .send(NetRequest {
                host,
                method: method.clone(),
                url: url.clone(),
                headers,
                body,
                limits: Limits::page(),
            })
            .await?;
        log::info!(
            target: "app_core::school_session",
            "{method} {} -> {}{}",
            loggable(url),
            response.status,
            response
                .location()
                .and_then(|location| url.join(&location).ok())
                .map(|location| format!(" -> {}", loggable(&location)))
                .unwrap_or_default()
        );
        let set_cookies = response.set_cookies();
        if !set_cookies.is_empty() {
            self.cookies.store(url, &set_cookies);
            if let Some((hosts, into)) = self.capture {
                if hosts.contains(&host) {
                    let mut captured = into
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    captured.extend(set_cookies.iter().map(|line| WebCookie {
                        url: url.to_string(),
                        set_cookie: line.clone(),
                    }));
                }
            }
        }
        Ok(response)
    }
}

/// Host and path only.
pub(super) fn loggable(url: &Url) -> String {
    format!("{}{}", url.host_str().unwrap_or("-"), url.path())
}

/// The few markers that say how a page continues, without logging its body.
fn markers_of(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut found = Vec::new();
    for (marker, needle) in [
        ("refresh", "http-equiv=\"refresh\""),
        ("saml", "samlresponse"),
        ("login-form", "form id=\"login\""),
        ("totp", "totp-form"),
        ("portal-menu", "resourcelist"),
        ("logout", "login/logout.php"),
    ] {
        if lower.contains(needle) {
            found.push(marker);
        }
    }
    found.join(",")
}

/// The next address of a redirect: https on a school host, or one of
/// GetAccess's own rejection pages (`is_rejection`), which the caller judges.
fn next_hop(from: &Url, location: Option<&str>) -> Result<Url, WalkError> {
    let location = location
        .map(str::trim)
        .filter(|location| !location.is_empty())
        .ok_or(WalkError::Off("a redirect named no address"))?;
    let next = from
        .join(location)
        .map_err(|_| WalkError::Off("a redirect named an unreadable address"))?;
    let school = Host::of(&next).is_some_and(|host| host != Host::IsctData);
    if next.scheme() != "https" || (!school && !is_rejection(&next)) {
        return Err(WalkError::Off(
            "the walk was redirected away from the school",
        ));
    }
    Ok(next)
}

/// GetAccess's own rejection answers: the portal's top page (not a school
/// host) and the `Message=fail` redirect the form's script would have sent.
pub(super) fn is_rejection(url: &Url) -> bool {
    if url.host_str() == Some("portal.titech.ac.jp") {
        return true;
    }
    url.query()
        .is_some_and(|query| query.to_ascii_lowercase().contains("message=fail"))
        || url.path().eq_ignore_ascii_case("/passwordfail.html")
}

/// Where a SAML hand-off may be posted: the LMS's assertion consumer only.
pub(super) fn is_lms_acs(url: &Url) -> bool {
    Host::of(url) == Some(Host::Moodle) && url.path().contains("/auth/saml2/sp/saml2-acs.php")
}
