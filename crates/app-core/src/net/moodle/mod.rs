//! The LMS (Moodle) client: one interface over two transports.
//!
//! [`rest::TokenTransport`] is the primary path: the official mobile web
//! service with a token obtained by a cookie-authenticated `launch.php`.
//! [`ajax::CookieTransport`] is the fallback: AJAX `service.php` with the
//! dashboard's sesskey, plus Rust HTML parsing of course and activity pages.
//! Both take only [`MoodleFunction`]s, a closed read-only list. The token
//! path reads the custom timetable through the generic dashboard-block API;
//! the cookie fallback also reads the block's other quarters.
//!
//! Secrets: the token, `privatetoken`, passport, sesskey and cookies are never
//! logged, formatted into errors or stored outside the OS vault. Requests
//! carry the token in the form body, so no URL holds it.
mod ajax;
mod envelope;
mod functions;
mod mapping;
mod reader;
mod rest;
mod token;

pub use functions::MoodleFunction;
pub(crate) use mapping::months_to_read;
pub(crate) use reader::LmsReader;

use crate::{
    net::{
        sign_in::Session, Body, Host, Http, Limits, NetError, NetRequest, Response, SessionCookies,
        SignInGate,
    },
    school_auth::{MoodleToken, SchoolAuthProfile, SchoolAuthService},
    sync_state::Dataset,
};
use adpt_isct::{
    lms::{
        is_lms_origin, page_config, read_timetable, timetable_filters, timetable_shape, PageConfig,
        LMS_WWWROOT,
    },
    SchoolParseError,
};
use envelope::EnvelopeError;
use reqwest::{header, Method};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;
use url::Url;

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
    /// The answer reported warnings about something other than an
    /// inaccessible course, so it may be incomplete.
    Partial,
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
            Self::Partial => formatter.write_str("answer reported warnings"),
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
}

/// Why one LMS key could not be read in full: the part that failed (a REST
/// function name, a page path or `validate`) and a short code. Never
/// response text, URLs or credentials.
#[derive(Clone, Debug)]
pub(crate) struct KeyError {
    pub(crate) part: &'static str,
    error: MoodleError,
    /// Failed on the session cookie (dashboard, timetable): reading it again
    /// over the cookie transport cannot help.
    cookie_side: bool,
}

impl KeyError {
    pub(crate) fn new(part: &'static str, error: MoodleError) -> Self {
        Self {
            part,
            error,
            cookie_side: false,
        }
    }

    /// For `map_err`: the error of the request `part`.
    pub(crate) fn at(part: &'static str) -> impl Fn(MoodleError) -> Self {
        move |error| Self::new(part, error)
    }

    /// A rejected read: the parser's reason is logged, not carried.
    fn parse(error: SchoolParseError) -> Self {
        log::warn!(target: "app_core::net::moodle", "LMS key rejected: {error}");
        Self::new("validate", MoodleError::Malformed("rejected by validation"))
    }

    /// Marks an error of a cookie request, which the cookie fallback cannot
    /// cure.
    fn on_cookie(mut self) -> Self {
        self.cookie_side = true;
        self
    }

    fn wants_cookie_fallback(&self) -> bool {
        !self.cookie_side && self.error.wants_cookie_fallback()
    }

    pub(crate) fn is_sign_in(&self) -> bool {
        matches!(
            self.error,
            MoodleError::SignInRequired | MoodleError::Net(NetError::SignInRequired)
        )
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        matches!(self.error, MoodleError::Net(NetError::Cancelled))
    }

    /// When the host's breaker lets requests through again, if it paused.
    pub(crate) fn paused_until(&self) -> Option<i64> {
        match self.error {
            MoodleError::Net(NetError::HostPaused { until_unix, .. }) => Some(until_unix),
            _ => None,
        }
    }

    /// The short code for `sync_runs.error_kind` and outcome messages.
    pub(crate) fn kind(&self) -> String {
        match &self.error {
            MoodleError::Net(error) => error.kind(),
            MoodleError::SignInRequired | MoodleError::InvalidSesskey => "sign-in".into(),
            MoodleError::InvalidToken => "token".into(),
            MoodleError::ServiceUnavailable(_) => "unavailable".into(),
            MoodleError::Api(code) => {
                let code: String = code
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .take(40)
                    .collect();
                format!("moodle-{code}")
            }
            MoodleError::Malformed(_) => "parse".into(),
            MoodleError::Partial => "partial".into(),
        }
    }
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.part, self.error)
    }
}

/// The canonical `sync_runs.source` of an LMS key: the function that reads
/// its inventory.
pub(crate) fn source(dataset: Dataset) -> &'static str {
    match dataset {
        Dataset::LmsEnrolment => MoodleFunction::EnrolledCoursesByTimeline.name(),
        Dataset::LmsDeadlines => MoodleFunction::ActionEventsByTimesort.name(),
        Dataset::LmsAssign => MoodleFunction::AssignGetSubmissionStatus.name(),
        Dataset::LmsHistory => MoodleFunction::CalendarMonthlyView.name(),
        _ => MoodleFunction::CourseContents.name(),
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

    /// Opens the transport one pull reads its keys with: the token path,
    /// proven once with `site_info`, or the session cookie when the token
    /// path is not offered.
    pub(crate) async fn open(&self) -> Result<LmsReader<'_>, KeyError> {
        match rest::TokenTransport::open(self).await {
            Ok((transport, language)) => Ok(LmsReader::new(
                self,
                reader::Transport::Token(transport),
                language,
                None,
            )),
            Err(error) if error.wants_cookie_fallback() => {
                log::warn!(target: "app_core::net::moodle", "LMS token transport unavailable ({error}); reading with the session cookie");
                let (transport, dashboard) =
                    self.cookie_transport().await.map_err(KeyError::at("my/"))?;
                let language = dashboard.config.language.clone();
                Ok(LmsReader::new(
                    self,
                    reader::Transport::Cookie(transport),
                    language,
                    Some(dashboard),
                ))
            }
            Err(error) => Err(KeyError::new(MoodleFunction::SiteInfo.name(), error)),
        }
    }

    /// The cookie transport over a freshly read dashboard.
    async fn cookie_transport(
        &self,
    ) -> Result<(ajax::CookieTransport<'_>, Dashboard), MoodleError> {
        let dashboard = self.dashboard().await?;
        let transport = ajax::CookieTransport::new(self, &dashboard)?;
        Ok((transport, dashboard))
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
                log::warn!(target: "app_core::net::moodle", "could not store LMS cookies: {error}");
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
        self.signed_in(|| self.load_dashboard()).await
    }

    /// Runs `read`, signing in once in the WebView when it finds the session
    /// gone (`net::SignInGate::signed_in`).
    async fn signed_in<T, Fut>(&self, read: impl Fn() -> Fut) -> Result<T, MoodleError>
    where
        Fut: std::future::Future<Output = Result<T, MoodleError>>,
    {
        self.sign_in
            .signed_in(SchoolAuthProfile::Science, || async {
                match read().await {
                    Err(MoodleError::SignInRequired) => Ok(Session::Gone),
                    other => other.map(Session::Live),
                }
            })
            .await?
            .ok_or(MoodleError::SignInRequired)
    }

    /// The timetable grids: the dashboard's current one first, then every
    /// other quarter the block offers for `years` (the enrolled academic
    /// years) through the block's own filter POST. Any grid that cannot be
    /// read fails the whole list.
    async fn timetables(
        &self,
        dashboard: &Dashboard,
        years: &BTreeSet<i32>,
    ) -> Result<Vec<Value>, KeyError> {
        let page = "my/index.php";
        let current = read_timetable(&dashboard.html, &dashboard.url).ok_or_else(|| {
            log::warn!(target: "app_core::net::moodle", "LMS timetable unreadable: {}", timetable_shape(&dashboard.html));
            KeyError::new("my/", MoodleError::Malformed("timetable"))
        })?;
        let current_term = grid_term(&current);
        let mut grids = vec![current];
        let filters = timetable_filters(&dashboard.html)
            .filter(|filters| !filters.years.is_empty() && filters.quarters.len() == 4)
            .ok_or_else(|| KeyError::new("my/", MoodleError::Malformed("timetable filters")))?;
        let post_url = lms_url(page).map_err(KeyError::at(page))?;
        for (year_value, year) in &filters.years {
            if !year.parse::<i32>().is_ok_and(|year| years.contains(&year)) {
                continue;
            }
            for (quarter_value, quarter) in &filters.quarters {
                let expected = format!("{year}-{quarter}Q");
                if current_term == expected {
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
                let response = self
                    .cookie_send(
                        Method::POST,
                        post_url.clone(),
                        Body::Form(form),
                        Limits::page,
                    )
                    .await
                    .map_err(KeyError::at(page))?;
                if response.status != 200 {
                    return Err(KeyError::new(
                        page,
                        MoodleError::Net(NetError::Http(response.status)),
                    ));
                }
                let html = response
                    .text()
                    .map_err(|error| KeyError::new(page, MoodleError::from(error)))?;
                match read_timetable(&html, &post_url) {
                    Some(grid) if grid_term(&grid) == expected => grids.push(grid),
                    Some(grid) => {
                        log::warn!(target: "app_core::net::moodle", "LMS timetable for {expected} answered {}", grid_term(&grid));
                        return Err(KeyError::new(
                            page,
                            MoodleError::Malformed("timetable quarter"),
                        ));
                    }
                    None => {
                        log::warn!(target: "app_core::net::moodle", "LMS timetable for {expected} unreadable: {}", timetable_shape(&html));
                        return Err(KeyError::new(page, MoodleError::Malformed("timetable")));
                    }
                }
            }
        }
        Ok(grids)
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
                log::warn!(target: "app_core::net::moodle", "LMS token vault read failed: {error}");
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
            log::warn!(target: "app_core::net::moodle", "LMS token vault write failed: {error}");
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
        self.signed_in(|| self.launch_once()).await
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
