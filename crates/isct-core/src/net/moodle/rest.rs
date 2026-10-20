//! The token transport: `webservice/rest/server.php`, form-encoded, with the
//! mobile token in the body.
use super::{
    envelope::{form_fields, has_warnings, rest_result},
    lms_url,
    mapping::{
        assign_state, attachment_links, detail_modules, folder_tree, forum_article, plain_html,
        quiz_state, structure_from_contents, ModuleInfo,
    },
    part,
    pull::{CourseRead, Pull, WORKERS},
    MoodleClient, MoodleError, MoodleFunction,
};
use crate::net::{Body, Host, Limits, NetError, NetRequest};
use crate::school_auth::MoodleToken;
use crate::school_parser::lms::id_of;
use futures_util::stream::{self, StreamExt};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Courses per `*_by_courses` call; keeps each answer well under the cap.
const COURSE_BATCH: usize = 10;
/// Forum posts read per course and per forum, as before.
const FORUM_POSTS_PER_COURSE: usize = 16;
const FORUM_POSTS_PER_FORUM: usize = 8;
/// Submission states read per pull.
const MAX_STATUSES: usize = 320;

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

/// `core_course_get_contents`: the full outline with section and module
/// text, file links and instance ids. Re-read in full on every course read:
/// only a full read sees new, hidden and removed modules.
pub(super) async fn read_structure(
    _pull: &Pull<'_>,
    transport: &TokenTransport<'_>,
    course: u64,
) -> Result<(Value, HashMap<u64, ModuleInfo>), MoodleError> {
    let contents = transport
        .call_bulk(MoodleFunction::CourseContents, &json!({"courseid": course}))
        .await?;
    let (state, modules) =
        structure_from_contents(&contents).ok_or(MoodleError::Malformed("course contents"))?;
    Ok((state, modules.into_iter().collect()))
}

/// What one batch getter contributes, keyed by cmid.
#[derive(Default)]
struct Details {
    bodies: HashMap<u64, String>,
    resources: HashMap<u64, String>,
    /// cmid → quiz record (grade, decimal points) for the quiz state.
    quizzes: HashMap<u64, Value>,
    /// forum id → (cmid, course id)
    forums: Vec<(u64, u64, u64)>,
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

/// Module details from the per-kind `*_by_courses` getters, forum posts from
/// `mod_forum_get_forum_discussions`, and own work from the submission and
/// attempt functions.
pub(super) async fn read_details(
    pull: &Pull<'_>,
    transport: &TokenTransport<'_>,
    reads: &mut [CourseRead],
    statuses: &mut Vec<Value>,
) {
    let kinds = detail_modules(pull.scope, pull.course);
    let course_ids: Vec<u64> = reads
        .iter()
        .filter(|read| read.state.is_some())
        .map(|read| read.id)
        .collect();
    let has = |kind: &str| {
        reads.iter().any(|read| {
            read.modules
                .values()
                .any(|module| module.module == kind && module.uservisible)
        })
    };
    let module_of: HashMap<u64, (u64, ModuleInfo)> = reads
        .iter()
        .flat_map(|read| {
            read.modules
                .iter()
                .map(move |(cmid, module)| (*cmid, (read.id, module.clone())))
        })
        .collect();
    let detail_total = module_of
        .values()
        .filter(|(_, module)| module.uservisible && kinds.contains(&module.module.as_str()))
        .count();
    let mut detail_done = 0;
    let mut details = Details::default();
    let wanted = |kind: &str| kinds.contains(&kind) && has(kind);
    let fail = |error: MoodleError| {
        log::warn!(target: "isct_core::net::moodle", "LMS detail read failed: {error}");
        pull.miss(part::MATERIAL_DETAIL);
    };
    for chunk in course_ids.chunks(COURSE_BATCH) {
        let ids = json!({"courseids": chunk});
        let mut covered = 0;
        if wanted("page") {
            match transport
                .call_bulk(MoodleFunction::PageGetPagesByCourses, &ids)
                .await
            {
                Ok(value) => {
                    if has_warnings(&value) {
                        pull.miss(part::MATERIAL_DETAIL);
                    }
                    for page in records(&value, "pages") {
                        let Some(cmid) = id_of(page.get("coursemodule")) else {
                            continue;
                        };
                        let content =
                            plain_html(page.get("content").and_then(Value::as_str).unwrap_or(""));
                        details
                            .bodies
                            .insert(cmid, format!("{}\n{content}", intro(page)));
                        covered += 1;
                    }
                }
                Err(error) => fail(error),
            }
        }
        if wanted("resource") {
            match transport
                .call_bulk(MoodleFunction::ResourceGetResourcesByCourses, &ids)
                .await
            {
                Ok(value) => {
                    if has_warnings(&value) {
                        pull.miss(part::MATERIAL_DETAIL);
                    }
                    for resource in records(&value, "resources") {
                        if let Some(cmid) = id_of(resource.get("coursemodule")) {
                            details.bodies.insert(cmid, intro(resource));
                            covered += 1;
                        }
                    }
                }
                Err(error) => fail(error),
            }
        }
        if wanted("folder") {
            match transport
                .call_bulk(MoodleFunction::FolderGetFoldersByCourses, &ids)
                .await
            {
                Ok(value) => {
                    if has_warnings(&value) {
                        pull.miss(part::MATERIAL_DETAIL);
                    }
                    for folder in records(&value, "folders") {
                        let Some(cmid) = id_of(folder.get("coursemodule")) else {
                            continue;
                        };
                        let files = module_of
                            .get(&cmid)
                            .map(|(_, module)| folder_tree(&module.files))
                            .unwrap_or_default();
                        details
                            .bodies
                            .insert(cmid, format!("{}\n{files}", intro(folder)));
                        covered += 1;
                    }
                }
                Err(error) => fail(error),
            }
        }
        if wanted("url") {
            match transport
                .call_bulk(MoodleFunction::UrlGetUrlsByCourses, &ids)
                .await
            {
                Ok(value) => {
                    if has_warnings(&value) {
                        pull.miss(part::MATERIAL_DETAIL);
                    }
                    for link in records(&value, "urls") {
                        let Some(cmid) = id_of(link.get("coursemodule")) else {
                            continue;
                        };
                        details.bodies.insert(cmid, intro(link));
                        if let Some(external) = link
                            .get("externalurl")
                            .and_then(Value::as_str)
                            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
                        {
                            details.resources.insert(cmid, external.to_string());
                        }
                        covered += 1;
                    }
                }
                Err(error) => fail(error),
            }
        }
        if wanted("assign") {
            match transport
                .call_bulk(MoodleFunction::AssignGetAssignments, &ids)
                .await
            {
                Ok(value) => {
                    if has_warnings(&value) {
                        pull.miss(part::MATERIAL_DETAIL);
                    }
                    for course in records(&value, "courses") {
                        for assignment in records(course, "assignments") {
                            let Some(cmid) = id_of(assignment.get("cmid")) else {
                                continue;
                            };
                            let attachments = attachment_links(
                                assignment
                                    .get("introattachments")
                                    .or_else(|| assignment.get("introfiles")),
                            );
                            details
                                .bodies
                                .insert(cmid, format!("{}{attachments}", intro(assignment)));
                            covered += 1;
                        }
                    }
                }
                Err(error) => fail(error),
            }
        }
        if has("quiz") {
            match transport
                .call_bulk(MoodleFunction::QuizGetQuizzesByCourses, &ids)
                .await
            {
                Ok(value) => {
                    for quiz in records(&value, "quizzes") {
                        if let Some(cmid) = id_of(quiz.get("coursemodule")) {
                            details.quizzes.insert(cmid, quiz.clone());
                        }
                    }
                }
                Err(error) => fail(error),
            }
        }
        if wanted("forum") {
            match transport
                .call_bulk(MoodleFunction::ForumGetForumsByCourses, &ids)
                .await
            {
                Ok(value) => {
                    for forum in value.as_array().into_iter().flatten() {
                        let (Some(id), Some(cmid), Some(course)) = (
                            id_of(forum.get("id")),
                            id_of(forum.get("cmid")),
                            id_of(forum.get("course")),
                        ) else {
                            continue;
                        };
                        details.bodies.insert(cmid, intro(forum));
                        details.forums.push((id, cmid, course));
                    }
                }
                Err(error) => fail(error),
            }
        }
        detail_done = (detail_done + covered).min(detail_total);
        let course = reads.iter().find(|read| chunk.contains(&read.id));
        pull.report(
            "material",
            detail_done,
            detail_total,
            course.map(|read| (read.id, read.title.as_str(), read.term.as_str())),
            "",
        );
    }

    // Announcements: the newest discussions of each forum, within a
    // per-course share.
    let mut share: HashMap<u64, usize> = HashMap::new();
    let mut forum_reads = Vec::new();
    for (forum, cmid, course) in details.forums.drain(..) {
        let left = share.entry(course).or_insert(FORUM_POSTS_PER_COURSE);
        let count = (*left).min(FORUM_POSTS_PER_FORUM);
        *left -= count;
        if count > 0 {
            forum_reads.push((forum, cmid, count));
        }
    }
    let posts: Vec<(u64, Option<String>)> = stream::iter(forum_reads)
        .map(move |(forum, cmid, count)| async move {
            let args = json!({"forumid": forum, "sortorder": -1, "page": 0, "perpage": count});
            match transport
                .call(MoodleFunction::ForumGetForumDiscussions, &args)
                .await
            {
                Ok(value) => {
                    if has_warnings(&value) {
                        pull.miss(part::MATERIAL_DETAIL);
                    }
                    let articles: Vec<String> = records(&value, "discussions")
                        .take(count)
                        .filter_map(forum_article)
                        .collect();
                    (cmid, Some(articles.join("\n")))
                }
                Err(error) => {
                    log::warn!(target: "isct_core::net::moodle", "LMS forum read failed: {error}");
                    pull.miss(part::MATERIAL_DETAIL);
                    (cmid, None)
                }
            }
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    for (cmid, articles) in posts {
        if let Some(articles) = articles {
            let body = details.bodies.entry(cmid).or_default();
            body.push_str(&articles);
        }
    }

    // Write bodies and links onto the state, cached ones stay where none came.
    for read in reads.iter_mut() {
        for cmid in read.modules.keys().copied().collect::<Vec<_>>() {
            let visible = read.modules.get(&cmid).is_some_and(|module| {
                module.uservisible && kinds.contains(&module.module.as_str())
            });
            if !visible {
                continue;
            }
            if let Some(body) = details
                .bodies
                .get(&cmid)
                .and_then(|body| pull.budget.take(body))
            {
                read.set_cm(cmid, "bodyHtml", body.into());
            }
            if let Some(resource) = details.resources.get(&cmid) {
                read.set_cm(cmid, "resourceUrl", resource.clone().into());
            }
        }
    }

    // Own work: every assignment and quiz of the read courses.
    let work: Vec<(u64, u64, ModuleInfo)> = reads
        .iter()
        .flat_map(|read| {
            read.modules
                .iter()
                .filter(|(_, module)| matches!(module.module.as_str(), "assign" | "quiz"))
                .map(move |(cmid, module)| (read.id, *cmid, module.clone()))
        })
        .filter(|(_, _, module)| module.instance > 0)
        .take(MAX_STATUSES)
        .collect();
    let total = work.len();
    let done = AtomicUsize::new(0);
    let done_ref = &done;
    let quizzes = &details.quizzes;
    let titles: HashMap<u64, (String, String)> = reads
        .iter()
        .map(|read| (read.id, (read.title.clone(), read.term.clone())))
        .collect();
    let titles = &titles;
    let rows: Vec<Option<Value>> = stream::iter(work)
        .map(move |(course, cmid, module)| async move {
            let row = match module.module.as_str() {
                "assign" => transport
                    .call(
                        MoodleFunction::AssignGetSubmissionStatus,
                        &json!({"assignid": module.instance}),
                    )
                    .await
                    .map(|status| assign_state(cmid, &status)),
                _ => {
                    let attempts = transport
                        .call(
                            MoodleFunction::QuizGetUserAttempts,
                            &json!({"quizid": module.instance, "status": "all", "includepreviews": false}),
                        )
                        .await;
                    let best = transport
                        .call(
                            MoodleFunction::QuizGetUserBestGrade,
                            &json!({"quizid": module.instance}),
                        )
                        .await;
                    attempts.and_then(|attempts| {
                        best.map(|best| Some(quiz_state(cmid, &attempts, &best, quizzes.get(&cmid))))
                    })
                }
            };
            let row = row.unwrap_or_else(|error| {
                log::warn!(target: "isct_core::net::moodle", "LMS submission read failed: {error}");
                pull.miss(part::ASSIGN_STATUS);
                None
            });
            let completed = done_ref.fetch_add(1, Ordering::SeqCst) + 1;
            let (title, term) = titles.get(&course).cloned().unwrap_or_default();
            pull.report(
                "assignment",
                completed,
                total,
                Some((course, title.as_str(), term.as_str())),
                &module.name,
            );
            row
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    statuses.extend(rows.into_iter().flatten());
}
