//! The cookie transport: AJAX `lib/ajax/service.php` with the dashboard's
//! sesskey, and course and activity pages read with Rust HTML parsing. Used
//! when the token path is not available.
use super::{
    envelope::ajax_result,
    lms_url,
    mapping::{detail_modules, pick, ModuleInfo},
    part,
    pull::{set_detail, CourseRead, Pull, WORKERS},
    sign_in_redirect, Dashboard, MoodleClient, MoodleError, MoodleFunction,
};
use crate::net::{Body, Limits, NetError};
use crate::school_parser::lms::{
    course_page_bodies, forum_discussion_links, forum_posts, id_of, is_lms_origin, module_body,
    module_page_matches, page_assign_state, parse_resource_page, ResolvedResource, LMS_PATH,
};
use futures_util::stream::{self, StreamExt};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use url::Url;

/// Activity pages read per course and per pull, newest first.
const DETAILS_PER_COURSE: usize = 40;
const MAX_DETAILS: usize = 320;
/// Submission pages read per pull beyond those the detail reads covered.
const MAX_STATUS_PAGES: usize = 60;
const FORUM_POSTS_PER_COURSE: usize = 16;
const FORUM_POSTS_PER_FORUM: usize = 8;
/// Larger activity pages are not read.
const MAX_PAGE_CHARS: usize = 1_000_000;

pub(super) struct CookieTransport<'a> {
    client: &'a MoodleClient,
    sesskey: tokio::sync::Mutex<String>,
    /// One background sign-in per pull.
    signed_in: AtomicBool,
}

impl<'a> CookieTransport<'a> {
    pub(super) fn new(
        client: &'a MoodleClient,
        dashboard: &Dashboard,
    ) -> Result<Self, MoodleError> {
        let sesskey = dashboard
            .config
            .sesskey
            .clone()
            .ok_or(MoodleError::SignInRequired)?;
        Ok(Self {
            client,
            sesskey: tokio::sync::Mutex::new(sesskey),
            signed_in: AtomicBool::new(false),
        })
    }

    pub(super) async fn call(
        &self,
        function: MoodleFunction,
        args: &Value,
    ) -> Result<Value, MoodleError> {
        if !function.via_ajax() {
            return Err(MoodleError::ServiceUnavailable(function.name().into()));
        }
        let sesskey = self.sesskey.lock().await.clone();
        match self.post(&sesskey, function, args).await {
            Err(MoodleError::InvalidSesskey) => {
                let fresh = self.refresh(&sesskey).await?;
                self.post(&fresh, function, args).await
            }
            Err(MoodleError::SignInRequired) if !self.signed_in.swap(true, Ordering::SeqCst) => {
                self.client.ensure_signed_in().await?;
                let fresh = self.refresh(&sesskey).await?;
                self.post(&fresh, function, args).await
            }
            other => other,
        }
    }

    /// A new sesskey from the dashboard, once per stale key.
    async fn refresh(&self, stale: &str) -> Result<String, MoodleError> {
        let mut slot = self.sesskey.lock().await;
        if slot.as_str() != stale {
            return Ok(slot.clone());
        }
        let dashboard = self.client.load_dashboard().await?;
        let fresh = dashboard
            .config
            .sesskey
            .ok_or(MoodleError::SignInRequired)?;
        *slot = fresh.clone();
        Ok(fresh)
    }

    async fn post(
        &self,
        sesskey: &str,
        function: MoodleFunction,
        args: &Value,
    ) -> Result<Value, MoodleError> {
        let mut url = lms_url("lib/ajax/service.php")?;
        url.query_pairs_mut()
            .append_pair("sesskey", sesskey)
            .append_pair("info", function.name());
        let body = json!([{"index": 0, "methodname": function.name(), "args": args}]);
        let response = self
            .client
            .cookie_send(Method::POST, url.clone(), Body::Json(body), Limits::page)
            .await?;
        if (300..400).contains(&response.status) {
            return Err(match response.location() {
                Some(location) if sign_in_redirect(&location, &url) => MoodleError::SignInRequired,
                _ => MoodleError::Net(NetError::Http(response.status)),
            });
        }
        if response.status != 200 {
            return Err(MoodleError::Net(NetError::Http(response.status)));
        }
        // A sign-in page instead of JSON means the session is gone.
        if !response
            .content_type()
            .is_some_and(|kind| kind.contains("json"))
        {
            return Err(MoodleError::SignInRequired);
        }
        ajax_result(&response.body).map_err(|error| self.client.envelope_error(error))
    }
}

/// The course-format state, with teacher text from the course page when that
/// page can be read; the outline alone stands otherwise.
pub(super) async fn read_structure(
    pull: &Pull<'_>,
    transport: &CookieTransport<'_>,
    course: u64,
) -> Result<(Value, HashMap<u64, ModuleInfo>), MoodleError> {
    let raw = transport
        .call(
            MoodleFunction::CourseformatState,
            &json!({"courseid": course}),
        )
        .await?;
    let full: Value = match &raw {
        Value::String(text) => serde_json::from_str::<Value>(text),
        other => serde_json::from_str::<Value>(
            other.get("data").and_then(Value::as_str).unwrap_or(""),
        ),
    }
    .map_err(|_| MoodleError::Malformed("course state"))?;
    let list = |key: &str, keys: &[&str]| -> Vec<Value> {
        full.get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| pick(item, keys))
            .collect()
    };
    let mut state = json!({
        "section": list("section", &["id", "number", "title", "visible", "anchor"]),
        "cm": list("cm", &["id", "name", "module", "url", "sectionid", "visible", "uservisible", "anchor"]),
    });
    let mut page_url = lms_url("course/view.php")?;
    page_url
        .query_pairs_mut()
        .append_pair("id", &course.to_string());
    if let Ok(html) = transport.client.cookie_page(page_url.clone()).await {
        let bodies = course_page_bodies(&html, &page_url, &state);
        for (cmid, body) in bodies.modules {
            if let Some(body) = pull.budget.take(&body) {
                set_detail(&mut state, "cm", cmid, "bodyHtml", body.into());
            }
        }
        for (id, body) in bodies.sections {
            if let Some(body) = pull.budget.take(&body) {
                set_detail(&mut state, "section", id, "bodyHtml", body.into());
            }
        }
    }
    Ok((state, HashMap::new()))
}

/// What one activity page gave.
#[derive(Default)]
struct PageDetail {
    body: Option<String>,
    resource: Option<String>,
    status: Option<Value>,
}

fn view_path(module: &str) -> String {
    format!("{LMS_PATH}mod/{module}/view.php")
}

/// One activity page, as the former reader did: same-origin pages of the
/// six kinds only, a direct redirect to a file is the file, and a page that
/// is not this activity is ignored.
async fn read_page(
    transport: &CookieTransport<'_>,
    shares: &Mutex<HashMap<u64, usize>>,
    course: u64,
    cmid: u64,
    module: &str,
    url: &str,
) -> Result<PageDetail, MoodleError> {
    let mut detail = PageDetail::default();
    let Some(target) = Url::parse(url)
        .ok()
        .filter(|target| is_lms_origin(target) && target.path() == view_path(module))
    else {
        return Ok(detail);
    };
    let response = transport
        .client
        .cookie_send(Method::GET, target.clone(), Body::None, Limits::page)
        .await?;
    if (300..400).contains(&response.status) {
        if let Some(location) = response.location() {
            if sign_in_redirect(&location, &target) {
                return Err(MoodleError::SignInRequired);
            }
            if let Ok(next) = target.join(&location) {
                if is_lms_origin(&next) && next.path().contains("/pluginfile.php/") {
                    detail.resource = Some(next.into());
                } else if module == "url" && next.scheme() == "https" && !is_lms_origin(&next) {
                    detail.resource = Some(next.into());
                }
            }
        }
        return Ok(detail);
    }
    if response.status != 200
        || !response
            .content_type()
            .is_some_and(|kind| kind.contains("text/html"))
    {
        return Ok(detail);
    }
    let html = response.text()?;
    if html.len() > MAX_PAGE_CHARS || !module_page_matches(&html, cmid) {
        return Ok(detail);
    }
    if module == "assign" {
        detail.status = page_assign_state(&html, cmid, "assign");
    }
    if module == "forum" {
        let links = forum_discussion_links(&html, &target);
        let take = {
            let mut shares = shares
                .lock()
                .map_err(|_| MoodleError::Malformed("forum share"))?;
            let left = shares.entry(course).or_insert(FORUM_POSTS_PER_COURSE);
            let take = links.len().min(FORUM_POSTS_PER_FORUM).min(*left);
            *left -= take;
            take
        };
        let mut bodies = Vec::new();
        for link in links.into_iter().take(take) {
            // One failed discussion keeps the other announcements.
            if let Ok(page) = transport.client.cookie_page(link.clone()).await {
                if page.len() <= MAX_PAGE_CHARS {
                    bodies.push(forum_posts(&page, &link));
                }
            }
        }
        detail.body = Some(format!(
            "{}{}",
            module_body(&html, &target, "forum"),
            bodies.join("\n")
        ));
    } else {
        detail.body = Some(module_body(&html, &target, module));
    }
    if matches!(module, "resource" | "url") {
        match parse_resource_page(&html, &target, module) {
            ResolvedResource::File { url } | ResolvedResource::External { url } => {
                detail.resource = Some(url)
            }
            _ => {}
        }
    }
    Ok(detail)
}

/// Module details from their pages, then the submission state of the
/// assignments and quizzes no detail read covered.
pub(super) async fn read_details(
    pull: &Pull<'_>,
    transport: &CookieTransport<'_>,
    reads: &mut [CourseRead],
    statuses: &mut Vec<Value>,
) {
    let kinds = detail_modules(pull.scope, pull.course);
    // Each course gets its own share, newest modules first, so one large
    // course cannot starve the rest; cached details fill the gaps.
    let targets: Vec<(u64, u64, String, String, String)> = reads
        .iter()
        .flat_map(|read| {
            read.cms()
                .into_iter()
                .filter(|cm| {
                    cm.get("uservisible").and_then(Value::as_bool) != Some(false)
                        && cm
                            .get("module")
                            .and_then(Value::as_str)
                            .is_some_and(|module| kinds.contains(&module))
                })
                .rev()
                .take(DETAILS_PER_COURSE)
                .filter_map(|cm| {
                    Some((
                        read.id,
                        id_of(cm.get("id"))?,
                        cm.get("module")?.as_str()?.to_string(),
                        cm.get("url")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        cm.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    ))
                })
                .collect::<Vec<_>>()
        })
        .take(MAX_DETAILS)
        .collect();
    let titles: HashMap<u64, (String, String)> = reads
        .iter()
        .map(|read| (read.id, (read.title.clone(), read.term.clone())))
        .collect();
    let titles = &titles;
    let shares = Mutex::new(HashMap::new());
    let shares = &shares;
    let total = targets.len();
    let done = AtomicUsize::new(0);
    let done = &done;
    let details: Vec<(u64, u64, PageDetail)> = stream::iter(targets)
        .map(move |(course, cmid, module, url, name)| async move {
            let detail = match read_page(transport, shares, course, cmid, &module, &url).await {
                Ok(detail) => detail,
                Err(error) => {
                    log::warn!(target: "isct_core::net::moodle", "LMS page read failed: {error}");
                    pull.miss(part::MATERIAL_DETAIL);
                    PageDetail::default()
                }
            };
            let completed = done.fetch_add(1, Ordering::SeqCst) + 1;
            let (title, term) = titles.get(&course).cloned().unwrap_or_default();
            pull.report(
                "material",
                completed,
                total,
                Some((course, title.as_str(), term.as_str())),
                &name,
            );
            (course, cmid, detail)
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    let mut read_status = HashSet::new();
    for (course, cmid, detail) in details {
        if let Some(status) = detail.status {
            read_status.insert(cmid);
            statuses.push(status);
        }
        let Some(read) = reads.iter_mut().find(|read| read.id == course) else {
            continue;
        };
        if let Some(body) = detail
            .body
            .as_deref()
            .and_then(|body| pull.budget.take(body))
        {
            read.set_cm(cmid, "bodyHtml", body.into());
        }
        if let Some(resource) = detail.resource {
            read.set_cm(cmid, "resourceUrl", resource.into());
        }
    }

    // Submission state as the assignment or quiz page shows it.
    let work: Vec<(u64, u64, String, String)> = reads
        .iter()
        .flat_map(|read| {
            read.cms()
                .into_iter()
                .filter_map(|cm| {
                    let module = cm.get("module")?.as_str()?.to_string();
                    let cmid = id_of(cm.get("id"))?;
                    (matches!(module.as_str(), "assign" | "quiz") && !read_status.contains(&cmid))
                        .then(|| {
                            let name = cm.get("name").and_then(Value::as_str).unwrap_or("");
                            (read.id, cmid, module, name.to_string())
                        })
                })
                .collect::<Vec<_>>()
        })
        .take(MAX_STATUS_PAGES)
        .collect();
    let total = work.len();
    let done = AtomicUsize::new(0);
    let done = &done;
    let rows: Vec<Option<Value>> = stream::iter(work)
        .map(move |(course, cmid, module, name)| async move {
            let row = async {
                let mut url = lms_url(&format!("mod/{module}/view.php"))?;
                url.query_pairs_mut().append_pair("id", &cmid.to_string());
                let html = transport.client.cookie_page(url).await?;
                Ok::<_, MoodleError>(page_assign_state(&html, cmid, &module))
            }
            .await
            .unwrap_or_else(|error| {
                log::warn!(target: "isct_core::net::moodle", "LMS submission page failed: {error}");
                pull.miss(part::ASSIGN_STATUS);
                None
            });
            let completed = done.fetch_add(1, Ordering::SeqCst) + 1;
            let (title, term) = titles.get(&course).cloned().unwrap_or_default();
            pull.report(
                "assignment",
                completed,
                total,
                Some((course, title.as_str(), term.as_str())),
                &name,
            );
            row
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    statuses.extend(rows.into_iter().flatten());
}
