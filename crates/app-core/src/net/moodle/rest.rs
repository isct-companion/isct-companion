//! The token transport: `webservice/rest/server.php`, form-encoded, with the
//! mobile token in the body.
use super::{
    envelope::{form_fields, has_warnings, rest_result},
    lms_url,
    mapping::{
        assign_state, attachment_links, folder_tree, forum_article, plain_html, quiz_state,
        structure_from_contents, BodyBudget, ModuleInfo, DETAIL_MODULES,
    },
    reader::{remove_detail, set_detail, MAX_STATES, WORKERS},
    KeyError, MoodleClient, MoodleError, MoodleFunction,
};
use crate::net::{Body, Host, Limits, NetError, NetRequest};
use crate::school_auth::MoodleToken;
use adpt_isct::lms::{id_of, StoredModule};
use futures_util::stream::{self, StreamExt};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Forum posts read per course and per forum, as before.
const FORUM_POSTS_PER_COURSE: usize = 16;
const FORUM_POSTS_PER_FORUM: usize = 8;

pub(super) struct TokenTransport<'a> {
    client: &'a MoodleClient,
    /// One relaunch per pull; a token refused again ends the token path.
    relaunched: AtomicBool,
}

impl<'a> TokenTransport<'a> {
    /// Obtains or loads the token and proves it with `site_info`, before any
    /// fan-out, so a refused token is renewed once rather than per request.
    /// Returns the user's language.
    pub(super) async fn open(client: &'a MoodleClient) -> Result<(Self, String), MoodleError> {
        let transport = Self {
            client,
            relaunched: AtomicBool::new(false),
        };
        let info = transport.call(MoodleFunction::SiteInfo, &json!({})).await?;
        let language = info
            .get("lang")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Ok((transport, language))
    }

    pub(super) async fn call(
        &self,
        function: MoodleFunction,
        args: &Value,
    ) -> Result<Value, MoodleError> {
        self.call_with(function, args, Limits::page).await
    }

    /// For the `*_by_courses` getters, whose answers carry page bodies.
    async fn call_bulk(
        &self,
        function: MoodleFunction,
        args: &Value,
    ) -> Result<Value, MoodleError> {
        self.call_with(function, args, Limits::bulk).await
    }

    async fn call_with(
        &self,
        function: MoodleFunction,
        args: &Value,
        limits: fn() -> Limits,
    ) -> Result<Value, MoodleError> {
        if !function.via_rest() {
            return Err(MoodleError::ServiceUnavailable(function.name().into()));
        }
        let token = self.client.token().await?;
        match self.post(&token, function, args, limits).await {
            Err(MoodleError::InvalidToken) if !self.relaunched.swap(true, Ordering::SeqCst) => {
                let token = self.client.renew_token(&token).await?;
                self.post(&token, function, args, limits).await
            }
            other => other,
        }
    }

    async fn post(
        &self,
        token: &MoodleToken,
        function: MoodleFunction,
        args: &Value,
        limits: fn() -> Limits,
    ) -> Result<Value, MoodleError> {
        let mut form = vec![
            ("wstoken".to_string(), token.expose().to_string()),
            ("wsfunction".to_string(), function.name().to_string()),
            ("moodlewsrestformat".to_string(), "json".to_string()),
            // Text as the web pages show it (multilang and other filters),
            // matching what the AJAX path returns.
            ("moodlewssettingfilter".to_string(), "true".to_string()),
        ];
        form.extend(form_fields(args));
        let response = self
            .client
            .http
            .send(NetRequest {
                host: Host::Moodle,
                method: Method::POST,
                url: lms_url("webservice/rest/server.php")?,
                headers: Vec::new(),
                body: Body::Form(form),
                limits: limits(),
            })
            .await?;
        if response.status != 200 {
            // A gateway refusal is HTML; Moodle's own errors are HTTP 200.
            return Err(MoodleError::Net(NetError::Http(response.status)));
        }
        rest_result(&response.body).map_err(|error| self.client.envelope_error(error))
    }
}

/// `lms.course/{id}` over REST: `core_course_get_contents` in full on every
/// read (only a full read sees new, hidden and removed modules), then module
/// details from the per-kind `*_by_courses` getters and forum discussions.
///
/// A getter is called only when one of its kind's visible modules changed:
/// a module whose content stamp equals the stored one and whose body is
/// stored keeps that body. Assignments and forums list no contents, so their
/// details are always read. Any failed request fails the key.
pub(super) async fn read_course(
    transport: &TokenTransport<'_>,
    course: u64,
    stored: &HashMap<u64, StoredModule>,
    budget: &BodyBudget,
) -> Result<Value, KeyError> {
    let function = MoodleFunction::CourseContents;
    let contents = transport
        .call_bulk(function, &json!({"courseid": course}))
        .await
        .map_err(KeyError::at(function.name()))?;
    let (mut state, modules) = structure_from_contents(&contents)
        .ok_or_else(|| KeyError::new(function.name(), MoodleError::Malformed("course contents")))?;
    let modules: HashMap<u64, ModuleInfo> = modules.into_iter().collect();
    let unchanged = |cmid: &u64, module: &ModuleInfo| {
        module.modified > 0
            && stored
                .get(cmid)
                .is_some_and(|known| known.has_body && known.modified == Some(module.modified))
    };
    let wanted = |kind: &str| {
        modules.iter().any(|(cmid, module)| {
            module.uservisible && module.module == kind && !unchanged(cmid, module)
        })
    };
    let ids = json!({"courseids": [course]});
    let mut bodies: HashMap<u64, String> = HashMap::new();
    let mut resources: HashMap<u64, String> = HashMap::new();
    let mut forums: Vec<(u64, u64)> = Vec::new();
    if wanted("page") {
        let value = bulk(transport, MoodleFunction::PageGetPagesByCourses, &ids).await?;
        for page in records(&value, "pages") {
            if let Some(cmid) = id_of(page.get("coursemodule")) {
                let content = plain_html(page.get("content").and_then(Value::as_str).unwrap_or(""));
                bodies.insert(cmid, format!("{}\n{content}", intro(page)));
            }
        }
    }
    if wanted("resource") {
        let value = bulk(
            transport,
            MoodleFunction::ResourceGetResourcesByCourses,
            &ids,
        )
        .await?;
        for resource in records(&value, "resources") {
            if let Some(cmid) = id_of(resource.get("coursemodule")) {
                bodies.insert(cmid, intro(resource));
            }
        }
    }
    if wanted("folder") {
        let value = bulk(transport, MoodleFunction::FolderGetFoldersByCourses, &ids).await?;
        for folder in records(&value, "folders") {
            let Some(cmid) = id_of(folder.get("coursemodule")) else {
                continue;
            };
            let files = modules
                .get(&cmid)
                .map(|module| folder_tree(&module.files))
                .unwrap_or_default();
            bodies.insert(cmid, format!("{}\n{files}", intro(folder)));
        }
    }
    if wanted("url") {
        let value = bulk(transport, MoodleFunction::UrlGetUrlsByCourses, &ids).await?;
        for link in records(&value, "urls") {
            let Some(cmid) = id_of(link.get("coursemodule")) else {
                continue;
            };
            bodies.insert(cmid, intro(link));
            if let Some(external) = link
                .get("externalurl")
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            {
                resources.insert(cmid, external.to_string());
            }
        }
    }
    if wanted("assign") {
        let value = bulk(transport, MoodleFunction::AssignGetAssignments, &ids).await?;
        for listed in records(&value, "courses") {
            for assignment in records(listed, "assignments") {
                let Some(cmid) = id_of(assignment.get("cmid")) else {
                    continue;
                };
                let attachments = attachment_links(
                    assignment
                        .get("introattachments")
                        .or_else(|| assignment.get("introfiles")),
                );
                bodies.insert(cmid, format!("{}{attachments}", intro(assignment)));
            }
        }
    }
    if wanted("forum") {
        let function = MoodleFunction::ForumGetForumsByCourses;
        let value = transport
            .call_bulk(function, &ids)
            .await
            .map_err(KeyError::at(function.name()))?;
        for forum in value.as_array().into_iter().flatten() {
            if let (Some(id), Some(cmid)) = (id_of(forum.get("id")), id_of(forum.get("cmid"))) {
                bodies.insert(cmid, intro(forum));
                forums.push((id, cmid));
            }
        }
    }

    // Announcements: the newest discussions of each forum, within the
    // course's share.
    let mut left = FORUM_POSTS_PER_COURSE;
    let mut forum_reads = Vec::new();
    for (forum, cmid) in forums {
        let count = left.min(FORUM_POSTS_PER_FORUM);
        left -= count;
        if count > 0 {
            forum_reads.push((forum, cmid, count));
        }
    }
    let posts: Vec<Result<(u64, String), KeyError>> = stream::iter(forum_reads)
        .map(move |(forum, cmid, count)| async move {
            let function = MoodleFunction::ForumGetForumDiscussions;
            let args = json!({"forumid": forum, "sortorder": -1, "page": 0, "perpage": count});
            let value = transport
                .call(function, &args)
                .await
                .map_err(KeyError::at(function.name()))?;
            if has_warnings(&value) {
                return Err(KeyError::new(function.name(), MoodleError::Partial));
            }
            let articles: Vec<String> = records(&value, "discussions")
                .take(count)
                .filter_map(forum_article)
                .collect();
            Ok::<_, KeyError>((cmid, articles.join("\n")))
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    for post in posts {
        let (cmid, articles) = post?;
        bodies.entry(cmid).or_default().push_str(&articles);
    }

    // Bodies and links onto the outline. An unchanged module whose getter
    // was not called keeps its stored body: the outline's description must
    // not replace it.
    for (cmid, module) in &modules {
        if !(module.uservisible && DETAIL_MODULES.contains(&module.module.as_str())) {
            continue;
        }
        match bodies.get(cmid).and_then(|body| budget.take(body)) {
            Some(body) => set_detail(&mut state, "cm", *cmid, "bodyHtml", body.into()),
            None if unchanged(cmid, module) => remove_detail(&mut state, "cm", *cmid, "bodyHtml"),
            None => {}
        }
        if let Some(resource) = resources.get(cmid) {
            set_detail(
                &mut state,
                "cm",
                *cmid,
                "resourceUrl",
                resource.clone().into(),
            );
        }
    }
    Ok(state)
}

/// One `*_by_courses` getter for a single course. Warnings other than an
/// inaccessible course mean its answer may be partial, which fails the key.
async fn bulk(
    transport: &TokenTransport<'_>,
    function: MoodleFunction,
    ids: &Value,
) -> Result<Value, KeyError> {
    let value = transport
        .call_bulk(function, ids)
        .await
        .map_err(KeyError::at(function.name()))?;
    if has_warnings(&value) {
        return Err(KeyError::new(function.name(), MoodleError::Partial));
    }
    Ok(value)
}

fn records<'v>(value: &'v Value, key: &str) -> impl Iterator<Item = &'v Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn intro(record: &Value) -> String {
    plain_html(record.get("intro").and_then(Value::as_str).unwrap_or(""))
}

/// One assignment or quiz whose own state `lms.assign` reads.
enum Work {
    Assign { cmid: u64, id: u64 },
    Quiz { cmid: u64, id: u64, quiz: Value },
}

/// `lms.assign/{id}` over REST: every assignment and quiz of the course, from
/// their `*_by_courses` getters, then each one's submission status or
/// attempts and best grade. Any failed request fails the key.
pub(super) async fn read_assign(
    transport: &TokenTransport<'_>,
    course: u64,
) -> Result<Vec<Value>, KeyError> {
    let ids = json!({"courseids": [course]});
    let assignments = bulk(transport, MoodleFunction::AssignGetAssignments, &ids).await?;
    let quizzes = bulk(transport, MoodleFunction::QuizGetQuizzesByCourses, &ids).await?;
    let mut work = Vec::new();
    for listed in records(&assignments, "courses") {
        for assignment in records(listed, "assignments") {
            if let (Some(cmid), Some(id)) =
                (id_of(assignment.get("cmid")), id_of(assignment.get("id")))
            {
                work.push(Work::Assign { cmid, id });
            }
        }
    }
    for quiz in records(&quizzes, "quizzes") {
        if let (Some(cmid), Some(id)) = (id_of(quiz.get("coursemodule")), id_of(quiz.get("id"))) {
            work.push(Work::Quiz {
                cmid,
                id,
                quiz: quiz.clone(),
            });
        }
    }
    if work.len() > MAX_STATES {
        return Err(KeyError::new(
            MoodleFunction::AssignGetSubmissionStatus.name(),
            MoodleError::Malformed("assignment limit"),
        ));
    }
    let rows: Vec<Result<Option<Value>, KeyError>> = stream::iter(work)
        .map(move |work| async move {
            match work {
                Work::Assign { cmid, id } => {
                    let function = MoodleFunction::AssignGetSubmissionStatus;
                    let status = transport
                        .call(function, &json!({"assignid": id}))
                        .await
                        .map_err(KeyError::at(function.name()))?;
                    Ok::<_, KeyError>(assign_state(cmid, &status))
                }
                Work::Quiz { cmid, id, quiz } => {
                    let function = MoodleFunction::QuizGetUserAttempts;
                    let attempts = transport
                        .call(
                            function,
                            &json!({"quizid": id, "status": "all", "includepreviews": false}),
                        )
                        .await
                        .map_err(KeyError::at(function.name()))?;
                    let function = MoodleFunction::QuizGetUserBestGrade;
                    let best = transport
                        .call(function, &json!({"quizid": id}))
                        .await
                        .map_err(KeyError::at(function.name()))?;
                    Ok(Some(quiz_state(cmid, &attempts, &best, Some(&quiz))))
                }
            }
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    rows.into_iter().filter_map(Result::transpose).collect()
}
