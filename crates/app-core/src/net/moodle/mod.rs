//! The LMS (Moodle) client: one interface over two transports, every request
//! on the LMS site session's attempt (`school_session::Attempt`).
//!
//! [`rest::TokenTransport`] is the primary path: the official mobile web
//! service with a token obtained by a cookie-authenticated `launch.php`. Its
//! calls carry the token alone, so they need no site session: only a
//! cookie request (a launch for a new token, the fallback, the timetable
//! grids, files) gets one, and may go through SSO or a sign-in.
//! [`ajax::CookieTransport`] is the fallback: AJAX `service.php` with the
//! dashboard's sesskey, plus Rust HTML parsing of course and activity pages.
//! Which one a key reads with is decided per key (`reader::Via`), from its
//! credential path and what is known of the token (`TokenFact`).
//! A session found gone ends the attempt as `SignInRequired`; recovering it
//! and reading again is `school_session::run`'s, never this client's.
//! Both take only [`MoodleFunction`]s, a closed read-only list. The token
//! path reads the custom timetable through the generic dashboard-block API;
//! the cookie fallback also reads the block's other quarters. LMS writes
//! (assignment submissions) are a separate closed list in `submission`,
//! sent only for the submission service.
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
#[cfg(feature = "agent")]
mod submission;
mod token;

pub use functions::MoodleFunction;
pub(crate) use mapping::months_to_read;
pub(crate) use reader::{LmsReader, Via};
#[cfg(feature = "agent")]
pub(crate) use submission::LmsSubmissions;

use crate::{
    net::{Body, Host, Limits, NetError, NetRequest, Response},
    school_auth::{MoodleToken, SchoolAuthProfile, SchoolAuthService},
    school_session::{Attempt, CredentialSignal, OpError},
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
use reqwest::Method;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
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
        self.error.signed_out()
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
            MoodleError::SignInRequired => "sign-in".into(),
            MoodleError::InvalidSesskey => "sesskey".into(),
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

impl OpError for MoodleError {
    fn signed_out(&self) -> bool {
        matches!(
            self,
            Self::SignInRequired | Self::Net(NetError::SignInRequired)
        )
    }
}

impl OpError for KeyError {
    fn signed_out(&self) -> bool {
        self.is_sign_in()
    }
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.part, self.error)
    }
}

/// The token functions a read of `dataset` calls (`site_info` proves the
/// token first). When the token is known unable to serve any of them, the
/// key reads with the session cookie only (`sync_engine::planner::usable_path`).
pub(crate) fn token_functions(dataset: Dataset) -> &'static [MoodleFunction] {
    use MoodleFunction as F;
    match dataset {
        Dataset::LmsEnrolment => &[F::SiteInfo, F::EnrolledCoursesByTimeline, F::DashboardBlocks],
        Dataset::LmsDeadlines => &[
            F::SiteInfo,
            F::ActionEventsByTimesort,
            F::CalendarUpcomingView,
        ],
        Dataset::LmsCourse => &[
            F::SiteInfo,
            F::CourseContents,
            F::PageGetPagesByCourses,
            F::ResourceGetResourcesByCourses,
            F::FolderGetFoldersByCourses,
            F::UrlGetUrlsByCourses,
            F::AssignGetAssignments,
            F::ForumGetForumsByCourses,
            F::ForumGetForumDiscussions,
        ],
        Dataset::LmsAssign => &[
            F::SiteInfo,
            F::AssignGetAssignments,
            F::QuizGetQuizzesByCourses,
            F::AssignGetSubmissionStatus,
            F::QuizGetUserAttempts,
            F::QuizGetUserBestGrade,
        ],
        Dataset::LmsHistory => &[F::SiteInfo, F::CalendarMonthlyView],
        Dataset::KyomuPage | Dataset::CatalogOfficial | Dataset::Reference => &[],
    }
}

/// The LMS token as sync reads it, for the account generation asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TokenFact {
    /// Not looked for in this generation yet (the vault not read, or its
    /// read failed): try-able.
    Unchecked,
    /// Confirmed absent: the vault holds none, or the token was refused and
    /// its renewal failed. `launchable`: a launch with the session cookie
    /// may still get one (false once the LMS answered that it offers none).
    Absent { launchable: bool },
    /// A token is held; the functions it was found unable to serve.
    Held { unavailable: HashSet<MoodleFunction> },
}

/// The token of one account generation.
struct Slot {
    generation: u64,
    state: TokenState,
}

enum TokenState {
    Absent {
        launchable: bool,
    },
    Held {
        token: MoodleToken,
        unavailable: HashSet<MoodleFunction>,
    },
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

/// The signed-in dashboard: its HTML (timetable block) and `M.cfg`, and
/// the site instance it was read on.
pub(crate) struct Dashboard {
    html: String,
    url: Url,
    config: PageConfig,
    instance: u64,
}

/// Reads LMS data for the signed-in Science account.
pub struct MoodleClient {
    auth: Arc<SchoolAuthService>,
    /// One load or launch of the token at a time (held across the launch).
    flight: tokio::sync::Mutex<()>,
    /// The token and what is known of it, for the account generation it was
    /// got in; another generation's reads as unchecked.
    slot: Mutex<Option<Slot>>,
    /// Told whenever what `token_fact` says changes.
    signal: Arc<CredentialSignal>,
}

impl MoodleClient {
    pub(crate) fn new(auth: Arc<SchoolAuthService>, signal: Arc<CredentialSignal>) -> Self {
        Self {
            auth,
            flight: tokio::sync::Mutex::new(()),
            slot: Mutex::new(None),
            signal,
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Slot>> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What is known of the token in account generation `generation`.
    pub(crate) fn token_fact(&self, generation: u64) -> TokenFact {
        match self
            .slot()
            .as_ref()
            .filter(|slot| slot.generation == generation)
        {
            None => TokenFact::Unchecked,
            Some(Slot {
                state: TokenState::Absent { launchable },
                ..
            }) => TokenFact::Absent {
                launchable: *launchable,
            },
            Some(Slot {
                state: TokenState::Held { unavailable, .. },
                ..
            }) => TokenFact::Held {
                unavailable: unavailable.clone(),
            },
        }
    }

    /// The held token of `generation`, or its absence, as it is now.
    fn current(&self, generation: u64) -> Option<Result<MoodleToken, bool>> {
        let slot = self.slot();
        let slot = slot.as_ref().filter(|slot| slot.generation == generation)?;
        Some(match &slot.state {
            TokenState::Held { token, .. } => Ok(token.clone()),
            TokenState::Absent { launchable } => Err(*launchable),
        })
    }

    fn set(&self, generation: u64, state: TokenState) {
        *self.slot() = Some(Slot { generation, state });
        self.signal.changed();
    }

    /// The function `function` failed under `token` in a way the session
    /// cookie may serve: a fact of that token only, dropped with it.
    pub(super) fn function_unavailable(
        &self,
        generation: u64,
        token: &MoodleToken,
        function: MoodleFunction,
    ) {
        let added = match self.slot().as_mut() {
            Some(Slot {
                generation: held,
                state:
                    TokenState::Held {
                        token: current,
                        unavailable,
                    },
            }) if *held == generation && current == token => unavailable.insert(function),
            _ => false,
        };
        if added {
            log::warn!(target: "app_core::net::moodle", "LMS token cannot serve {}; its keys read with the session cookie", function.name());
            self.signal.changed();
        }
    }

    /// `token` was refused right after a renewal: it is no token, and a
    /// launch gives none that works.
    pub(super) fn refused(&self, generation: u64, token: &MoodleToken) {
        let refused = matches!(
            self.slot().as_ref(),
            Some(Slot { generation: held, state: TokenState::Held { token: current, .. } })
                if *held == generation && current == token
        );
        if refused {
            self.set(generation, TokenState::Absent { launchable: false });
        }
    }

    fn envelope_error(attempt: &Attempt, error: EnvelopeError) -> MoodleError {
        match error {
            EnvelopeError::NotJson => MoodleError::Malformed("not JSON"),
            EnvelopeError::InvalidToken => MoodleError::InvalidToken,
            EnvelopeError::ServiceUnavailable(code) => MoodleError::ServiceUnavailable(code),
            EnvelopeError::InvalidSesskey => MoodleError::InvalidSesskey,
            EnvelopeError::SignInRequired => MoodleError::SignInRequired,
            // The LMS's maintenance is the site session's state: its host
            // pauses and the session snapshot shows it.
            EnvelopeError::Maintenance => MoodleError::Net(attempt.maintenance(None)),
            EnvelopeError::Api(code) => MoodleError::Api(code),
        }
    }

    // -- Cookie requests -----------------------------------------------------

    /// One request with the LMS site session's cookies.
    async fn cookie_send(
        attempt: &Attempt,
        method: Method,
        url: Url,
        body: Body,
        limits: fn() -> Limits,
    ) -> Result<Response, MoodleError> {
        Ok(attempt
            .send(NetRequest {
                host: Host::Moodle,
                method,
                url,
                headers: Vec::new(),
                body,
                limits: limits(),
            })
            .await?)
    }

    /// A page GET that must answer 200 with HTML; a redirect to sign-in is
    /// `SignInRequired`.
    async fn cookie_page(attempt: &Attempt, url: Url) -> Result<String, MoodleError> {
        let response =
            Self::cookie_send(attempt, Method::GET, url.clone(), Body::None, Limits::page).await?;
        match response.status {
            200 => Ok(response.text()?),
            300..=399 => Err(match response.location() {
                Some(location) if sign_in_redirect(&location, &url) => MoodleError::SignInRequired,
                _ => MoodleError::Net(NetError::Http(response.status)),
            }),
            status => Err(MoodleError::Net(NetError::Http(status))),
        }
    }

    async fn load_dashboard(attempt: &Attempt) -> Result<Dashboard, MoodleError> {
        let url = lms_url("my/")?;
        let html = Self::cookie_page(attempt, url.clone()).await?;
        let config = page_config(&html);
        // A page without a sesskey is a sign-in form, not the dashboard.
        if config.sesskey.is_none() {
            return Err(MoodleError::SignInRequired);
        }
        let instance = attempt.instance().await?;
        Ok(Dashboard {
            html,
            url,
            config,
            instance,
        })
    }

    /// The timetable grids: the dashboard's current one first, then every
    /// other quarter the block offers for `years` (the enrolled academic
    /// years) through the block's own filter POST. Any grid that cannot be
    /// read fails the whole list.
    async fn timetables(
        &self,
        attempt: &Attempt,
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
                let response = Self::cookie_send(
                    attempt,
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

    /// The vault's token; `Err` when the vault could not be read, which
    /// says nothing about whether one is saved.
    async fn vault_token(&self) -> Result<Option<MoodleToken>, ()> {
        let auth = self.auth.clone();
        match tokio::task::spawn_blocking(move || {
            auth.load_moodle_token(SchoolAuthProfile::Science)
        })
        .await
        {
            Ok(Ok(token)) => Ok(token),
            Ok(Err(error)) => {
                log::warn!(target: "app_core::net::moodle", "LMS token vault read failed: {error}");
                Err(())
            }
            Err(_) => Err(()),
        }
    }

    /// Keeps `token` (or forgets it) in the vault. The write holds a permit
    /// of `attempt`'s account generation until it lands, so a sign-out or
    /// reset waits for it and a token never lands after it was deleted.
    async fn persist_token(&self, attempt: &Attempt, token: Option<MoodleToken>) {
        let Some(permit) = attempt.write_permit() else {
            return;
        };
        let auth = self.auth.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match token {
                Some(token) => auth.store_moodle_token(SchoolAuthProfile::Science, &token),
                None => auth.delete_moodle_token(SchoolAuthProfile::Science),
            }
        })
        .await;
        if let Ok(Err(error)) = result {
            // The token still works for this process; the next launch
            // replaces it.
            log::warn!(target: "app_core::net::moodle", "LMS token vault write failed: {error}");
        }
    }

    /// The token in use: memory, else the vault, else a new launch.
    pub(crate) async fn token(&self, attempt: &Attempt) -> Result<MoodleToken, MoodleError> {
        let generation = attempt.account_generation();
        let _flight = self.flight.lock().await;
        attempt.authorize()?;
        match self.current(generation) {
            Some(Ok(token)) => return Ok(token),
            Some(Err(false)) => {
                return Err(MoodleError::ServiceUnavailable(
                    "the LMS offers no token".into(),
                ))
            }
            Some(Err(true)) => {}
            None => match self.vault_token().await {
                Ok(Some(token)) => {
                    self.set(
                        generation,
                        TokenState::Held {
                            token: token.clone(),
                            unavailable: HashSet::new(),
                        },
                    );
                    return Ok(token);
                }
                Ok(None) => self.set(generation, TokenState::Absent { launchable: true }),
                // Unchecked still: a launch may get one all the same.
                Err(()) => {}
            },
        }
        self.launch_held(attempt, generation).await
    }

    /// After `invalidtoken`: drop the token and launch again with the
    /// session cookie. Concurrent callers that saw the same refused token
    /// join one renewal. The refusal applies only to `refused`, the token
    /// the request used.
    async fn renew_token(
        &self,
        attempt: &Attempt,
        refused: &MoodleToken,
    ) -> Result<MoodleToken, MoodleError> {
        let generation = attempt.account_generation();
        let _flight = self.flight.lock().await;
        attempt.authorize()?;
        match self.current(generation) {
            Some(Ok(current)) if current != *refused => return Ok(current),
            Some(Err(false)) => {
                return Err(MoodleError::ServiceUnavailable(
                    "the LMS offers no token".into(),
                ))
            }
            _ => {}
        }
        // Refused: absent until a renewal gets another.
        self.set(generation, TokenState::Absent { launchable: true });
        self.persist_token(attempt, None).await;
        self.launch_held(attempt, generation).await
    }

    /// A launch, kept as the generation's token. A launch the LMS answers
    /// with no token marks it absent for good in this generation.
    async fn launch_held(
        &self,
        attempt: &Attempt,
        generation: u64,
    ) -> Result<MoodleToken, MoodleError> {
        match self.launch(attempt).await {
            Ok(token) => {
                self.persist_token(attempt, Some(token.clone())).await;
                self.set(
                    generation,
                    TokenState::Held {
                        token: token.clone(),
                        unavailable: HashSet::new(),
                    },
                );
                Ok(token)
            }
            Err(error @ MoodleError::ServiceUnavailable(_)) => {
                self.set(generation, TokenState::Absent { launchable: false });
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// One `launch.php` GET with the LMS cookie, redirects off.
    async fn launch(&self, attempt: &Attempt) -> Result<MoodleToken, MoodleError> {
        let passport = token::new_passport();
        let url = token::launch_url(LMS_WWWROOT, &passport)
            .map_err(|_| MoodleError::Malformed("launch URL"))?;
        let response =
            Self::cookie_send(attempt, Method::GET, url.clone(), Body::None, Limits::page).await?;
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
