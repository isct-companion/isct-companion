//! The cookie transport: AJAX `lib/ajax/service.php` with the dashboard's
//! sesskey, and course and activity pages read with Rust HTML parsing. Used
//! when the token path is not available.
use super::{
    envelope::ajax_result,
    lms_url,
    mapping::{pick, BodyBudget, DETAIL_MODULES},
    reader::{set_detail, MAX_STATES, WORKERS},
    sign_in_redirect, Dashboard, KeyError, MoodleClient, MoodleError, MoodleFunction,
};
use crate::net::{Body, Limits, NetError};
use crate::school_parser::lms::{
    course_page_bodies, forum_discussion_links, forum_posts, id_of, is_lms_origin, module_body,
    module_page_matches, page_assign_state, parse_resource_page, ResolvedResource, StoredModule,
    LMS_PATH,
};
use futures_util::stream::{self, StreamExt};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use url::Url;

/// Activity pages read per course read, newest first.
const DETAILS_PER_COURSE: usize = 40;
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

/// The course-format outline `{section, cm}` of one course.
async fn course_state(transport: &CookieTransport<'_>, course: u64) -> Result<Value, KeyError> {
    let function = MoodleFunction::CourseformatState;
    let raw = transport
        .call(function, &json!({"courseid": course}))
        .await
        .map_err(KeyError::at(function.name()))?;
    let full: Value = match &raw {
        Value::String(text) => serde_json::from_str::<Value>(text),
        other => {
            serde_json::from_str::<Value>(other.get("data").and_then(Value::as_str).unwrap_or(""))
        }
    }
    .map_err(|_| KeyError::new(function.name(), MoodleError::Malformed("course state")))?;
    let list = |key: &str, keys: &[&str]| -> Vec<Value> {
        full.get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| pick(item, keys))
            .collect()
    };
    Ok(json!({
        "section": list("section", &["id", "number", "title", "visible", "anchor"]),
        "cm": list("cm", &["id", "name", "module", "url", "sectionid", "visible", "uservisible", "anchor"]),
    }))
}

/// The `cm` entries of an outline.
fn cms(state: &Value) -> Vec<Value> {
    state
        .get("cm")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// What one activity page gave.
#[derive(Default)]
struct PageDetail {
    body: Option<String>,
    resource: Option<String>,
}

fn view_path(module: &str) -> String {
    format!("{LMS_PATH}mod/{module}/view.php")
}

/// One activity page, as the former reader did: same-origin pages of the
/// six kinds only, a direct redirect to a file is the file, and a page that
/// is not this activity is ignored. A failed request fails the key.
async fn read_page(
    transport: &CookieTransport<'_>,
    shares: &Mutex<usize>,
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
                let file = is_lms_origin(&next) && next.path().contains("/pluginfile.php/");
                let external = module == "url" && next.scheme() == "https" && !is_lms_origin(&next);
                if file || external {
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
    if module == "forum" {
        let links = forum_discussion_links(&html, &target);
        let take = {
            let mut left = shares
                .lock()
                .map_err(|_| MoodleError::Malformed("forum share"))?;
            let take = links.len().min(FORUM_POSTS_PER_FORUM).min(*left);
            *left -= take;
            take
        };
        let mut bodies = Vec::new();
        for link in links.into_iter().take(take) {
            let page = transport.client.cookie_page(link.clone()).await?;
            if page.len() <= MAX_PAGE_CHARS {
                bodies.push(forum_posts(&page, &link));
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

/// `lms.course/{id}` over the cookie: the course-format outline, teacher
/// text from the course page, then the pages of the newest activities.
///
/// The cookie path has no modification times, so `stored` does not narrow
/// it: as before, each read re-reads the newest activities' pages and the
/// others keep their stored details. Any failed request fails the key.
pub(super) async fn read_course(
    transport: &CookieTransport<'_>,
    course: u64,
    _stored: &HashMap<u64, StoredModule>,
    budget: &BodyBudget,
) -> Result<Value, KeyError> {
    let mut state = course_state(transport, course).await?;
    let mut page_url = lms_url("course/view.php").map_err(KeyError::at("course/view.php"))?;
    page_url
        .query_pairs_mut()
        .append_pair("id", &course.to_string());
    let html = transport
        .client
        .cookie_page(page_url.clone())
        .await
        .map_err(KeyError::at("course/view.php"))?;
    let bodies = course_page_bodies(&html, &page_url, &state);
    for (cmid, body) in bodies.modules {
        if let Some(body) = budget.take(&body) {
            set_detail(&mut state, "cm", cmid, "bodyHtml", body.into());
        }
    }
    for (id, body) in bodies.sections {
        if let Some(body) = budget.take(&body) {
            set_detail(&mut state, "section", id, "bodyHtml", body.into());
        }
    }

    let targets: Vec<(u64, String, String)> = cms(&state)
        .into_iter()
        .filter(|cm| {
            cm.get("uservisible").and_then(Value::as_bool) != Some(false)
                && cm
                    .get("module")
                    .and_then(Value::as_str)
                    .is_some_and(|module| DETAIL_MODULES.contains(&module))
        })
        .rev()
        .take(DETAILS_PER_COURSE)
        .filter_map(|cm| {
            Some((
                id_of(cm.get("id"))?,
                cm.get("module")?.as_str()?.to_string(),
                cm.get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ))
        })
        .collect();
    let shares = Mutex::new(FORUM_POSTS_PER_COURSE);
    let shares = &shares;
    let details: Vec<Result<(u64, PageDetail), KeyError>> = stream::iter(targets)
        .map(move |(cmid, module, url)| async move {
            read_page(transport, shares, cmid, &module, &url)
                .await
                .map(|detail| (cmid, detail))
                .map_err(KeyError::at("mod/view.php"))
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    for detail in details {
        let (cmid, detail) = detail?;
        if let Some(body) = detail.body.as_deref().and_then(|body| budget.take(body)) {
            set_detail(&mut state, "cm", cmid, "bodyHtml", body.into());
        }
        if let Some(resource) = detail.resource {
            set_detail(&mut state, "cm", cmid, "resourceUrl", resource.into());
        }
    }
    Ok(state)
}

/// `lms.assign/{id}` over the cookie: the state each assignment and quiz page
/// of the course shows. Any failed request fails the key.
pub(super) async fn read_assign(
    transport: &CookieTransport<'_>,
    course: u64,
) -> Result<Vec<Value>, KeyError> {
    let state = course_state(transport, course).await?;
    let work: Vec<(u64, String)> = cms(&state)
        .into_iter()
        .filter_map(|cm| {
            let module = cm.get("module")?.as_str()?.to_string();
            let cmid = id_of(cm.get("id"))?;
            matches!(module.as_str(), "assign" | "quiz").then_some((cmid, module))
        })
        .collect();
    if work.len() > MAX_STATES {
        return Err(KeyError::new(
            "mod/view.php",
            MoodleError::Malformed("assignment limit"),
        ));
    }
    let rows: Vec<Result<Option<Value>, KeyError>> = stream::iter(work)
        .map(move |(cmid, module)| async move {
            let mut url =
                lms_url(&format!("mod/{module}/view.php")).map_err(KeyError::at("mod/view.php"))?;
            url.query_pairs_mut().append_pair("id", &cmid.to_string());
            let html = transport
                .client
                .cookie_page(url)
                .await
                .map_err(KeyError::at("mod/view.php"))?;
            Ok::<_, KeyError>(page_assign_state(&html, cmid, &module))
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    rows.into_iter().filter_map(Result::transpose).collect()
}
