//! 教務Web over Rust HTTP.
//!
//! The WebView only signs in; this client reuses its session cookies to GET
//! the six server-rendered student pages and the course search, and parses
//! the HTML in `school_parser::kyomu`. `Default.aspx` is never requested: it
//! bounces to the GetAccess menu and cannot start a session for Rust.

use crate::course_query::OfficialFilterOption;
use crate::net::{
    sign_in::Session, Body, Host, Http, KeyOutcome, Limits, NetError, NetRequest, SessionCookies,
    SignInGate,
};
use crate::school_auth::SchoolAuthProfile;
use crate::school_parser::kyomu::{
    self,
    search::{self, SearchRead},
    KyomuPayload, KyomuStage,
};
use reqwest::header::{COOKIE, REFERER};
use reqwest::Method;
use std::sync::{Arc, Mutex};
use url::Url;

const GETACCESS_HOST: &str = "portal.nap.gsic.titech.ac.jp";
/// The menu every student page is opened from in the browser. Sent as the
/// Referer only; the page itself is never requested.
const MENU: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
/// The breaker's first pause after a maintenance report. The breaker owns
/// the real schedule; this only fills the error the caller sees.
const FIRST_PAUSE_SECONDS: i64 = 5 * 60;
/// `KeyOutcome::Unavailable` reason when the search answers for another
/// academic year (the next year opens on the school's schedule).
pub const YEAR_NOT_OFFERED: &str =
    "教務Web did not return the requested academic year. The current year is available after sign-in.";

/// Whole-department rows from the official course search.
#[derive(Clone, Debug)]
pub struct CatalogRows {
    /// Rows in the course shape `DataStore::catalog_upsert_offerings` merges.
    pub courses: Vec<serde_json::Value>,
    pub search_options: Vec<OfficialFilterOption>,
    /// The school's per-search cap was reached (or another page remained):
    /// split the search further to read everything.
    pub truncated: bool,
}

/// Cookies the server set for one detached 教務 session: its own ASP.NET
/// session and language. Requests carry the WebView's sign-in cookies with
/// these in place of the WebView's session cookies, and nothing the server
/// sets is written back, so switching this session to English never reaches
/// a 教務 page the person has open.
#[derive(Default)]
struct Detached(Mutex<Vec<(String, String)>>);

/// Cookies a detached session never borrows from the WebView.
const SESSION_COOKIES: [&str; 2] = ["ASP.NET_SessionId", "Language"];

impl Detached {
    fn cookies(&self) -> std::sync::MutexGuard<'_, Vec<(String, String)>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The WebView's `Cookie` header for this session.
    fn header(&self, shared: &str) -> String {
        let own = self.cookies();
        shared
            .split("; ")
            .filter(|pair| {
                let name = pair.split('=').next().unwrap_or("");
                !pair.is_empty()
                    && !SESSION_COOKIES.contains(&name)
                    && !own.iter().any(|(owned, _)| owned == name)
            })
            .map(str::to_owned)
            .chain(own.iter().map(|(name, value)| format!("{name}={value}")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Keeps what the server set, for this session only.
    fn store(&self, set_cookies: &[String]) {
        let mut own = self.cookies();
        for raw in set_cookies {
            let Some((name, rest)) = raw.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = rest.split(';').next().unwrap_or("").trim();
            let expired = value.is_empty() || raw.to_ascii_lowercase().contains("max-age=0");
            own.retain(|(owned, _)| owned != name);
            if !expired {
                own.push((name.to_owned(), value.to_owned()));
            }
        }
    }
}

/// What the course search is asked for. Every field maps to a query
/// parameter of the search URL; empty fields are left out.
#[derive(Clone, Debug, Default)]
pub struct CatalogSearch {
    /// Course code or code prefix (`ShinkokuNoS`).
    pub code: String,
    /// Course name words (`Name`).
    pub name: String,
    /// Instructor name (`KyouinName`).
    pub teacher: String,
    /// One quarter, 1–4 (`Quarters`); 0 searches every quarter.
    pub quarter: u8,
}

#[derive(Clone)]
pub struct KyomuClient {
    http: Http,
    cookies: Arc<dyn SessionCookies>,
    sign_in: SignInGate,
    notice: Arc<Mutex<Option<String>>>,
}

/// How the school answered a request, from its status and `Location` alone.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Reply {
    Page,
    /// Redirected to GetAccess: the 教務 session is gone.
    SessionExpired,
    Unexpected(u16),
}

pub(crate) fn classify(status: u16, location: Option<&str>) -> Reply {
    match status {
        200 => Reply::Page,
        300..=399 if location.is_some_and(is_sign_in_redirect) => Reply::SessionExpired,
        status => Reply::Unexpected(status),
    }
}

/// An expired session answers with a redirect to GetAccess
/// (`/GetAccess/ResourceList`, observed live). A relative redirect stays on
/// 教務Web and is not a sign-in signal.
fn is_sign_in_redirect(location: &str) -> bool {
    Url::parse(location.trim()).is_ok_and(|url| {
        url.host_str() == Some(GETACCESS_HOST)
            && url.path().to_ascii_lowercase().starts_with("/getaccess/")
    })
}

enum Fetched {
    Html(String),
    SignedOut,
    /// A redirect within 教務Web.
    Redirected(u16),
}

fn invalid(error: impl std::fmt::Display) -> NetError {
    NetError::Invalid(error.to_string())
}

/// The official course search URL, with the query
/// in its parameters.
pub(crate) fn search_url(year: u16, query: &CatalogSearch) -> Result<Url, NetError> {
    if !(2020..=2200).contains(&year)
        || [&query.code, &query.name, &query.teacher]
            .iter()
            .any(|value| value.chars().count() > 200 || value.chars().any(char::is_control))
    {
        return Err(NetError::Invalid("Invalid official course search.".into()));
    }
    let mut url = Url::parse(&format!(
        "https://{}{}",
        kyomu::HOST,
        crate::catalog::KYOMU_SEARCH_PATH
    ))
    .map_err(invalid)?;
    url.query_pairs_mut().extend_pairs([
        ("back", "0"),
        ("ZengakuSenmonAll", "True"),
        ("TermUnit", &format!("{year}2")),
        ("_search", "True"),
    ]);
    for (key, value) in [
        ("ShinkokuNoS", &query.code),
        ("Name", &query.name),
        ("KyouinName", &query.teacher),
    ] {
        if !value.is_empty() {
            url.query_pairs_mut().append_pair(key, value);
        }
    }
    if (1..=4).contains(&query.quarter) {
        url.query_pairs_mut()
            .append_pair("Quarters", &query.quarter.to_string());
    }
    Ok(url)
}

impl KyomuClient {
    pub fn new(http: Http, cookies: Arc<dyn SessionCookies>, sign_in: SignInGate) -> Self {
        Self {
            http,
            cookies,
            sign_in,
            notice: Arc::new(Mutex::new(None)),
        }
    }

    /// The school's text from the last maintenance page, until a later read
    /// succeeds. A maintenance page is reported as `NetError::HostPaused`.
    pub fn maintenance_notice(&self) -> Option<String> {
        self.notice.lock().ok().and_then(|notice| notice.clone())
    }

    /// One of the six pages in `school_parser::kyomu::PAGES`.
    ///
    /// `Complete(payload)` carries a payload `parse_payload` accepted: status
    /// `"ok"` (persist with `apply_kyomu_payload`) or `"unavailable"`, the
    /// school saying in words that the page is not offered to this account
    /// (record with `record_kyomu_unavailable`). An incomplete or unfamiliar
    /// page is `NetError::Invalid` with the reader's reason.
    pub async fn page(&self, kind: &'static str) -> Result<KeyOutcome<KyomuPayload>, NetError> {
        let url = kyomu::page_url(kind)
            .and_then(|url| Url::parse(&url).ok())
            .ok_or_else(|| NetError::Invalid(format!("Unknown 教務Web page: {kind}")))?;
        let url = &url;
        let read = self
            .sign_in
            .signed_in(SchoolAuthProfile::TokyoTech, move || self.read_page(url))
            .await?;
        Ok(read.map_or(KeyOutcome::SignInRequired, KeyOutcome::Complete))
    }

    /// Whole-department official catalog: GET the search for one code
    /// prefix, then one ViewState postback to the largest page size.
    /// A prefix over the school's per-search cap is read again one quarter
    /// at a time; still `truncated` means some rows were not read.
    pub async fn catalog(
        &self,
        year: u16,
        code_prefix: &str,
    ) -> Result<KeyOutcome<CatalogRows>, NetError> {
        self.catalog_in(year, code_prefix, None).await
    }

    /// The department's English titles and romanized instructors, read as
    /// [`Self::catalog`] reads (rows in the `search::read_english` shape) in
    /// a detached session switched to English. The WebView's own session,
    /// and any 教務 page the person has open, stays as it is.
    pub async fn catalog_english(
        &self,
        year: u16,
        code_prefix: &str,
    ) -> Result<KeyOutcome<CatalogRows>, NetError> {
        let url = search_url(
            year,
            &CatalogSearch {
                code: code_prefix.into(),
                ..CatalogSearch::default()
            },
        )?;
        let url = &url;
        let started = self
            .sign_in
            .signed_in(SchoolAuthProfile::TokyoTech, move || async move {
                let english = Detached::default();
                Ok::<_, NetError>(match self.to_english(url, &english).await? {
                    Session::Live(()) => Session::Live(english),
                    Session::Gone => Session::Gone,
                })
            })
            .await?;
        match started {
            Some(english) => self.catalog_in(year, code_prefix, Some(&english)).await,
            None => Ok(KeyOutcome::SignInRequired),
        }
    }

    /// `english` reads the English page in that detached session; `None`
    /// reads the Japanese page in the WebView's session.
    async fn catalog_in(
        &self,
        year: u16,
        code_prefix: &str,
        english: Option<&Detached>,
    ) -> Result<KeyOutcome<CatalogRows>, NetError> {
        let whole = CatalogSearch {
            code: code_prefix.into(),
            ..CatalogSearch::default()
        };
        let first = match self.search_in(year, &whole, english).await? {
            KeyOutcome::Complete(rows) if rows.truncated => rows,
            other => return Ok(other),
        };
        let mut seen = std::collections::HashSet::new();
        let mut merged = CatalogRows {
            courses: Vec::new(),
            search_options: first.search_options,
            truncated: false,
        };
        for quarter in 1..=4 {
            let part = CatalogSearch {
                quarter,
                ..whole.clone()
            };
            let rows = match self.search_in(year, &part, english).await? {
                KeyOutcome::Complete(rows) => rows,
                other => return Ok(other),
            };
            merged.truncated |= rows.truncated;
            // A course of several quarters is listed once per quarter.
            for course in rows.courses {
                let id = course
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                if id.is_none_or(|id| seen.insert(id)) {
                    merged.courses.push(course);
                }
            }
        }
        Ok(KeyOutcome::Complete(merged))
    }

    /// The official course search for any query the URL can carry.
    pub async fn search(
        &self,
        year: u16,
        query: &CatalogSearch,
    ) -> Result<KeyOutcome<CatalogRows>, NetError> {
        self.search_in(year, query, None).await
    }

    async fn search_in(
        &self,
        year: u16,
        query: &CatalogSearch,
        english: Option<&Detached>,
    ) -> Result<KeyOutcome<CatalogRows>, NetError> {
        let url = &search_url(year, query)?;
        let read = self
            .sign_in
            .signed_in(SchoolAuthProfile::TokyoTech, move || {
                self.read_search(year, url, english)
            })
            .await?;
        Ok(read.unwrap_or(KeyOutcome::SignInRequired))
    }

    async fn read_page(&self, url: &Url) -> Result<Session<KyomuPayload>, NetError> {
        let html = match self
            .send(None, Method::GET, url, MENU, Body::None, Limits::page())
            .await?
        {
            Fetched::Html(html) => html,
            Fetched::SignedOut => return Ok(Session::Gone),
            Fetched::Redirected(status) => return Err(NetError::Http(status)),
        };
        let payload = kyomu::extract_page(url.as_str(), &html);
        match payload.status.as_str() {
            "maintenance" => return Err(self.maintenance(payload.message)),
            "login" => return Ok(Session::Gone),
            _ => {}
        }
        // The same trust boundary every 教務 payload has always passed:
        // source identity, completeness, size and record caps.
        let raw = serde_json::to_string(&payload).map_err(invalid)?;
        match kyomu::parse_payload(&raw).map_err(invalid)? {
            (KyomuStage::Ready | KyomuStage::Unavailable, Some(payload)) => {
                self.clear_notice();
                Ok(Session::Live(payload))
            }
            (KyomuStage::ReconnectRequired, _) => Ok(Session::Gone),
            _ => Err(NetError::Invalid("kyomu extraction was incomplete".into())),
        }
    }

    async fn read_search(
        &self,
        year: u16,
        url: &Url,
        english: Option<&Detached>,
    ) -> Result<Session<KeyOutcome<CatalogRows>>, NetError> {
        let parse = match english {
            None => search::read_search,
            Some(_) => search::read_english,
        };
        let fetched_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let html = match self
            .send(english, Method::GET, url, MENU, Body::None, Limits::page())
            .await?
        {
            Fetched::Html(html) => html,
            Fetched::SignedOut => return Ok(Session::Gone),
            Fetched::Redirected(status) => return Err(NetError::Http(status)),
        };
        let read = match parse(&html, true, &fetched_at).map_err(invalid)? {
            SearchRead::Expand(fields) => {
                // A 1000-row page is about 3 MB.
                let html = match self
                    .send(
                        english,
                        Method::POST,
                        url,
                        url.as_str(),
                        Body::Form(fields),
                        Limits::bulk(),
                    )
                    .await?
                {
                    Fetched::Html(html) => html,
                    Fetched::SignedOut => return Ok(Session::Gone),
                    Fetched::Redirected(status) => return Err(NetError::Http(status)),
                };
                parse(&html, false, &fetched_at).map_err(invalid)?
            }
            read => read,
        };
        let results = match read {
            SearchRead::Results(results) => results,
            SearchRead::Maintenance(notice) => return Err(self.maintenance(notice)),
            SearchRead::Expand(_) => {
                return Err(NetError::Invalid(
                    "The search paging controls changed.".into(),
                ))
            }
        };
        self.clear_notice();
        if results
            .courses
            .iter()
            .any(|course| course["year"].as_u64() != Some(u64::from(year)))
        {
            return Ok(Session::Live(KeyOutcome::Unavailable(YEAR_NOT_OFFERED)));
        }
        Ok(Session::Live(KeyOutcome::Complete(CatalogRows {
            courses: results.courses,
            search_options: results.options,
            truncated: results.truncated,
        })))
    }

    /// Starts the detached session `english` on the page at `url` and posts
    /// its header link to English (`ctl00$ToEnglish`).
    async fn to_english(&self, url: &Url, english: &Detached) -> Result<Session<()>, NetError> {
        let html = match self
            .send(Some(english), Method::GET, url, MENU, Body::None, Limits::page())
            .await?
        {
            Fetched::Html(html) => html,
            Fetched::SignedOut => return Ok(Session::Gone),
            Fetched::Redirected(status) => return Err(NetError::Http(status)),
        };
        let Some(fields) = search::language_postback(&html, search::TO_ENGLISH) else {
            return Err(NetError::Invalid(
                "The 教務Web language links changed.".into(),
            ));
        };
        // The postback answers with the page again or a redirect to it.
        match self
            .send(
                Some(english),
                Method::POST,
                url,
                url.as_str(),
                Body::Form(fields),
                Limits::bulk(),
            )
            .await?
        {
            Fetched::SignedOut => Ok(Session::Gone),
            Fetched::Html(_) | Fetched::Redirected(_) => Ok(Session::Live(())),
        }
    }

    /// One request with the WebView's 教務 cookies, or with those of the
    /// detached session `detached`; never follows redirects.
    async fn send(
        &self,
        detached: Option<&Detached>,
        method: Method,
        url: &Url,
        referer: &str,
        body: Body,
        limits: Limits,
    ) -> Result<Fetched, NetError> {
        let cookie = match self.cookies.cookie_header(url).await {
            Ok(cookie) => detached.map_or(cookie.clone(), |detached| detached.header(&cookie)),
            Err(NetError::SignInRequired) => return Ok(Fetched::SignedOut),
            Err(error) => return Err(error),
        };
        let response = self
            .http
            .send(NetRequest {
                host: Host::Kyomu,
                method,
                url: url.clone(),
                headers: vec![(COOKIE, cookie), (REFERER, referer.into())],
                body,
                limits,
            })
            .await?;
        // ASP.NET and GetAccess may rotate or clear cookies on any answer,
        // the expiry redirect included; the WebView must see the same jar.
        // A detached session keeps what it is sent to itself.
        let set_cookies = response.set_cookies();
        match detached {
            Some(detached) => detached.store(&set_cookies),
            None if !set_cookies.is_empty() => {
                self.cookies.store_set_cookies(url, &set_cookies).await?
            }
            None => {}
        }
        match classify(response.status, response.location().as_deref()) {
            Reply::Page => Ok(Fetched::Html(response.text()?)),
            Reply::SessionExpired => Ok(Fetched::SignedOut),
            Reply::Unexpected(status) if (300..400).contains(&status) => {
                Ok(Fetched::Redirected(status))
            }
            Reply::Unexpected(status) => Err(NetError::Http(status)),
        }
    }

    /// A maintenance page pauses the whole host; the notice is kept for the
    /// status the shells show.
    fn maintenance(&self, notice: String) -> NetError {
        self.http.report_maintenance(Host::Kyomu);
        if let Ok(mut slot) = self.notice.lock() {
            *slot = Some(notice);
        }
        NetError::HostPaused {
            host: Host::Kyomu,
            until_unix: chrono::Utc::now().timestamp() + FIRST_PAUSE_SECONDS,
        }
    }

    fn clear_notice(&self) {
        if let Ok(mut slot) = self.notice.lock() {
            *slot = None;
        }
    }
}

#[cfg(test)]
#[path = "../tests/net__kyomu.rs"]
mod tests;
