//! Core's HTTP layer for school and public sources: one client per host with
//! that host's budget, timeouts, size cap and circuit breaker. School requests
//! carry the sign-in of the school WebView through [`SessionCookies`]; the
//! WebView itself only signs in and shows pages the user opens.

mod breaker;
pub mod cookies;
mod http;
pub mod kyomu;
pub mod moodle;
pub mod redact;
pub mod sign_in;
mod tls;

pub use cookies::SessionCookies;
pub use http::Http;
pub use sign_in::{SignIn, SignInGate};

use std::{future::Future, pin::Pin, time::Duration};
use url::Url;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Sent on every request: reqwest sends no User-Agent of its own.
pub const USER_AGENT: &str = "Mozilla/5.0 (compatible; IsctCompanion/0.1)";

/// A `reqwest` client builder with the TLS every Core and shell client uses:
/// rustls with ring and the platform verifier (reqwest 0.13's default
/// verifier). Build every client through this, never `reqwest::Client::builder`:
/// without a process crypto provider reqwest panics when the client is built.
pub fn client_builder() -> reqwest::ClientBuilder {
    tls::install_provider();
    reqwest::Client::builder()
}

/// The TLS a school host needs on top of the platform verifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HostTls {
    Platform,
    /// kyomu0's missing intermediate, offered to the platform verifier (`tls`).
    KyomuIntermediate,
}

impl HostTls {
    pub(crate) fn of(host: &str) -> Self {
        if host == Host::Kyomu.name() {
            Self::KyomuIntermediate
        } else {
            Self::Platform
        }
    }
}

/// [`client_builder`] with the TLS `host` needs. Every Core client for a
/// school host is built through this, so the 教務Web workaround has one owner
/// whether the request goes through [`Http`] or a file download.
pub(crate) fn host_client_builder(host: &str) -> Result<reqwest::ClientBuilder, NetError> {
    let builder = client_builder();
    Ok(match HostTls::of(host) {
        HostTls::Platform => builder,
        HostTls::KyomuIntermediate => {
            builder.tls_backend_preconfigured(tls::kyomu_config().map_err(NetError::Tls)?)
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Host {
    Moodle,
    Kyomu,
    GetAccess,
    ScienceAccount,
    IsctData,
}

impl Host {
    pub const ALL: [Host; 5] = [
        Host::Moodle,
        Host::Kyomu,
        Host::GetAccess,
        Host::ScienceAccount,
        Host::IsctData,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Host::Moodle => "lms.s.isct.ac.jp",
            Host::Kyomu => "kyomu0.gakumu.titech.ac.jp",
            Host::GetAccess => "portal.nap.gsic.titech.ac.jp",
            Host::ScienceAccount => "isct.ex-tic.com",
            Host::IsctData => "raw.githubusercontent.com",
        }
    }

    /// The host `url` belongs to: https on the default port, no userinfo.
    pub fn of(url: &Url) -> Option<Host> {
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return None;
        }
        let name = url.host_str()?;
        Host::ALL.into_iter().find(|host| host.name() == name)
    }

    /// Concurrent requests and the fixed spacing between request starts.
    pub(crate) const fn budget(self) -> (usize, Duration) {
        match self {
            Host::Moodle => (4, Duration::from_millis(60)),
            Host::Kyomu | Host::GetAccess | Host::ScienceAccount => (1, Duration::from_millis(300)),
            Host::IsctData => (2, Duration::ZERO),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum NetError {
    #[error("offline")]
    Offline,
    #[error("DNS lookup failed")]
    Dns,
    #[error("TLS failed: {0}")]
    Tls(String),
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("no data for too long")]
    Stalled,
    #[error("deadline exceeded")]
    Deadline,
    #[error("response too large")]
    TooLarge,
    #[error("HTTP {0}")]
    Http(u16),
    #[error("{host:?} paused until {until_unix}")]
    HostPaused { host: Host, until_unix: i64 },
    #[error("sign-in required")]
    SignInRequired,
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Invalid(String),
}

impl NetError {
    /// Short code for `sync_runs.error_kind` and backoff records; never
    /// response text.
    pub fn kind(&self) -> String {
        match self {
            Self::Offline => "offline".into(),
            Self::Dns => "dns".into(),
            Self::Tls(_) => "tls".into(),
            Self::Connect(_) => "connect".into(),
            Self::Stalled => "stalled".into(),
            Self::Deadline => "timeout".into(),
            Self::TooLarge => "too-large".into(),
            Self::Http(status) => format!("http-{status}"),
            Self::HostPaused { .. } => "host-paused".into(),
            Self::SignInRequired => "sign-in".into(),
            Self::Cancelled => "cancelled".into(),
            Self::Invalid(_) => "parse".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_bytes: usize,
    pub deadline: Duration,
}

impl Limits {
    /// One HTML page or API call.
    pub const fn page() -> Self {
        Self {
            max_bytes: 4 * 1024 * 1024,
            deadline: Duration::from_secs(60),
        }
    }

    /// A large listing, such as a whole-department catalog search.
    pub const fn bulk() -> Self {
        Self {
            max_bytes: 16 * 1024 * 1024,
            deadline: Duration::from_secs(180),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Body {
    None,
    Form(Vec<(String, String)>),
    Json(serde_json::Value),
}

#[derive(Clone, Debug)]
pub struct NetRequest {
    pub host: Host,
    pub method: reqwest::Method,
    /// Must belong to `host`; https only.
    pub url: Url,
    /// Includes Cookie, Referer and Range when the caller needs them.
    pub headers: Vec<(reqwest::header::HeaderName, String)>,
    pub body: Body,
    pub limits: Limits,
}

impl NetRequest {
    /// A GET with page limits and no extra headers.
    pub fn get(host: Host, url: Url) -> Self {
        Self {
            host,
            method: reqwest::Method::GET,
            url,
            headers: Vec::new(),
            body: Body::None,
            limits: Limits::page(),
        }
    }
}

/// A complete response of any status. Redirects are returned, never followed.
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub url: Url,
    pub headers: reqwest::header::HeaderMap,
    pub body: bytes::Bytes,
}

impl Response {
    /// The raw `Location` value. It is never parsed: `moodlemobile://token=…`
    /// is not a URL, and a URL parser reads the token as its host.
    pub fn location(&self) -> Option<String> {
        self.headers
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// Raw `Set-Cookie` values, for [`SessionCookies::store_set_cookies`].
    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(str::to_owned)
            .collect()
    }

    pub fn content_type(&self) -> Option<String> {
        self.headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    /// The body decoded with the `Content-Type` charset, UTF-8 when none is
    /// declared. A byte-order mark wins over the declaration, and malformed
    /// sequences become U+FFFD, as in a browser. An unknown charset is an
    /// error rather than a guess.
    pub fn text(&self) -> Result<String, NetError> {
        let encoding = match self.content_type().as_deref().and_then(charset) {
            Some(label) => encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or_else(|| NetError::Invalid(format!("unsupported charset {label}")))?,
            None => encoding_rs::UTF_8,
        };
        let (text, _, _) = encoding.decode(&self.body);
        Ok(text.into_owned())
    }
}

/// The `charset` parameter of a `Content-Type` value.
fn charset(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
    })
}

/// What a transport managed to read for one dataset key.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyOutcome<T> {
    Complete(T),
    /// Usable data, but these named parts could not be read; keep their old values.
    Partial {
        data: T,
        missing: Vec<&'static str>,
    },
    /// The current transport cannot provide this data at all.
    Unavailable(&'static str),
    SignInRequired,
}

#[cfg(test)]
#[path = "../tests/net.rs"]
mod tests;
