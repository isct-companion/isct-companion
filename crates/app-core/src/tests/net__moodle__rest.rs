//! Live, read-only survey of what the school's Moodle offers for writing
//! submissions, behind `docs/lms-api` and slotledger `lms-write-api-survey`.
//!
//! Ignored: it reads the desktop's stored token from the login keychain,
//! which macOS asks the person to allow. Run it by hand:
//!
//! ```sh
//! ISCT_LMS_SURVEY_OUT=/path/outside/the/repo.json ISCT_LMS_SURVEY_TERMS=2026-3Q,2026-2Q \
//!   cargo test -p app-core write_surface_survey -- --ignored
//! ```
//!
//! Every request goes through `TokenTransport::call`, which takes only a
//! read-only `MoodleFunction`, so no write can be expressed here. A refused
//! token ends the run: no relaunch, cookie or sign-in is wired in. The
//! summary keeps codes, counts and settings under synthetic ids; no token,
//! names, ids or course text.
use super::*;
use crate::net::{BoxFuture, Http, SessionCookies, SignIn, SignInGate};
use crate::ports::{vault_key, SecretError, SecretVault, VAULT_SERVICE};
use crate::school_auth::{SchoolAuthProfile, SchoolAuthService};
use serde_json::Map;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use url::Url;

struct NoCookies;

impl SessionCookies for NoCookies {
    fn cookie_header<'a>(&'a self, _url: &'a Url) -> BoxFuture<'a, Result<String, NetError>> {
        Box::pin(async { Err(NetError::SignInRequired) })
    }

    fn store_set_cookies<'a>(
        &'a self,
        _url: &'a Url,
        _set_cookie: &'a [String],
    ) -> BoxFuture<'a, Result<(), NetError>> {
        Box::pin(async { Ok(()) })
    }
}

struct NoSignIn;

impl SignIn for NoSignIn {
    fn sign_in<'a>(&'a self, _profile: SchoolAuthProfile) -> BoxFuture<'a, Result<(), NetError>> {
        Box::pin(async { Err(NetError::SignInRequired) })
    }
}

/// Keeps the real keychain out of `SchoolAuthService`.
struct NoVault;

impl SecretVault for NoVault {
    fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
        Err(SecretError::InvalidAccount)
    }

    fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }

    fn delete(&self, _account: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

/// The desktop's stored token, from the item `OsSecretVault` writes in the
/// login keychain. `SchoolAuthService::load_moodle_token` cannot read it
/// here: its owner check opens the synchronized school login, which only the
/// signed app can, and it deletes a record whose owner it cannot match.
fn stored_token() -> MoodleToken {
    let key = vault_key(VAULT_SERVICE, "lms-token-science").unwrap();
    let output = std::process::Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "IsctCompanion",
            "-a",
            &key,
            "-w",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "no stored LMS token was readable");
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    let token = record
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .unwrap();
    MoodleToken::new(token.to_string())
}

fn sha(text: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(text.as_bytes()))
}

fn int(value: &Value, key: &str) -> i64 {
    match value.get(key) {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Value::Bool(flag)) => i64::from(*flag),
        Some(Value::String(text)) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn text<'v>(value: &'v Value, key: &str) -> &'v str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list<'v>(value: &'v Value, key: &str) -> &'v [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Sequential synthetic ids, the docs' convention (courses 1001…).
struct Synthetic {
    next: u64,
    ids: HashMap<u64, u64>,
}

impl Synthetic {
    fn new(base: u64) -> Self {
        Self {
            next: base,
            ids: HashMap::new(),
        }
    }

    fn of(&mut self, real: u64) -> u64 {
        *self.ids.entry(real).or_insert_with(|| {
            self.next += 1;
            self.next
        })
    }
}

/// Where `now` falls in the assignment's dates, with an extension counted.
fn phase(config: &Value, extension: i64, now: i64) -> &'static str {
    let opens = int(config, "allowsubmissionsfromdate");
    let due = int(config, "duedate").max(extension);
    let cutoff = int(config, "cutoffdate");
    if opens > now {
        "before-open"
    } else if cutoff > 0 && now > cutoff.max(extension) {
        "after-cutoff"
    } else if due > 0 && now > due {
        "after-due"
    } else if due > 0 {
        "before-due"
    } else {
        "no-due"
    }
}

/// One submission (own or team) without content: plugin kinds, file counts
/// and types, online text length.
fn submission_shape(submission: &Value) -> Value {
    let mut plugins = Map::new();
    for plugin in list(submission, "plugins") {
        let files: Vec<&Value> = list(plugin, "fileareas")
            .iter()
            .flat_map(|area| list(area, "files"))
            .collect();
        let types: BTreeSet<&str> = files.iter().map(|file| text(file, "mimetype")).collect();
        let chars: usize = list(plugin, "editorfields")
            .iter()
            .map(|field| text(field, "text").chars().count())
            .sum();
        plugins.insert(
            text(plugin, "type").to_string(),
            json!({
                "files": files.len(),
                "mimetypes": types,
                "fileareas": list(plugin, "fileareas").iter().map(|area| text(area, "area")).collect::<Vec<_>>(),
                "editorfields": list(plugin, "editorfields").iter().map(|field| text(field, "name")).collect::<Vec<_>>(),
                "editorChars": chars,
            }),
        );
    }
    json!({
        "status": text(submission, "status"),
        "attemptnumber": int(submission, "attemptnumber"),
        "latest": int(submission, "latest"),
        "timemodifiedSet": int(submission, "timemodified") > 0,
        "timestartedSet": int(submission, "timestarted") > 0,
        "plugins": plugins,
    })
}

fn status_shape(status: &Value, assignment: &Value, now: i64) -> Value {
    let last = status.get("lastattempt").cloned().unwrap_or(Value::Null);
    let extension = int(&last, "extensionduedate");
    let warnings: Vec<&str> = list(status, "warnings")
        .iter()
        .map(|warning| text(warning, "warningcode"))
        .collect();
    let previous: Vec<i64> = list(status, "previousattempts")
        .iter()
        .map(|attempt| int(attempt, "attemptnumber"))
        .collect();
    json!({
        "lastattemptPresent": !last.is_null(),
        "submission": last.get("submission").map(submission_shape),
        "teamsubmission": last.get("teamsubmission").map(submission_shape),
        "submissiongroupSet": int(&last, "submissiongroup") > 0,
        "membersWhoNeedToSubmit": list(&last, "submissiongroupmemberswhoneedtosubmit").len(),
        "usergroups": list(&last, "usergroups").len(),
        "submissionsenabled": last.get("submissionsenabled"),
        "locked": last.get("locked"),
        "graded": last.get("graded"),
        "canedit": last.get("canedit"),
        "caneditowner": last.get("caneditowner"),
        "cansubmit": last.get("cansubmit"),
        "extensionduedateSet": extension > 0,
        "timelimit": last.get("timelimit"),
        "blindmarking": last.get("blindmarking"),
        "gradingstatus": last.get("gradingstatus"),
        "feedbackPresent": status.get("feedback").is_some(),
        "previousattempts": previous,
        "warnings": warnings,
        "phase": phase(assignment, extension, now),
    })
}

/// The assignment's settings that decide whether a submission can change.
fn assignment_config(assignment: &Value, now: i64) -> Value {
    let mut plugins: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    for config in list(assignment, "configs") {
        if text(config, "subtype") == "assignsubmission" {
            plugins
                .entry(text(config, "plugin").to_string())
                .or_default()
                .insert(
                    text(config, "name").to_string(),
                    config.get("value").cloned().unwrap_or(Value::Null),
                );
        }
    }
    let statement = text(assignment, "submissionstatement");
    json!({
        "nosubmissions": int(assignment, "nosubmissions"),
        "submissiondrafts": int(assignment, "submissiondrafts"),
        "requiresubmissionstatement": int(assignment, "requiresubmissionstatement"),
        "statementSha256": (!statement.is_empty()).then(|| sha(statement)),
        "duedateSet": int(assignment, "duedate") > 0,
        "allowsubmissionsfromdateSet": int(assignment, "allowsubmissionsfromdate") > 0,
        "cutoffdateSet": int(assignment, "cutoffdate") > 0,
        "cutoffAfterDue": int(assignment, "cutoffdate") > int(assignment, "duedate"),
        "gradingduedateSet": int(assignment, "gradingduedate") > 0,
        "timelimit": int(assignment, "timelimit"),
        "attemptreopenmethod": text(assignment, "attemptreopenmethod"),
        "maxattempts": int(assignment, "maxattempts"),
        "teamsubmission": int(assignment, "teamsubmission"),
        "requireallteammemberssubmit": int(assignment, "requireallteammemberssubmit"),
        "teamsubmissiongroupingidSet": int(assignment, "teamsubmissiongroupingid") > 0,
        "preventsubmissionnotingroup": int(assignment, "preventsubmissionnotingroup"),
        "blindmarking": int(assignment, "blindmarking"),
        "markingworkflow": int(assignment, "markingworkflow"),
        "sendnotifications": int(assignment, "sendnotifications"),
        "plugins": plugins,
        "phase": phase(assignment, 0, now),
    })
}

fn quiz_config(quiz: &Value, now: i64) -> Value {
    let close = int(quiz, "timeclose");
    json!({
        "attempts": int(quiz, "attempts"),
        "timelimit": int(quiz, "timelimit"),
        "grademethod": int(quiz, "grademethod"),
        "overduehandling": quiz.get("overduehandling"),
        "graceperiod": quiz.get("graceperiod"),
        "browsersecurity": quiz.get("browsersecurity"),
        "hasquestions": quiz.get("hasquestions"),
        "navmethod": quiz.get("navmethod"),
        "preferredbehaviour": quiz.get("preferredbehaviour"),
        "attemptonlast": quiz.get("attemptonlast"),
        "canredoquestions": quiz.get("canredoquestions"),
        "allowofflineattempts": quiz.get("allowofflineattempts"),
        "passwordFieldReturned": quiz.get("password").is_some(),
        "timeopenSet": int(quiz, "timeopen") > 0,
        "timecloseSet": close > 0,
        "closed": close > 0 && now > close,
    })
}

/// `2026-3Q` from the course's `[2026]` shortname suffix and its category,
/// which names the starting quarter (`lms_ingest::quarter_of`).
fn course_term(course: &Value) -> Option<String> {
    let year = crate::net::moodle::mapping::course_year(course)?;
    let quarter = int(course, "coursecategory");
    (1..=4)
        .contains(&quarter)
        .then(|| format!("{year}-{quarter}Q"))
}

async fn read_or_code(
    transport: &TokenTransport<'_>,
    function: MoodleFunction,
    args: Value,
) -> Result<Value, String> {
    transport
        .call_bulk(function, &args)
        .await
        .map_err(|error| error.to_string())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: reads the school LMS with the desktop's stored token"]
async fn write_surface_survey() {
    let out = std::env::var("ISCT_LMS_SURVEY_OUT").expect("ISCT_LMS_SURVEY_OUT");
    let terms: BTreeSet<String> = std::env::var("ISCT_LMS_SURVEY_TERMS")
        .expect("ISCT_LMS_SURVEY_TERMS")
        .split(',')
        .map(|term| term.trim().to_string())
        .collect();
    let client = MoodleClient::new(
        Http::new().unwrap(),
        Arc::new(NoCookies),
        SignInGate::new(Arc::new(NoSignIn)),
        Arc::new(SchoolAuthService::new(NoVault)),
    );
    *client.token.lock().await = Some(stored_token());
    let transport = TokenTransport {
        client: &client,
        relaunched: AtomicBool::new(true),
    };
    let now = chrono::Utc::now().timestamp();

    // What the service grants.
    let info = transport
        .call(MoodleFunction::SiteInfo, &json!({}))
        .await
        .unwrap();
    let functions: BTreeSet<&str> = list(&info, "functions")
        .iter()
        .map(|function| text(function, "name"))
        .collect();

    // Courses and their terms.
    let mut courses = Vec::new();
    let mut offset = 0i64;
    loop {
        let page = transport
            .call(
                MoodleFunction::EnrolledCoursesByTimeline,
                &json!({"classification": "all", "limit": 0, "offset": offset, "sort": "fullname"}),
            )
            .await
            .unwrap();
        let batch = list(&page, "courses").to_vec();
        courses.extend(batch.iter().cloned());
        match page.get("nextoffset").and_then(Value::as_i64) {
            Some(next) if next > offset && !batch.is_empty() => offset = next,
            _ => break,
        }
    }
    let mut course_ids = Synthetic::new(1000);
    let mut term_of: HashMap<u64, Option<String>> = HashMap::new();
    let mut by_term: BTreeMap<String, usize> = BTreeMap::new();
    for course in &courses {
        let term = course_term(course);
        *by_term
            .entry(term.clone().unwrap_or_else(|| "unplaced".into()))
            .or_default() += 1;
        term_of.insert(int(course, "id") as u64, term);
    }
    let all_ids: Vec<u64> = {
        let mut ids: Vec<u64> = term_of.keys().copied().collect();
        ids.sort_unstable();
        ids
    };
    let in_scope = |course: u64| {
        term_of
            .get(&course)
            .and_then(Option::as_ref)
            .is_some_and(|term| terms.contains(term))
    };

    // Every assignment's settings and submission status; `inScope` marks
    // the surveyed terms.
    let assignments = transport
        .call_bulk(
            MoodleFunction::AssignGetAssignments,
            &json!({"courseids": all_ids}),
        )
        .await
        .unwrap();
    let mut assign_ids = Synthetic::new(2000);
    let mut statements: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut work = Vec::new();
    for listed in list(&assignments, "courses") {
        let course = int(listed, "id") as u64;
        for assignment in list(listed, "assignments") {
            let config = assignment_config(assignment, now);
            let statement = text(assignment, "submissionstatement");
            if !statement.is_empty() {
                statements
                    .entry(sha(statement))
                    .or_insert_with(|| (0, plain_html(statement)))
                    .0 += 1;
            }
            work.push((
                course,
                int(assignment, "id") as u64,
                in_scope(course),
                config,
                assignment.clone(),
            ));
        }
    }
    let surveyed: Vec<_> = work
        .iter()
        .map(|(_, id, _, _, assignment)| (*id, assignment.clone()))
        .collect();
    assert!(surveyed.len() <= 400, "too many assignments to survey");
    let statuses: HashMap<u64, Value> = stream::iter(surveyed)
        .map(|(id, assignment)| {
            let transport = &transport;
            async move {
                let shape = match read_or_code(
                    transport,
                    MoodleFunction::AssignGetSubmissionStatus,
                    json!({"assignid": id}),
                )
                .await
                {
                    Ok(status) => status_shape(&status, &assignment, now),
                    Err(code) => json!({"error": code}),
                };
                (id, shape)
            }
        })
        .buffer_unordered(WORKERS)
        .collect()
        .await;
    let assignment_rows: Vec<Value> = work
        .iter()
        .map(|(course, id, scope, config, _)| {
            json!({
                "id": assign_ids.of(*id),
                "course": course_ids.of(*course),
                "term": term_of.get(course).cloned().flatten(),
                "inScope": scope,
                "config": config,
                "status": statuses.get(id),
            })
        })
        .collect();

    // Quizzes: settings for all; attempts and access rules for the terms.
    let quizzes = transport
        .call_bulk(
            MoodleFunction::QuizGetQuizzesByCourses,
            &json!({"courseids": all_ids}),
        )
        .await
        .unwrap();
    let access_offered = functions.contains(MoodleFunction::QuizGetQuizAccessInformation.name());
    let mut quiz_ids = Synthetic::new(4000);
    let quiz_work: Vec<(u64, u64, Value)> = list(&quizzes, "quizzes")
        .iter()
        .map(|quiz| {
            (
                int(quiz, "course") as u64,
                int(quiz, "id") as u64,
                quiz_config(quiz, now),
            )
        })
        .collect();
    let quiz_states: HashMap<u64, Value> = stream::iter(
        quiz_work
            .iter()
            .filter(|(course, _, _)| in_scope(*course))
            .map(|(_, id, _)| *id)
            .collect::<Vec<_>>(),
    )
    .map(|id| {
        let transport = &transport;
        async move {
            let attempts = read_or_code(
                transport,
                MoodleFunction::QuizGetUserAttempts,
                json!({"quizid": id, "status": "all", "includepreviews": false}),
            )
            .await;
            let mut states: BTreeMap<String, usize> = BTreeMap::new();
            if let Ok(attempts) = &attempts {
                for attempt in list(attempts, "attempts") {
                    *states
                        .entry(text(attempt, "state").to_string())
                        .or_default() += 1;
                }
            }
            let access = if access_offered {
                match read_or_code(
                    transport,
                    MoodleFunction::QuizGetQuizAccessInformation,
                    json!({"quizid": id}),
                )
                .await
                {
                    Ok(access) => json!({
                        "activerulenames": access.get("activerulenames"),
                        "canattempt": access.get("canattempt"),
                        "canreviewmyattempts": access.get("canreviewmyattempts"),
                        "preventaccessreasons": list(&access, "preventaccessreasons").len(),
                    }),
                    Err(code) => json!({"error": code}),
                }
            } else {
                Value::Null
            };
            (
                id,
                json!({
                    "attemptStates": states,
                    "attemptsError": attempts.err(),
                    "access": access,
                }),
            )
        }
    })
    .buffer_unordered(WORKERS)
    .collect()
    .await;
    let quiz_rows: Vec<Value> = quiz_work
        .iter()
        .map(|(course, id, config)| {
            json!({
                "id": quiz_ids.of(*id),
                "course": course_ids.of(*course),
                "term": term_of.get(course).cloned().flatten(),
                "inScope": in_scope(*course),
                "config": config,
                "state": quiz_states.get(id),
            })
        })
        .collect();

    let summary = json!({
        "observedAt": chrono::Utc::now().to_rfc3339(),
        "terms": terms,
        "siteInfo": {
            "release": info.get("release"),
            "version": info.get("version"),
            "functionCount": functions.len(),
            "functions": functions,
            "uploadfiles": info.get("uploadfiles"),
            "downloadfiles": info.get("downloadfiles"),
            "usermaxuploadfilesize": info.get("usermaxuploadfilesize"),
            "userquota": info.get("userquota"),
            "usercanmanageownfiles": info.get("usercanmanageownfiles"),
        },
        "courses": {"total": courses.len(), "byTerm": by_term},
        "assignmentWarnings": list(&assignments, "warnings").iter().map(|warning| text(warning, "warningcode")).collect::<Vec<_>>(),
        "assignments": assignment_rows,
        "statements": statements.into_iter().map(|(sha, (count, text))| json!({"sha256": sha, "count": count, "text": text})).collect::<Vec<_>>(),
        "quizWarnings": list(&quizzes, "warnings").iter().map(|warning| text(warning, "warningcode")).collect::<Vec<_>>(),
        "quizzes": quiz_rows,
    });
    std::fs::write(&out, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
}
