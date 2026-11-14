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
use crate::net::Http;
use crate::ports::{vault_key, SecretError, SecretVault, VAULT_SERVICE};
use crate::school_auth::SchoolAuthService;
use crate::school_session::{Attempt, SchoolSessions, Site};
use serde_json::Map;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
    transport: &TokenTransport,
    attempt: &Attempt,
    function: MoodleFunction,
    args: Value,
) -> Result<Value, String> {
    transport
        .call_bulk(attempt, function, &args)
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
    let auth = Arc::new(SchoolAuthService::new(NoVault));
    // Admitted: the survey's own auth holds no credentials, and reads are
    // admitted only for a configured account.
    let sessions = SchoolSessions::ready(Some(Http::new().unwrap()), auth.clone());
    let client = Arc::new(MoodleClient::new(auth, sessions.credential_signal()));
    // A detached attempt runs in account generation 0, the token's.
    client.set(
        0,
        super::super::TokenState::Held {
            token: stored_token(),
            unavailable: Default::default(),
        },
    );
    let attempt = Attempt::detached(&sessions, Site::Lms, Default::default());
    let transport = TokenTransport {
        client,
        relaunched: AtomicBool::new(true),
    };
    let now = chrono::Utc::now().timestamp();

    // What the service grants.
    let info = transport
        .call(&attempt, MoodleFunction::SiteInfo, &json!({}))
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
                &attempt,
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
            &attempt,
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
            let attempt = &attempt;
            async move {
                let shape = match read_or_code(
                    transport,
                    attempt,
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
            &attempt,
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
        let attempt = &attempt;
        async move {
            let attempts = read_or_code(
                transport,
                attempt,
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
                    attempt,
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

const MAINTENANCE_PAGE: &str = r#"<html><body id="page-maintenance-message" class="pagelayout-maintenance">
    <h2>Site under maintenance</h2><div class="maintenance_message">Back at 18:00</div></body></html>"#;

// During maintenance the LMS answers HTML wherever JSON was asked for: the
// cookie AJAX with 200 (which read as a lost session) and REST with 503 (a
// bare HTTP failure). Both are the site's maintenance: its host pauses with
// the notice, and no verdict on the session or the token follows. A page that
// only mentions maintenance stays what it was.
#[tokio::test]
async fn maintenance_html_on_the_ajax_and_rest_paths_pauses_the_host() {
    use crate::school_auth::SchoolAuthProfile::Science;
    let answering = |status, content_type, body| {
        let auth = Arc::new(SchoolAuthService::new(NoVault));
        let sessions = SchoolSessions::with_transport(
            Arc::new(crate::net::one_answer::OneAnswer {
                status,
                content_type,
                body,
            }),
            auth.clone(),
        );
        sessions.assume_configured(Science);
        let attempt = Attempt::detached(&sessions, Site::Lms, Default::default());
        let client = Arc::new(MoodleClient::new(auth, sessions.credential_signal()));
        (sessions, attempt, client)
    };
    let notice = |sessions: &SchoolSessions| {
        sessions
            .snapshot()
            .profile(Science)
            .maintenance
            .clone()
            .and_then(|maintenance| maintenance.notice)
    };
    let cookie = || {
        super::super::ajax::CookieTransport::new(&super::super::Dashboard {
            html: String::new(),
            url: lms_url("my/").unwrap(),
            config: adpt_isct::lms::PageConfig {
                sesskey: Some("key".into()),
                language: String::new(),
            },
            instance: 0,
        })
        .unwrap()
    };
    let token = MoodleToken::new("token".into());
    let rest = MoodleFunction::SiteInfo;
    let ajax = MoodleFunction::CourseformatState;
    let args = json!({"courseid": 1});

    for (status, kind) in [(200, "text/html; charset=utf-8"), (503, "text/html")] {
        let (sessions, attempt, _) = answering(status, kind, MAINTENANCE_PAGE);
        let answer = cookie().call(&attempt, ajax, &args).await;
        assert!(
            matches!(answer, Err(MoodleError::Net(NetError::HostPaused { .. }))),
            "AJAX {status}: {answer:?}"
        );
        assert_eq!(
            notice(&sessions).as_deref(),
            Some("Site under maintenance Back at 18:00")
        );

        let (sessions, attempt, client) = answering(status, kind, MAINTENANCE_PAGE);
        let answer = TokenTransport::new(client)
            .post(&attempt, &token, rest, &json!({}), Limits::page)
            .await;
        assert!(
            matches!(answer, Err(MoodleError::Net(NetError::HostPaused { .. }))),
            "REST {status}: {answer:?}"
        );
        assert!(notice(&sessions).is_some());
    }

    let (sessions, attempt, _) = answering(
        200,
        "text/html",
        "<html><body>Scheduled maintenance of the lab</body></html>",
    );
    let answer = cookie().call(&attempt, ajax, &args).await;
    assert!(
        matches!(answer, Err(MoodleError::SignInRequired)),
        "{answer:?}"
    );
    assert_eq!(notice(&sessions), None);

    let (sessions, attempt, client) = answering(200, "application/json", r#"{"lang":"en"}"#);
    assert!(TokenTransport::new(client)
        .post(&attempt, &token, rest, &json!({}), Limits::page)
        .await
        .is_ok());
    assert_eq!(notice(&sessions), None);
}
