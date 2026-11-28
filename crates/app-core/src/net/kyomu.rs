//! 教務Web over Rust HTTP, on an attempt of its site session
//! (`school_session::Attempt`).
//!
//! These are operations: each GETs the six server-rendered student pages or
//! runs the course search with the attempt's cookies, and parses the HTML in
//! `adpt_isct::kyomu`. Which address of a page an account has is its student
//! menu's answer, read once per site instance. A session found gone is
//! `NetError::SignInRequired`; recovering it is `school_session::run`'s. The English catalog reads on the
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
use std::sync::Arc;
use url::Url;

const GETACCESS_HOST: &str = "portal.nap.gsic.titech.ac.jp";
/// The menu every student page is opened from in the browser. Sent as the
/// Referer only; the page itself is never requested.
const MENU: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
/// `KeyOutcome::Unavailable` reason when the search answers for another
/// academic year (the next year opens on the school's schedule).
pub(crate) const YEAR_NOT_OFFERED: &str =
    "教務Web did not return the requested academic year. The current year is available after sign-in.";

/// Whole-department rows from the official course search.
#[derive(Clone, Debug)]
pub(crate) struct CatalogRows {
    /// Rows in the course shape `DataStore::catalog_upsert_offerings` merges.
    pub courses: Vec<serde_json::Value>,
    pub(crate) search_options: Vec<OfficialFilterOption>,
    /// The school's per-search cap was reached (or another page remained):
    /// split the search further to read everything.
    pub(crate) truncated: bool,
}

/// What the course search is asked for. Every field maps to a query
/// parameter of the search URL; empty fields are left out.
#[derive(Clone, Debug, Default)]
pub(crate) struct CatalogSearch {
    /// Course code or code prefix (`ShinkokuNoS`).
    pub code: String,
    /// Course name words (`Name`).
    pub name: String,
    /// Instructor name (`KyouinName`).
    pub(crate) teacher: String,
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
        300..=399
            if location
                .and_then(target)
                .is_some_and(|url| session_gone(&url)) =>
        {
            Reply::SessionExpired
        }
        status => Reply::Unexpected(status),
    }
}

/// Where a redirect points, resolved against 教務Web (a relative `Location`
/// stays on it).
fn target(location: &str) -> Option<Url> {
    Url::parse(MENU).ok()?.join(location.trim()).ok()
}

/// A session is gone when 教務Web sends the page to GetAccess
/// (`/GetAccess/ResourceList`, observed live: the SSO session expired) or
/// back to its own entry, `Default.aspx` (its own site session expired while
/// GetAccess's lives on). Either way the site's SSO, which enters through
/// `Default.aspx`, builds a new one; a run recovers once, so a page that
/// keeps answering this way still ends the call. Other redirects, GetAccess's
/// menu included, are not a session signal.
fn session_gone(url: &Url) -> bool {
    let path = url.path().to_ascii_lowercase();
    match url.host_str() {
        Some(GETACCESS_HOST) => path.starts_with("/getaccess/"),
        Some(kyomu::HOST) => path == "/titech/default.aspx",
        _ => false,
    }
}

/// A redirect's target without its query, for the log: enough to tell
/// where 教務Web sent a page, never a token it carried.
fn redirect_target(location: Option<&str>) -> String {
    location.and_then(target).map_or_else(
        || "nowhere".into(),
        |url| format!("{}{}", url.host_str().unwrap_or_default(), url.path()),
    )
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

/// The student menu of the 教務Web site instance it was read with: the
/// instance id and the menu. A rebuilt session reads it again; clones
/// share it.
#[derive(Clone, Default)]
pub(crate) struct MenuCache(Arc<tokio::sync::Mutex<Option<ReadMenu>>>);

/// A student menu and the site instance it was read with.
type ReadMenu = (u64, Arc<kyomu::Menu>);

/// This instance's student menu: the cached one, or the student top's,
/// read now.
async fn menu(attempt: &Attempt, cache: &MenuCache) -> Result<Arc<kyomu::Menu>, NetError> {
    let instance = attempt.instance().await?;
    // Held across the read: 教務Web runs one operation per instance anyway.
    let mut slot = cache.0.lock().await;
    if let Some((id, menu)) = slot.as_ref() {
        if *id == instance {
            return Ok(menu.clone());
        }
    }
    let url = Url::parse(kyomu::STUDENT_TOP).map_err(invalid)?;
    let html = send(attempt, Method::GET, &url, MENU, Body::None, Limits::page()).await?;
    let menu = Arc::new(kyomu::Menu::read(url.as_str(), &html).map_err(invalid)?);
    *slot = Some((instance, menu.clone()));
    Ok(menu)
}

/// One of the six pages in `adpt_isct::kyomu::PAGES`, at the address the
/// account's student menu links.
///
/// The payload `parse_payload` accepted: status `"ok"` (persist with
/// `apply_kyomu_payload`) or `"unavailable"`, the school saying in words that
/// the page is not offered to this account (record with
/// `record_kyomu_unavailable`). An incomplete or unfamiliar page is
/// `NetError::Invalid` with the reader's reason; a maintenance page pauses
/// the host (`NetError::HostPaused`).
pub(crate) async fn page(
    attempt: &Attempt,
    cache: &MenuCache,
    kind: &str,
) -> Result<KyomuPayload, NetError> {
    let address = menu(attempt, cache).await?.url(kind).map_err(invalid)?;
    let url = Url::parse(&address).map_err(invalid)?;
    let html = send(attempt, Method::GET, &url, MENU, Body::None, Limits::page()).await?;
    let payload = kyomu::extract_page(url.as_str(), &html);
    // The same trust boundary every 教務 payload has always passed:
    // source identity, completeness, size and record caps.
    let raw = serde_json::to_string(&payload).map_err(invalid)?;
    match kyomu::parse_payload(&raw).map_err(invalid)? {
        (KyomuStage::Ready | KyomuStage::Unavailable, Some(payload)) => Ok(payload),
        _ => Err(NetError::Invalid("kyomu extraction was incomplete".into())),
    }
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
    let fetched_at = crate::clock::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let html = send(attempt, Method::GET, &url, MENU, Body::None, Limits::page()).await?;
    let read = match parse(&html, true, &fetched_at).map_err(invalid)? {
        SearchRead::Expand(fields) => {
            // A 1000-row page is about 3 MB.
            let html = send(
                attempt,
                Method::POST,
                &url,
                url.as_str(),
                Body::Form(fields),
                Limits::bulk(),
            )
            .await?;
            parse(&html, false, &fetched_at).map_err(invalid)?
        }
        read => read,
    };
    let results = match read {
        SearchRead::Results(results) => results,
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
/// The one place a 教務 answer is classified (`d/wm1bwz`): the page, or the
/// session gone (a redirect to the sign-in, or a sign-in form served with
/// 200), the site in maintenance (the host pauses), or an unexpected
/// redirect or status. Every 教務 read starts from what this returns.
async fn send(
    attempt: &Attempt,
    method: Method,
    url: &Url,
    referer: &str,
    body: Body,
    limits: Limits,
) -> Result<String, NetError> {
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
    let html = match classify(response.status, response.location().as_deref()) {
        Reply::Page => response.text()?,
        Reply::SessionExpired => return Err(NetError::SignInRequired),
        Reply::Unexpected(status) if (300..400).contains(&status) => {
            log::warn!(
                target: "app_core::kyomu",
                "教務Web answered {status} to {} with a redirect to {}",
                url.path(),
                redirect_target(response.location().as_deref())
            );
            return Err(NetError::Http(status));
        }
        // A maintenance page may come with an error status.
        Reply::Unexpected(status) => match response.text() {
            Ok(html) if adpt_isct::maintenance_notice(url, &html).is_some() => html,
            _ => return Err(NetError::Http(status)),
        },
    };
    if let Some(notice) = adpt_isct::maintenance_notice(url, &html) {
        return Err(attempt.maintenance(Some(notice)));
    }
    if kyomu::is_login_page(url, &html) {
        return Err(NetError::SignInRequired);
    }
    Ok(html)
}

#[cfg(test)]
#[path = "../tests/net__kyomu.rs"]
mod tests;
