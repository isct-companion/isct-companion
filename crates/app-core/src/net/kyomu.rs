//! 教務Web over Rust HTTP, on an attempt of its site session
//! (`school_session::Attempt`).
//!
//! These are operations: each GETs the six server-rendered student pages or
//! runs the course search with the attempt's cookies, and parses the HTML in
//! `adpt_isct::kyomu`. A session found gone is `NetError::SignInRequired`;
//! recovering it is `school_session::run`'s. The English catalog reads on the
//! English site instance, which was switched to English when it was set up,
//! so the Japanese session never changes language.

use crate::course_query::OfficialFilterOption;
use crate::net::{Body, Host, KeyOutcome, Limits, NetError, NetRequest};
use crate::school_session::{Attempt, Site};
use adpt_isct::kyomu::{
    self,
    search::{self, SearchRead},
    KyomuPayload, KyomuStage,
};
use reqwest::header::REFERER;
use reqwest::Method;
use url::Url;

const GETACCESS_HOST: &str = "portal.nap.gsic.titech.ac.jp";
/// The menu every student page is opened from in the browser. Sent as the
/// Referer only; the page itself is never requested.
const MENU: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
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

/// One of the six pages in `adpt_isct::kyomu::PAGES`.
///
/// The payload `parse_payload` accepted: status `"ok"` (persist with
/// `apply_kyomu_payload`) or `"unavailable"`, the school saying in words that
/// the page is not offered to this account (record with
/// `record_kyomu_unavailable`). An incomplete or unfamiliar page is
/// `NetError::Invalid` with the reader's reason; a maintenance page pauses
/// the host (`NetError::HostPaused`).
pub(crate) async fn page(attempt: &Attempt, kind: &str) -> Result<KyomuPayload, NetError> {
    let url = kyomu::page_url(kind)
        .and_then(|url| Url::parse(&url).ok())
        .ok_or_else(|| NetError::Invalid(format!("Unknown 教務Web page: {kind}")))?;
    let html = match send(attempt, Method::GET, &url, MENU, Body::None, Limits::page()).await? {
        Fetched::Html(html) => html,
        Fetched::SignedOut => return Err(NetError::SignInRequired),
        Fetched::Redirected(status) => return Err(NetError::Http(status)),
    };
    let payload = kyomu::extract_page(url.as_str(), &html);
    match payload.status.as_str() {
        "maintenance" => return Err(attempt.maintenance(Some(payload.message))),
        "login" => return Err(NetError::SignInRequired),
        _ => {}
    }
    // The same trust boundary every 教務 payload has always passed:
    // source identity, completeness, size and record caps.
    let raw = serde_json::to_string(&payload).map_err(invalid)?;
    match kyomu::parse_payload(&raw).map_err(invalid)? {
        (KyomuStage::Ready | KyomuStage::Unavailable, Some(payload)) => Ok(payload),
        (KyomuStage::ReconnectRequired, _) => Err(NetError::SignInRequired),
        _ => Err(NetError::Invalid("kyomu extraction was incomplete".into())),
    }
}

/// Whole-department official catalog: GET the search for one code prefix,
/// then one ViewState postback to the largest page size. A prefix over the
/// school's per-search cap is read again one quarter at a time; still
/// `truncated` means some rows were not read. On the English site the rows
/// are in the `search::read_english` shape.
pub(crate) async fn catalog(
    attempt: &Attempt,
    year: u16,
    code_prefix: &str,
) -> Result<KeyOutcome<CatalogRows>, NetError> {
    let whole = CatalogSearch {
        code: code_prefix.into(),
        ..CatalogSearch::default()
    };
    let first = match search(attempt, year, &whole).await? {
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
        let rows = match search(attempt, year, &part).await? {
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
pub(crate) async fn search(
    attempt: &Attempt,
    year: u16,
    query: &CatalogSearch,
) -> Result<KeyOutcome<CatalogRows>, NetError> {
    let url = search_url(year, query)?;
    let parse = match attempt.site() {
        Site::KyomuEnglish => search::read_english,
        _ => search::read_search,
    };
    let fetched_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let html = match send(attempt, Method::GET, &url, MENU, Body::None, Limits::page()).await? {
        Fetched::Html(html) => html,
        Fetched::SignedOut => return Err(NetError::SignInRequired),
        Fetched::Redirected(status) => return Err(NetError::Http(status)),
    };
    let read = match parse(&html, true, &fetched_at).map_err(invalid)? {
        SearchRead::Expand(fields) => {
            // A 1000-row page is about 3 MB.
            let html = match send(
                attempt,
                Method::POST,
                &url,
                url.as_str(),
                Body::Form(fields),
                Limits::bulk(),
            )
            .await?
            {
                Fetched::Html(html) => html,
                Fetched::SignedOut => return Err(NetError::SignInRequired),
                Fetched::Redirected(status) => return Err(NetError::Http(status)),
            };
            parse(&html, false, &fetched_at).map_err(invalid)?
        }
        read => read,
    };
    let results = match read {
        SearchRead::Results(results) => results,
        SearchRead::Maintenance(notice) => return Err(attempt.maintenance(Some(notice))),
        SearchRead::Expand(_) => {
            return Err(NetError::Invalid(
                "The search paging controls changed.".into(),
            ))
        }
    };
    if results
        .courses
        .iter()
        .any(|course| course["year"].as_u64() != Some(u64::from(year)))
    {
        return Ok(KeyOutcome::Unavailable(YEAR_NOT_OFFERED));
    }
    Ok(KeyOutcome::Complete(CatalogRows {
        courses: results.courses,
        search_options: results.options,
        truncated: results.truncated,
    }))
}

/// One request with the site instance's cookies; never follows redirects.
async fn send(
    attempt: &Attempt,
    method: Method,
    url: &Url,
    referer: &str,
    body: Body,
    limits: Limits,
) -> Result<Fetched, NetError> {
    let response = attempt
        .send(NetRequest {
            host: Host::Kyomu,
            method,
            url: url.clone(),
            headers: vec![(REFERER, referer.into())],
            body,
            limits,
        })
        .await?;
    match classify(response.status, response.location().as_deref()) {
        Reply::Page => Ok(Fetched::Html(response.text()?)),
        Reply::SessionExpired => Ok(Fetched::SignedOut),
        Reply::Unexpected(status) if (300..400).contains(&status) => {
            Ok(Fetched::Redirected(status))
        }
        // A maintenance page may come with an error status: it is the page,
        // for the caller's maintenance check.
        Reply::Unexpected(status) => match response.text() {
            Ok(html) if adpt_isct::maintenance_notice(url, &html).is_some() => {
                Ok(Fetched::Html(html))
            }
            _ => Err(NetError::Http(status)),
        },
    }
}

#[cfg(test)]
#[path = "../tests/net__kyomu.rs"]
mod tests;
