//! Dev builds only (`approach/mcp-dev-school-fetch`): an agent sends any
//! request to a school site and Core sends it with its own site session,
//! signing in when the session needs it. The agent names the site, the URL,
//! extra headers and the body; it never sees or sets a credential, a cookie
//! or a token, and identity provider pages are out of reach (only the LMS
//! and 教務Web hosts are). `school_fetch` reads (GET, HEAD) and recovers a
//! lost session once; `school_send` takes every other method, asks the
//! person on every call, and is never repeated after a lost session.
use super::registry::{Effect, Tool};
use super::{AgentError, Context};
use crate::net::{kyomu, Body, BoxFuture, Host, Limits, NetError, NetRequest, Response};
use crate::plan::AccountWait;
use crate::school_session::{Attempt, Blocked, OpError, Operation, RunError, Site};
use base64::Engine as _;
use reqwest::header::{HeaderName, LOCATION, SET_COOKIE};
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::time::Duration;
use url::Url;

/// Within the MCP adapter's own request timeout.
const CALL_DEADLINE: Duration = Duration::from_secs(55);
const MAX_REDIRECTS: usize = 10;
/// The largest answer read; beyond it the call fails.
const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Text answered inline, in bytes; the rest is cut.
const MAX_TEXT: usize = 1024 * 1024;
/// A binary answer inline as Base64; a larger one needs `saveAs`.
const MAX_INLINE_BINARY: usize = 1024 * 1024;

/// Headers Core sets itself or that would carry a credential.
const FORBIDDEN_HEADERS: [&str; 8] = [
    "cookie",
    "authorization",
    "proxy-authorization",
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
];

/// LMS pages out of reach, matched anywhere in the decoded path (the LMS
/// lives under a yearly prefix such as `/2025/`): sign-in, logout and the
/// token service (`/login/`), the page listing the person's web service
/// tokens, and the mobile app launch that mints one.
const LMS_BLOCKED: [&str; 3] = ["/login/", "/user/managetoken.php", "/admin/tool/mobile/"];
/// 教務Web's sign-in entry: it always goes through GetAccess, which a run
/// would read as a lost session and sign in again, then pause the account.
const KYOMU_BLOCKED: [&str; 1] = ["/titech/default.aspx"];
/// Where `saveAs` files go.
const SAVE_DIR: &str = "IsctCompanion-school-fetch";

#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum SiteArg {
    /// The LMS, lms.s.isct.ac.jp.
    Lms,
    /// 教務Web in Japanese, kyomu0.gakumu.titech.ac.jp.
    Kyomu,
    /// 教務Web in English: the same host, its own server session.
    KyomuEnglish,
}

impl SiteArg {
    fn site(self) -> Site {
        match self {
            Self::Lms => Site::Lms,
            Self::Kyomu => Site::Kyomu,
            Self::KyomuEnglish => Site::KyomuEnglish,
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct FetchArgs {
    site: SiteArg,
    /// An absolute https URL on the site's host.
    url: String,
    /// `GET` (the default) or `HEAD`.
    #[serde(default)]
    method: Option<String>,
    /// Extra request headers such as Referer or Accept. Cookie,
    /// Authorization, Host and the body's headers are Core's.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// Follow redirects that stay on the site's host (default true).
    #[serde(default = "yes")]
    follow_redirects: bool,
    /// A new file name (no folders) to write the body to instead of
    /// answering it inline; it goes in the system temp folder's
    /// `IsctCompanion-school-fetch/`, and the answer gives its path.
    #[serde(default)]
    save_as: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SendArgs {
    site: SiteArg,
    /// An absolute https URL on the site's host.
    url: String,
    /// `POST`, `PUT`, `PATCH` or `DELETE`.
    method: String,
    /// Extra request headers such as Referer or X-Requested-With. Cookie,
    /// Authorization, Host and the body's headers are Core's.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<BodyArg>,
    /// Follow redirects that stay on the site's host (default true); a
    /// 301, 302 or 303 continues as a GET, a 307 or 308 is answered as is.
    #[serde(default = "yes")]
    follow_redirects: bool,
    /// A new file name (no folders) to write the body to instead of
    /// answering it inline; it goes in the system temp folder's
    /// `IsctCompanion-school-fetch/`, and the answer gives its path.
    #[serde(default)]
    save_as: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum BodyArg {
    /// `application/x-www-form-urlencoded`, fields in order.
    Form { fields: Vec<Field> },
    /// `application/json`.
    Json { value: Value },
    /// Any text under its content type.
    #[serde(rename_all = "camelCase")]
    Text { content_type: String, text: String },
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Field {
    name: String,
    value: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Exchanged {
    status: u16,
    /// The URL that answered, after the redirects followed.
    url: String,
    /// Each redirect followed, in order.
    redirects: Vec<String>,
    /// Response headers in order, without Set-Cookie; a Location to a
    /// non-web scheme (an app launch carrying a token) is withheld.
    headers: Vec<HeaderLine>,
    body: BodyOut,
}

#[derive(Serialize, JsonSchema)]
struct HeaderLine {
    name: String,
    value: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum BodyOut {
    Empty,
    /// Decoded with the response's charset.
    Text {
        text: String,
        truncated: bool,
    },
    Base64 {
        data: String,
    },
    /// Written to `saveAs`.
    Saved {
        path: String,
        bytes: u64,
    },
    /// Binary and too large to answer inline: pass `saveAs`.
    TooLarge {
        bytes: u64,
    },
}

pub(super) struct SchoolFetch;

impl Tool for SchoolFetch {
    const NAME: &'static str = "school_fetch";
    const DESCRIPTION: &'static str = "Dev builds only. Send a GET or HEAD to the LMS or 教務Web as the signed-in person: Core adds its own session cookies, signs in when the session needs it and recovers a lost session once. Answers the status, the final URL, the redirects followed, the response headers (no Set-Cookie) and the raw body: text inline, binary as Base64 up to 1 MiB, or written to a temp file named by saveAs. Only lms.s.isct.ac.jp and kyomu0.gakumu.titech.ac.jp are reachable; sign-in pages, LMS token pages and logout are not. For other methods use school_send.";
    const EFFECT: Effect = Effect::Read;
    const OPEN_WORLD: bool = true;
    type Args = FetchArgs;
    type Output = Exchanged;

    fn run(cx: &Context<'_>, args: FetchArgs) -> Result<Exchanged, AgentError> {
        let method = match args
            .method
            .as_deref()
            .map(str::to_ascii_uppercase)
            .as_deref()
        {
            None | Some("GET") => Method::GET,
            Some("HEAD") => Method::HEAD,
            Some(other) => {
                return Err(
                    format!("school_fetch sends GET or HEAD; use school_send for {other}").into(),
                )
            }
        };
        let request = Prepared::new(args.site, &args.url, method, &args.headers, Body::None)?;
        exchange(cx, request, args.follow_redirects, args.save_as.as_deref())
    }
}

pub(super) struct SchoolSend;

impl Tool for SchoolSend {
    const NAME: &'static str = "school_send";
    const DESCRIPTION: &'static str = "Dev builds only. Send a POST, PUT, PATCH or DELETE to the LMS or 教務Web as the signed-in person, with a form, JSON or text body; the person confirms every call. This can change the person's data at the school (registration, submissions, settings): say what the request does before sending it. Core adds its own session cookies and signs in when needed, but never repeats the request: when the session turns out to have expired, the call fails and a school_fetch GET to the site renews it. Answers like school_fetch. Only lms.s.isct.ac.jp and kyomu0.gakumu.titech.ac.jp are reachable; sign-in pages, LMS token pages and logout are not.";
    const EFFECT: Effect = Effect::SchoolSend;
    const OPEN_WORLD: bool = true;
    type Args = SendArgs;
    type Output = Exchanged;

    fn run(cx: &Context<'_>, args: SendArgs) -> Result<Exchanged, AgentError> {
        let method = match args.method.to_ascii_uppercase().as_str() {
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            "DELETE" => Method::DELETE,
            other => {
                return Err(
                    format!("school_send sends POST, PUT, PATCH or DELETE, not {other}").into(),
                )
            }
        };
        let body = match args.body {
            None => Body::None,
            Some(BodyArg::Form { fields }) => {
                Body::Form(fields.into_iter().map(|f| (f.name, f.value)).collect())
            }
            Some(BodyArg::Json { value }) => Body::Json(value),
            Some(BodyArg::Text { content_type, text }) => Body::Bytes {
                content_type,
                bytes: text.into(),
            },
        };
        let request = Prepared::new(args.site, &args.url, method, &args.headers, body)?;
        exchange(cx, request, args.follow_redirects, args.save_as.as_deref())
    }
}

/// A checked request: on the site's host, outside the blocked pages, with
/// only the agent's own headers.
struct Prepared {
    site: Site,
    method: Method,
    url: Url,
    headers: Vec<(HeaderName, String)>,
    body: Body,
}

impl Prepared {
    fn new(
        site: SiteArg,
        url: &str,
        method: Method,
        headers: &BTreeMap<String, String>,
        body: Body,
    ) -> Result<Self, AgentError> {
        let site = site.site();
        let url = Url::parse(url).map_err(|error| format!("url: {error}"))?;
        if Host::of(&url) != Some(site.host()) {
            return Err(format!("url must be https on {}", site.host().name()).into());
        }
        if blocked(site, &url) {
            return Err(format!("{} is not reachable through this tool", url.path()).into());
        }
        let headers = headers
            .iter()
            .map(|(name, value)| {
                let lower = name.to_ascii_lowercase();
                if FORBIDDEN_HEADERS.contains(&lower.as_str()) {
                    return Err(AgentError::from(format!("header {name} is set by Core")));
                }
                let name = HeaderName::from_bytes(lower.as_bytes())
                    .map_err(|_| format!("header name {name} is invalid"))?;
                if value.contains(['\r', '\n']) {
                    return Err(format!("header {name} has a line break").into());
                }
                Ok((name, value.clone()))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            site,
            method,
            url,
            headers,
            body,
        })
    }
}

/// Whether `url` is out of reach: its path percent-decoded, lowercased and
/// with repeated slashes folded, as a server would read it.
fn blocked(site: Site, url: &Url) -> bool {
    let decoded = percent_encoding::percent_decode_str(url.path())
        .decode_utf8_lossy()
        .to_ascii_lowercase()
        .replace('\\', "/");
    let mut path = String::with_capacity(decoded.len());
    for character in decoded.chars() {
        if !(character == '/' && path.ends_with('/')) {
            path.push(character);
        }
    }
    let fragments: &[&str] = match site {
        Site::Lms => &LMS_BLOCKED,
        Site::Kyomu | Site::KyomuEnglish => &KYOMU_BLOCKED,
    };
    fragments.iter().any(|fragment| path.contains(fragment))
}

/// Runs the request on the site's session and shapes the answer.
fn exchange(
    cx: &Context<'_>,
    request: Prepared,
    follow: bool,
    save_as: Option<&str>,
) -> Result<Exchanged, AgentError> {
    let save_to = save_as.map(save_path).transpose()?;
    let sessions = cx.sessions;
    let site = request.site;
    let permit = sessions.permit(site.profile()).ok_or_else(|| {
        AgentError::Unavailable(
            "the school account cannot be used now: it is signed out, not set up or switching"
                .into(),
        )
    })?;
    let runtime = tokio::runtime::Handle::try_current()
        .map_err(|_| AgentError::Compatibility("no async runtime for school requests".into()))?;
    let replayable = matches!(request.method, Method::GET | Method::HEAD);
    let mut operation = Exchange {
        request: &request,
        follow,
        replayable,
    };
    let (response, redirects) = runtime
        .block_on(async {
            let deadline = tokio::time::Instant::now() + CALL_DEADLINE;
            sessions.run(site, &permit, deadline, &mut operation).await
        })
        .map_err(run_error)?;
    let body = body(&response, save_to)?;
    Ok(Exchanged {
        status: response.status,
        url: response.url.to_string(),
        redirects,
        headers: header_lines(&response),
        body,
    })
}

/// `name` in the save folder: a plain file name, not there yet.
fn save_path(name: &str) -> Result<std::path::PathBuf, AgentError> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err("saveAs must be a plain file name".into());
    }
    let path = std::env::temp_dir().join(SAVE_DIR).join(name);
    if path.exists() {
        return Err(format!("{} already exists", path.display()).into());
    }
    Ok(path)
}

struct Exchange<'a> {
    request: &'a Prepared,
    follow: bool,
    replayable: bool,
}

enum ExchangeError {
    /// The site answered with its sign-in: the session had expired.
    SignedOut {
        /// The request is a GET or HEAD, so the run may send it again.
        replayable: bool,
        /// The site answered the request itself and the sign-in came on a
        /// redirect followed after it: a write may have taken effect.
        answered: bool,
    },
    Net(NetError),
}

impl OpError for ExchangeError {
    fn signed_out(&self) -> bool {
        match self {
            Self::SignedOut { replayable, .. } => *replayable,
            Self::Net(error) => error.signed_out(),
        }
    }
}

impl Operation for Exchange<'_> {
    type Output = (Response, Vec<String>);
    type Error = ExchangeError;

    fn attempt(&mut self, attempt: Attempt) -> BoxFuture<'_, Result<Self::Output, ExchangeError>> {
        Box::pin(async move {
            let request = self.request;
            let host = request.site.host();
            let mut method = request.method.clone();
            let mut body = request.body.clone();
            let mut url = request.url.clone();
            let mut redirects = Vec::new();
            loop {
                let response = attempt
                    .send(NetRequest {
                        host,
                        method: method.clone(),
                        url: url.clone(),
                        headers: request.headers.clone(),
                        body: body.clone(),
                        limits: Limits {
                            max_bytes: MAX_BYTES,
                            deadline: CALL_DEADLINE,
                        },
                    })
                    .await
                    .map_err(ExchangeError::Net)?;
                let signed_out = ExchangeError::SignedOut {
                    replayable: self.replayable,
                    answered: !redirects.is_empty(),
                };
                if !(300..400).contains(&response.status) {
                    if shows_sign_in(request.site, &url, &response) {
                        return Err(signed_out);
                    }
                    return Ok((response, redirects));
                }
                let location = response.location();
                let next = location
                    .as_deref()
                    .and_then(|location| url.join(location.trim()).ok());
                if next.as_ref().is_some_and(|next| {
                    sends_to_sign_in(request.site, response.status, location.as_deref(), next)
                }) {
                    return Err(signed_out);
                }
                let Some(next) = next.filter(|next| {
                    self.follow
                        && redirects.len() < MAX_REDIRECTS
                        && Host::of(next) == Some(host)
                        && !blocked(request.site, next)
                }) else {
                    return Ok((response, redirects));
                };
                match response.status {
                    301..=303 => {
                        if method != Method::HEAD {
                            method = Method::GET;
                        }
                        body = Body::None;
                    }
                    307 | 308 if matches!(method, Method::GET | Method::HEAD) => {}
                    _ => return Ok((response, redirects)),
                }
                redirects.push(next.to_string());
                url = next;
            }
        })
    }
}

/// A redirect that means the site's session is gone: 教務Web's own rule,
/// and for the LMS its login page or an identity provider. Not the LMS
/// readers' `sign_in_redirect`: they call fixed endpoints, where leaving the
/// LMS can only mean sign-in, but any LMS page may send a person to another
/// site (a URL resource, an external tool); taking that for a lost session
/// would sign in again, see the same redirect and pause the account.
fn sends_to_sign_in(site: Site, status: u16, location: Option<&str>, next: &Url) -> bool {
    match site {
        Site::Kyomu | Site::KyomuEnglish => {
            kyomu::classify(status, location) == kyomu::Reply::SessionExpired
        }
        Site::Lms => match Host::of(next) {
            Some(Host::Moodle) => next.path().contains("/login/"),
            Some(Host::ScienceAccount | Host::GetAccess | Host::GetAccessRouter) => true,
            _ => false,
        },
    }
}

/// A page that is 教務Web's sign-in, served with a 200 instead of a
/// redirect. The LMS always redirects to its login page.
fn shows_sign_in(site: Site, url: &Url, response: &Response) -> bool {
    matches!(site, Site::Kyomu | Site::KyomuEnglish)
        && response
            .content_type()
            .is_some_and(|content_type| content_type.to_ascii_lowercase().contains("html"))
        && response
            .text()
            .is_ok_and(|html| adpt_isct::kyomu::is_login_page(url, &html))
}

fn run_error(error: RunError<ExchangeError>) -> AgentError {
    let unavailable = |message: String| AgentError::Unavailable(message);
    match error {
        RunError::Op(ExchangeError::SignedOut {
            replayable: false,
            answered: true,
        }) => AgentError::Conflict(
            "the site answered the request with a redirect that led to its sign-in, so the request may have taken effect; check with school_fetch before sending it again"
                .into(),
        ),
        RunError::Op(ExchangeError::SignedOut {
            replayable: false, ..
        }) => AgentError::Conflict(
            "the site's session had expired, so the request was not sent again; a school_fetch GET to the site renews it"
                .into(),
        ),
        RunError::Op(ExchangeError::SignedOut { .. }) => {
            unavailable("the site still asked to sign in after a fresh session".into())
        }
        RunError::Op(ExchangeError::Net(error)) | RunError::Net(error) => {
            unavailable(format!("the request failed: {error}"))
        }
        RunError::Blocked(Blocked { wait, .. }) => unavailable(match wait {
            AccountWait::Person => {
                "the account needs the person: sign in or fix its credentials in the app".into()
            }
            AccountWait::Paused { until } => format!("sign-in is paused until {until}"),
        }),
        RunError::Offline => unavailable("the device is offline".into()),
        RunError::Cancelled => unavailable("the account changed while the request ran".into()),
        RunError::Deadline => unavailable("the school did not answer in time".into()),
    }
}

fn header_lines(response: &Response) -> Vec<HeaderLine> {
    response
        .headers
        .iter()
        .filter(|(name, _)| *name != SET_COOKIE)
        .map(|(name, value)| {
            let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
            HeaderLine {
                name: name.as_str().to_owned(),
                value: if *name == LOCATION {
                    shown_location(value)
                } else {
                    value
                },
            }
        })
        .collect()
}

/// A Location to a scheme other than http(s), such as the LMS app launch's
/// `moodlemobile://token=…`, carries a credential: it is withheld.
pub(super) fn shown_location(value: String) -> String {
    let scheme = value
        .split_once(':')
        .map(|(scheme, _)| scheme)
        .filter(|scheme| !scheme.contains(['/', '?', '#']));
    match scheme {
        Some(scheme)
            if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") =>
        {
            format!("{scheme}:(withheld)")
        }
        _ => value,
    }
}

fn body(response: &Response, save_to: Option<std::path::PathBuf>) -> Result<BodyOut, AgentError> {
    let bytes = &response.body;
    if let Some(path) = save_to {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("{}: {error}", folder.display()))?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| file.write_all(bytes))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        return Ok(BodyOut::Saved {
            path: path.display().to_string(),
            bytes: bytes.len() as u64,
        });
    }
    if bytes.is_empty() {
        return Ok(BodyOut::Empty);
    }
    let textual = response.content_type().is_some_and(|content_type| {
        let content_type = content_type.to_ascii_lowercase();
        content_type.starts_with("text/")
            || ["json", "xml", "javascript", "html"]
                .iter()
                .any(|kind| content_type.contains(kind))
    });
    if textual {
        if let Ok(mut text) = response.text() {
            let truncated = text.len() > MAX_TEXT;
            if truncated {
                let mut end = MAX_TEXT;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            return Ok(BodyOut::Text { text, truncated });
        }
    }
    if bytes.len() > MAX_INLINE_BINARY {
        return Ok(BodyOut::TooLarge {
            bytes: bytes.len() as u64,
        });
    }
    Ok(BodyOut::Base64 {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}
