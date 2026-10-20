//! The LMS (Moodle) client: one interface over two transports.
//!
//! [`rest::TokenTransport`] is the primary path: the official mobile web
//! service with a token obtained by a cookie-authenticated `launch.php`.
//! [`ajax::CookieTransport`] is the fallback: AJAX `service.php` with the
//! dashboard's sesskey, plus Rust HTML parsing of course and activity pages.
//! Both take only [`MoodleFunction`]s, a closed read-only list. The custom
//! timetable block has no web-service function, so it is read from the
//! dashboard with the session cookie on both paths.
//!
//! Secrets: the token, `privatetoken`, passport, sesskey and cookies are never
//! logged, formatted into errors or stored outside the OS vault. Requests
//! carry the token in the form body, so no URL holds it.
mod ajax;
mod envelope;
mod functions;
mod mapping;
mod pull;
mod rest;
mod token;

pub use functions::MoodleFunction;

use crate::{
    net::{
        Body, Host, Http, KeyOutcome, Limits, NetError, NetRequest, Response, SessionCookies,
        SignInGate,
    },
    school_auth::{MoodleToken, SchoolAuthProfile, SchoolAuthService},
    school_browser::SchoolPullProgress,
    school_parser::lms::{
        is_lms_origin, page_config, read_timetable, timetable_filters, timetable_shape,
        validate_payload, LmsPayload, LmsStage, PageConfig, LMS_WWWROOT,
    },
};
use envelope::EnvelopeError;
use reqwest::{header, Method};
use serde_json::Value;
use std::sync::Arc;
use url::Url;

/// How much a pull reads when no single course is named.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LmsScope {
    /// The current quarter's timetable and the deadlines.
    Summary,
    /// The summary plus the content of the current quarter's courses.
    CurrentQuarter,
    /// Every quarter's timetable, every course's content and past deadlines.
    Full,
}

/// Parts a pull reports in [`KeyOutcome::Partial`] when it could not read
/// them; their stored values stay.
pub mod part {
    pub const TIMETABLE: &str = "timetable";
    pub const UPCOMING_EVENTS: &str = "upcoming_events";
    pub const COURSE_STRUCTURE: &str = "course_structure";
    pub const MONTH_CALENDAR: &str = "month_calendar";
    pub const MATERIAL_DETAIL: &str = "material_detail";
    pub const ASSIGN_STATUS: &str = "assign_status";
}

/// Why one Moodle read failed. Carries error codes only, never response
/// text, URLs or credentials.
#[derive(Clone, Debug)]
pub(crate) enum MoodleError {
    Net(NetError),
    SignInRequired,
    /// The token was refused even after one relaunch.
    InvalidToken,
    /// The token path is not offered (launch refused, service or function
    /// unavailable); the cookie transport takes over.
    ServiceUnavailable(String),
    InvalidSesskey,
    Api(String),
    Malformed(&'static str),
}

impl From<NetError> for MoodleError {
    fn from(error: NetError) -> Self {
        match error {
            NetError::SignInRequired => Self::SignInRequired,
            other => Self::Net(other),
        }
    }
}

impl std::fmt::Display for MoodleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Net(error) => write!(formatter, "{error}"),
            Self::SignInRequired => formatter.write_str("sign-in required"),
            Self::InvalidToken => formatter.write_str("token refused"),
            Self::ServiceUnavailable(code) => write!(formatter, "service unavailable: {code}"),
            Self::InvalidSesskey => formatter.write_str("sesskey refused"),
            Self::Api(code) => write!(formatter, "moodle error: {code}"),
            Self::Malformed(what) => write!(formatter, "unexpected response: {what}"),
        }
    }
}

impl MoodleError {
    /// The token path cannot serve this pull; read it with the cookie.
    fn wants_cookie_fallback(&self) -> bool {
        matches!(
            self,
            Self::InvalidToken
                | Self::ServiceUnavailable(_)
                | Self::Api(_)
                | Self::Malformed(_)
                // A gateway or a disabled web service answering the REST path.
                | Self::Net(NetError::Http(400..=499))
        )
    }

    fn into_net(self) -> NetError {
        match self {
            Self::Net(error) => error,
            Self::SignInRequired => NetError::SignInRequired,
            other => NetError::Invalid(format!("LMS {other}")),
        }
    }
}

fn lms_url(path: &str) -> Result<Url, MoodleError> {
    Url::parse(&format!("{LMS_WWWROOT}/{path}")).map_err(|_| MoodleError::Malformed("LMS URL"))
}

/// A redirect to a sign-in page: off the LMS origin, or Moodle's own login.
fn sign_in_redirect(location: &str, base: &Url) -> bool {
    // Only http(s) locations are parsed; the launch's app-scheme answer is
    // handled by `token` and must never reach a generic URL parser.
    let lower = location.trim_start().to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with('/')) {
        return false;
    }
    base.join(location)
        .is_ok_and(|url| !is_lms_origin(&url) || url.path().contains("/login/"))
}

/// The signed-in dashboard: its HTML (timetable block) and `M.cfg`.
pub(crate) struct Dashboard {
    html: String,
    url: Url,
    config: PageConfig,
}

/// Reads LMS data for the signed-in Science account.
pub struct MoodleClient {
    http: Http,
    cookies: Arc<dyn SessionCookies>,
    sign_in: SignInGate,
    auth: Arc<SchoolAuthService>,
    /// The token in use; loaded from the vault or launched at most once at a
    /// time (the lock is held across the launch).
    token: tokio::sync::Mutex<Option<MoodleToken>>,
}

impl MoodleClient {
    pub fn new(
        http: Http,
        cookies: Arc<dyn SessionCookies>,
        sign_in: SignInGate,
        auth: Arc<SchoolAuthService>,
    ) -> Self {
        Self {
            http,
            cookies,
            sign_in,
            auth,
            token: tokio::sync::Mutex::new(None),
        }
    }

    /// Reads one LMS pull in the extractor's scopes and maps it into the
    /// existing ingest payload. Token transport first; the cookie transport
    /// when the token path is not available.
    pub async fn pull(
        &self,
        scope: LmsScope,
        course: Option<u64>,
        progress: &(dyn Fn(SchoolPullProgress) + Send + Sync),
    ) -> Result<KeyOutcome<LmsPayload>, NetError> {
        let result = match rest::TokenTransport::open(self).await {
            Ok((transport, language)) => {
                let run = pull::Pull::new(
                    self,
                    pull::Transport::Token(transport),
                    scope,
                    course,
                    progress,
                    language,
                    None,
                )
                .run()
                .await;
                match run {
                    Err(error) if error.wants_cookie_fallback() => {
                        log::warn!(target: "isct_core::net::moodle", "LMS token transport failed ({error}); reading with the session cookie");
                        self.pull_with_cookie(scope, course, progress).await
                    }
                    other => other,
                }
            }
            Err(error) if error.wants_cookie_fallback() => {
                log::warn!(target: "isct_core::net::moodle", "LMS token transport unavailable ({error}); reading with the session cookie");
                self.pull_with_cookie(scope, course, progress).await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(output) => match validate_payload(output.payload) {
                Ok((LmsStage::Ready, Some(payload))) if output.missing.is_empty() => {
                    Ok(KeyOutcome::Complete(payload))
                }
                Ok((LmsStage::Ready, Some(payload))) => Ok(KeyOutcome::Partial {
                    data: payload,
                    missing: output.missing,
                }),
                Ok(_) => Ok(KeyOutcome::SignInRequired),
                Err(error) => Err(NetError::Invalid(format!("LMS payload rejected: {error}"))),
            },
            Err(MoodleError::SignInRequired) => Ok(KeyOutcome::SignInRequired),
            Err(MoodleError::ServiceUnavailable(_)) => Ok(KeyOutcome::Unavailable("lms")),
            Err(error) => Err(error.into_net()),
        }
    }

    async fn pull_with_cookie(
        &self,
        scope: LmsScope,
        course: Option<u64>,
        progress: &(dyn Fn(SchoolPullProgress) + Send + Sync),
    ) -> Result<pull::Output, MoodleError> {
        let dashboard = self.dashboard().await?;
        let transport = ajax::CookieTransport::new(self, &dashboard)?;
        let language = dashboard.config.language.clone();
        pull::Pull::new(
            self,
            pull::Transport::Cookie(transport),
            scope,
            course,
            progress,
            language,
            Some(dashboard),
        )
        .run()
        .await
    }

    async fn ensure_signed_in(&self) -> Result<(), MoodleError> {
        self.sign_in
            .ensure(SchoolAuthProfile::Science)
            .await
            .map_err(MoodleError::from)
    }

    fn envelope_error(&self, error: EnvelopeError) -> MoodleError {
        match error {
            EnvelopeError::NotJson => MoodleError::Malformed("not JSON"),
            EnvelopeError::InvalidToken => MoodleError::InvalidToken,
            EnvelopeError::ServiceUnavailable(code) => MoodleError::ServiceUnavailable(code),
            EnvelopeError::InvalidSesskey => MoodleError::InvalidSesskey,
            EnvelopeError::SignInRequired => MoodleError::SignInRequired,
            EnvelopeError::Maintenance => {
                self.http.report_maintenance(Host::Moodle);
                MoodleError::Api("sitemaintenance".into())
            }
            EnvelopeError::Api(code) => MoodleError::Api(code),
        }
    }

    // -- Cookie requests -----------------------------------------------------

    /// One request with the school session cookie. Cookies the server sets
    /// go back to the WebView store so both sides keep one session.
    async fn cookie_send(
        &self,
        method: Method,
        url: Url,
        body: Body,
        limits: fn() -> Limits,
    ) -> Result<Response, MoodleError> {
        let cookie = self.cookies.cookie_header(&url).await?;
        let response = self
            .http
            .send(NetRequest {
                host: Host::Moodle,
                method,
                url: url.clone(),
                headers: vec![(header::COOKIE, cookie)],
                body,
                limits: limits(),
            })
            .await?;
        let set = response.set_cookies();
        if !set.is_empty() {
            if let Err(error) = self.cookies.store_set_cookies(&url, &set).await {
                log::warn!(target: "isct_core::net::moodle", "could not store LMS cookies: {error}");
            }
        }
        Ok(response)
    }

    /// A page GET that must answer 200 with HTML; a redirect to sign-in is
    /// `SignInRequired`.
    async fn cookie_page(&self, url: Url) -> Result<String, MoodleError> {
        let response = self
            .cookie_send(Method::GET, url.clone(), Body::None, Limits::page)
            .await?;
        match response.status {
            200 => Ok(response.text()?),
            300..=399 => Err(match response.location() {
                Some(location) if sign_in_redirect(&location, &url) => MoodleError::SignInRequired,
                _ => MoodleError::Net(NetError::Http(response.status)),
            }),
            status => Err(MoodleError::Net(NetError::Http(status))),
        }
    }

    async fn load_dashboard(&self) -> Result<Dashboard, MoodleError> {
        let url = lms_url("my/")?;
        let html = self.cookie_page(url.clone()).await?;
        let config = page_config(&html);
        // A page without a sesskey is a sign-in form, not the dashboard.
        if config.sesskey.is_none() {
            return Err(MoodleError::SignInRequired);
        }
        Ok(Dashboard { html, url, config })
    }

    /// The dashboard, signing in once in the WebView when the session is gone.
    async fn dashboard(&self) -> Result<Dashboard, MoodleError> {
        match self.load_dashboard().await {
            Err(MoodleError::SignInRequired) => {
                self.ensure_signed_in().await?;
                self.load_dashboard().await
            }
            other => other,
        }
    }

    /// The timetable grids: the dashboard's current one, and for a full pull
    /// every other offered year and quarter through the block's own filter
    /// POST. A grid that fails is named in the errors; the others stay.
    async fn timetables(
        &self,
        dashboard: &Dashboard,
        every_quarter: bool,
    ) -> (Vec<Value>, Vec<String>) {
        let current = read_timetable(&dashboard.html, &dashboard.url);
        let current_term = current.as_ref().map(grid_term);
        let mut grids: Vec<Value> = current.into_iter().collect();
        let mut errors = Vec::new();
        if !every_quarter {
            return (grids, errors);
        }
        let filters = timetable_filters(&dashboard.html).unwrap_or_default();
        if filters.years.is_empty() || filters.quarters.len() != 4 {
            errors.push("timetable-filters-unavailable".to_string());
        }
        let Ok(post_url) = lms_url("my/index.php") else {
            return (grids, errors);
        };
        for (year_value, year) in &filters.years {
            for (quarter_value, quarter) in &filters.quarters {
                let expected = format!("{year}-{quarter}Q");
                if current_term.as_deref() == Some(expected.as_str()) {
                    continue;
                }
                // The read-only year/quarter query: exactly these three
                // fields to this one endpoint. Keeping the form's search flag
                // matters: without it Moodle ignores year and quota.
                let form = vec![
                    ("year".to_string(), year_value.clone()),
                    ("quota".to_string(), quarter_value.clone()),
                    ("search".to_string(), filters.search.clone()),
                ];
                let result = match self
                    .cookie_send(
                        Method::POST,
                        post_url.clone(),
                        Body::Form(form),
                        Limits::page,
                    )
                    .await
                {
                    Ok(response) if response.status == 200 => match response.text() {
                        Ok(html) => match read_timetable(&html, &post_url) {
                            Some(grid) if grid_term(&grid) == expected => Ok(grid),
                            Some(grid) => Err(format!(
                                "timetable-quarter-mismatch: got {}",
                                grid_term(&grid)
                            )),
                            None => {
                                Err(format!("timetable-unreadable: {}", timetable_shape(&html)))
                            }
                        },
                        Err(error) => Err(format!("timetable-request-failed: {error}")),
                    },
                    Ok(response) => Err(format!(
                        "timetable-request-failed: HTTP {}",
                        response.status
                    )),
                    Err(error) => Err(format!("timetable-request-failed: {error}")),
                };
                match result {
                    Ok(grid) => grids.push(grid),
                    Err(message) => errors.push(format!("{expected}: {message}")),
                }
            }
        }
        (grids, errors)
    }

    // -- Token lifecycle -----------------------------------------------------

    async fn vault_token(&self) -> Option<MoodleToken> {
        let auth = self.auth.clone();
        match tokio::task::spawn_blocking(move || {
            auth.load_moodle_token(SchoolAuthProfile::Science)
        })
        .await
        {
            Ok(Ok(token)) => token,
            Ok(Err(error)) => {
                log::warn!(target: "isct_core::net::moodle", "LMS token vault read failed: {error}");
                None
            }
            Err(_) => None,
        }
    }

    async fn persist_token(&self, token: Option<MoodleToken>) {
        let auth = self.auth.clone();
        let result = tokio::task::spawn_blocking(move || match token {
            Some(token) => auth.store_moodle_token(SchoolAuthProfile::Science, &token),
            None => auth.delete_moodle_token(SchoolAuthProfile::Science),
        })
        .await;
        if let Ok(Err(error)) = result {
            // The token still works for this process; the next launch
            // replaces it.
            log::warn!(target: "isct_core::net::moodle", "LMS token vault write failed: {error}");
        }
    }

    /// The token in use: memory, else the vault, else a new launch.
    async fn token(&self) -> Result<MoodleToken, MoodleError> {
        let mut slot = self.token.lock().await;
        if let Some(token) = slot.as_ref() {
            return Ok(token.clone());
        }
        if let Some(token) = self.vault_token().await {
            *slot = Some(token.clone());
            return Ok(token);
        }
        let token = self.launch().await?;
        self.persist_token(Some(token.clone())).await;
        *slot = Some(token.clone());
        Ok(token)
    }

    /// After `invalidtoken`: drop the token, confirm the web sign-in and
    /// launch again. Concurrent callers that saw the same refused token join
    /// one renewal.
    async fn renew_token(&self, refused: &MoodleToken) -> Result<MoodleToken, MoodleError> {
        let mut slot = self.token.lock().await;
        if let Some(current) = slot.as_ref().filter(|current| *current != refused) {
            return Ok(current.clone());
        }
        *slot = None;
        self.persist_token(None).await;
        self.ensure_signed_in().await?;
        let token = self.launch().await?;
        self.persist_token(Some(token.clone())).await;
        *slot = Some(token.clone());
        Ok(token)
    }

    async fn launch(&self) -> Result<MoodleToken, MoodleError> {
        match self.launch_once().await {
            Err(MoodleError::SignInRequired) => {
                self.ensure_signed_in().await?;
                self.launch_once().await
            }
            other => other,
        }
    }

    /// One `launch.php` GET with the LMS cookie, redirects off.
    async fn launch_once(&self) -> Result<MoodleToken, MoodleError> {
        let passport = token::new_passport();
        let url = token::launch_url(LMS_WWWROOT, &passport)
            .map_err(|_| MoodleError::Malformed("launch URL"))?;
        let response = self
            .cookie_send(Method::GET, url.clone(), Body::None, Limits::page)
            .await?;
        if !(300..400).contains(&response.status) {
            return Err(MoodleError::ServiceUnavailable(format!(
                "launch HTTP {}",
                response.status
            )));
        }
        let location = response
            .location()
            .ok_or_else(|| MoodleError::ServiceUnavailable("launch without Location".into()))?;
        match token::parse_launch_location(&location, LMS_WWWROOT, &passport) {
            Ok(token) => Ok(token),
            Err(token::LaunchError::NotLaunch) if sign_in_redirect(&location, &url) => {
                Err(MoodleError::SignInRequired)
            }
            Err(token::LaunchError::NotLaunch) => Err(MoodleError::ServiceUnavailable(
                "launch redirected elsewhere".into(),
            )),
            Err(token::LaunchError::Malformed) => Err(MoodleError::ServiceUnavailable(
                "launch answer malformed".into(),
            )),
            Err(token::LaunchError::SignatureMismatch) => Err(MoodleError::ServiceUnavailable(
                "launch signature mismatch".into(),
            )),
        }
    }
}

/// `2026-3Q` of a raw grid.
fn grid_term(grid: &Value) -> String {
    format!(
        "{}-{}Q",
        grid.get("year").and_then(Value::as_u64).unwrap_or(0),
        grid.get("quarter").and_then(Value::as_str).unwrap_or("")
    )
}
