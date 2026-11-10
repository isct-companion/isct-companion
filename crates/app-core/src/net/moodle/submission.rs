//! The submission service's LMS transport over the token REST service: the
//! closed list of writes, the draft upload, and the reads every write is
//! checked against and settled by (`approach/lms-token-rest-transport`).
//!
//! No read path reaches a write. [`MoodleWrite`] never leaves this module,
//! and every method that sends one takes the submission service's
//! `WriteGrant`, which no other code can make. Writes follow the read rules:
//! a POST form with the token in the body, redirects off, TLS verified, the
//! app User-Agent (all from `net::Http`), the error envelope and any warning
//! taken as failure, and no token in a URL or a log line. Reads run as
//! operations of the LMS site session under the write's account permit
//! (`WritePermit`), which the service holds from its preflight read through
//! the upload and the save; a write goes out on one attempt and never runs
//! again. A verification also downloads the submitted files over the token
//! (a POST to `webservice/pluginfile.php`, a read) so the service can
//! compare their content with the snapshot.
use super::{
    envelope::{form_fields, has_warnings, rest_result},
    lms_url,
    rest::TokenTransport,
    MoodleClient, MoodleError, MoodleFunction,
};
use crate::{
    net::{Body, BoxFuture, Host, Limits, NetError, NetRequest, Response},
    school_auth::SchoolAuthProfile,
    school_session::{Attempt, Blocked, Operation, Permit, RunError, SchoolSessions, Site},
    submissions::{
        LmsIdentity, LmsModule, RemoteContent, RemoteFile, RemoteState, RemoteStatus,
        RemoteSubmission, SubmissionRules, SubmissionTransport, TransportError, UploadFile,
        WriteGrant, WritePermit,
    },
    plan::AccountWait,
};
use sha2::{Digest, Sha256};
use adpt_isct::lms::{id_of, LMS_WWWROOT};
use reqwest::Method;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};

/// The closed list of LMS writes, from the write survey (`docs/lms-api`).
#[derive(Clone, Copy)]
enum MoodleWrite {
    /// `webservice/upload.php`: one file into the person's draft area.
    UploadDraftFile,
    /// `mod_assign_save_submission`: the draft area replaces the file area;
    /// with drafts off, this is the submission.
    SaveSubmission,
    /// `mod_assign_remove_submission`: deletes the submission's content.
    RemoveSubmission,
}

impl MoodleWrite {
    const fn name(self) -> &'static str {
        match self {
            Self::UploadDraftFile => "webservice/upload.php",
            Self::SaveSubmission => "mod_assign_save_submission",
            Self::RemoveSubmission => "mod_assign_remove_submission",
        }
    }
}

/// A read's deadline, from its start: a sign-in, a few calls and the
/// submitted files.
const READ_DEADLINE: Duration = Duration::from_secs(5 * 60);
/// A write's attempt outlives its requests, which carry their own limits.
const WRITE_DEADLINE: Duration = Duration::from_secs(60 * 60);

/// The transport `Core` gives the submission service on every shell.
pub(crate) struct LmsSubmissions {
    client: Arc<MoodleClient>,
    sessions: SchoolSessions,
}

/// The submission read, as an operation of the LMS site session.
struct ReadSubmission<'a> {
    submissions: &'a LmsSubmissions,
    module: LmsModule,
    content: bool,
}

impl Operation for ReadSubmission<'_> {
    type Output = RemoteSubmission;
    type Error = MoodleError;

    fn attempt(
        &mut self,
        attempt: Attempt,
    ) -> BoxFuture<'_, Result<RemoteSubmission, MoodleError>> {
        Box::pin(async move {
            self.submissions
                .read_submission(&attempt, self.module, self.content)
                .await
        })
    }
}

/// The user id a removal names, as an operation.
struct ReadUser<'a>(&'a LmsSubmissions);

impl Operation for ReadUser<'_> {
    type Output = u64;
    type Error = MoodleError;

    fn attempt(&mut self, attempt: Attempt) -> BoxFuture<'_, Result<u64, MoodleError>> {
        Box::pin(async move {
            let info = TokenTransport::new(self.0.client.clone())
                .call(&attempt, MoodleFunction::SiteInfo, &json!({}))
                .await?;
            id_of(info.get("userid")).ok_or(MoodleError::Malformed("site user"))
        })
    }
}

/// A read that ended without its answer.
fn run_error(error: RunError<MoodleError>) -> TransportError {
    match error {
        RunError::Op(error) => read_error(error),
        RunError::Blocked(Blocked {
            wait: AccountWait::Person,
            ..
        }) => TransportError::SignInRequired,
        RunError::Blocked(_) | RunError::Offline => {
            TransportError::NotSent("the school cannot be reached now".into())
        }
        RunError::Net(error) => TransportError::NotSent(error.to_string()),
        RunError::Cancelled => TransportError::NotSent("signed out or reset".into()),
        RunError::Deadline => TransportError::NotSent("timed out".into()),
    }
}

impl LmsSubmissions {
    pub(crate) fn new(client: Arc<MoodleClient>, sessions: SchoolSessions) -> Self {
        Self { client, sessions }
    }

    /// The school sessions' permit a write holds.
    fn inner(permit: &WritePermit) -> Result<&Permit, TransportError> {
        permit
            .inner
            .as_ref()
            .ok_or_else(|| TransportError::NotSent("no account permit".into()))
    }

    /// A write's attempt: nothing sent yet. Its token writes need no site
    /// session; only a token launch gets one.
    async fn write_attempt(&self, permit: &Permit) -> Attempt {
        self.sessions
            .write_attempt(
                Site::Lms,
                permit,
                tokio::time::Instant::now() + WRITE_DEADLINE,
            )
            .await
    }

    /// The site, the assignment's settings and its current submission.
    async fn read_submission(
        &self,
        attempt: &Attempt,
        module: LmsModule,
        content: bool,
    ) -> Result<RemoteSubmission, MoodleError> {
        let course = module
            .course
            .ok_or(MoodleError::Malformed("assignment course"))?;
        let transport = TokenTransport::new(self.client.clone());
        let info = transport
            .call(attempt, MoodleFunction::SiteInfo, &json!({}))
            .await?;
        let user_id = id_of(info.get("userid")).ok_or(MoodleError::Malformed("site user"))?;
        let site = info
            .get("siteurl")
            .and_then(Value::as_str)
            .unwrap_or(LMS_WWWROOT)
            .trim_end_matches('/')
            .to_string();
        let listed = transport
            .call_bulk(
                attempt,
                MoodleFunction::AssignGetAssignments,
                &json!({"courseids": [course]}),
            )
            .await?;
        if has_warnings(&listed) {
            return Err(MoodleError::Partial);
        }
        let assignment = listed
            .get("courses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|course| course.get("assignments").and_then(Value::as_array))
            .flatten()
            .find(|assignment| id_of(assignment.get("cmid")) == Some(module.cmid))
            .ok_or(MoodleError::Malformed("assignment not listed"))?;
        let assign_id =
            id_of(assignment.get("id")).ok_or(MoodleError::Malformed("assignment id"))?;
        let status = transport
            .call(
                attempt,
                MoodleFunction::AssignGetSubmissionStatus,
                &json!({"assignid": assign_id}),
            )
            .await?;
        // A partial status could hide the submission a write would replace.
        if status
            .get("warnings")
            .and_then(Value::as_array)
            .is_some_and(|warnings| !warnings.is_empty())
        {
            return Err(MoodleError::Partial);
        }
        let state = remote_state(&status)?;
        let content = if content {
            self.content(attempt, &status, &site).await
        } else {
            None
        };
        Ok(RemoteSubmission {
            identity: LmsIdentity { site, user_id },
            assign_id,
            cmid: module.cmid,
            state,
            rules: rules(assignment, &status, &info),
            content,
        })
    }

    /// Each submitted file's sha256, downloaded over the token from the
    /// address the status names, which must be the LMS's own. `None` when
    /// any file cannot be read: the service then cannot tell content apart.
    async fn content(
        &self,
        attempt: &Attempt,
        status: &Value,
        site: &str,
    ) -> Option<Vec<RemoteContent>> {
        let token = self.client.token(attempt).await.ok()?;
        let mut content = Vec::new();
        for (name, bytes, url) in submitted_files(status)? {
            let url = url::Url::parse(&url).ok()?;
            let own = url::Url::parse(site).ok()?;
            if url.scheme() != "https" || url.host_str() != own.host_str() {
                return None;
            }
            let response = attempt
                .bare(NetRequest {
                    host: Host::Moodle,
                    method: Method::POST,
                    url,
                    headers: Vec::new(),
                    body: Body::Form(vec![("token".into(), token.expose().to_string())]),
                    limits: download_limits(bytes),
                })
                .await
                .ok()?;
            if response.status != 200 || response.body.len() as u64 != bytes {
                return None;
            }
            let mut hasher = Sha256::new();
            hasher.update(&response.body);
            content.push(RemoteContent {
                name,
                sha256: data_encoding::HEXLOWER.encode(&hasher.finalize()),
            });
        }
        Some(content)
    }

    /// Each file into one new draft area: the first with item id 0, the
    /// rest into the item id the first answer names.
    async fn upload_files(
        &self,
        _grant: &WriteGrant,
        permit: &WritePermit,
        files: &[UploadFile],
    ) -> Result<u64, TransportError> {
        let attempt = self.write_attempt(Self::inner(permit)?).await;
        let token = self.client.token(&attempt).await.map_err(read_error)?;
        let url = lms_url(MoodleWrite::UploadDraftFile.name()).map_err(read_error)?;
        let mut draft = 0_u64;
        for file in files {
            // Moodle stores the multipart file name as it is; one it would
            // have to alter never leaves the device.
            if file.name.is_empty()
                || file
                    .name
                    .chars()
                    .any(|character| character.is_control() || "\"\\/".contains(character))
            {
                return Err(TransportError::Rejected("file name".into()));
            }
            let contents = tokio::fs::read(&file.path)
                .await
                .map_err(|error| TransportError::NotSent(error.to_string()))?;
            if contents.len() as u64 != file.bytes {
                return Err(TransportError::NotSent("the snapshot changed".into()));
            }
            let boundary = format!("IsctCompanion{}", uuid::Uuid::new_v4().simple());
            let item = draft.to_string();
            let body = multipart(
                &boundary,
                &[
                    ("token", token.expose()),
                    ("filearea", "draft"),
                    ("filepath", "/"),
                    ("itemid", item.as_str()),
                ],
                &file.name,
                &contents,
            );
            let response = attempt
                .bare(NetRequest {
                    host: Host::Moodle,
                    method: Method::POST,
                    url: url.clone(),
                    headers: Vec::new(),
                    body: Body::Bytes {
                        content_type: format!("multipart/form-data; boundary={boundary}"),
                        bytes: body.into(),
                    },
                    limits: upload_limits(file.bytes),
                })
                .await
                .map_err(sent_error)?;
            let value = answer(&attempt, &response)?;
            let record = value
                .as_array()
                .and_then(|records| records.first())
                .ok_or_else(|| TransportError::Uncertain("upload answer".into()))?;
            if let Some(kind) = record.get("errortype").and_then(Value::as_str) {
                return Err(TransportError::Rejected(kind.chars().take(64).collect()));
            }
            if record.get("filename").and_then(Value::as_str) != Some(file.name.as_str()) {
                return Err(TransportError::Rejected("stored under another name".into()));
            }
            draft = id_of(record.get("itemid"))
                .filter(|item| draft == 0 || *item == draft)
                .ok_or_else(|| TransportError::Uncertain("draft area".into()))?;
        }
        if draft == 0 {
            return Err(TransportError::NotSent("no files".into()));
        }
        Ok(draft)
    }

    /// One write over REST. Moodle checks the token before running the
    /// function, so a refused token means nothing was done.
    async fn post(
        &self,
        _grant: &WriteGrant,
        permit: &WritePermit,
        write: MoodleWrite,
        args: &Value,
    ) -> Result<Value, TransportError> {
        let attempt = self.write_attempt(Self::inner(permit)?).await;
        let token = self.client.token(&attempt).await.map_err(read_error)?;
        let mut form = vec![
            ("wstoken".to_string(), token.expose().to_string()),
            ("wsfunction".to_string(), write.name().to_string()),
            ("moodlewsrestformat".to_string(), "json".to_string()),
        ];
        form.extend(form_fields(args));
        let response = attempt
            .bare(NetRequest {
                host: Host::Moodle,
                method: Method::POST,
                url: lms_url("webservice/rest/server.php").map_err(read_error)?,
                headers: Vec::new(),
                body: Body::Form(form),
                limits: Limits::page(),
            })
            .await
            .map_err(sent_error)?;
        answer(&attempt, &response)
    }

    /// The user id a removal names, from a fresh site info read.
    async fn user_id(&self, permit: &WritePermit) -> Result<u64, TransportError> {
        self.sessions
            .run(
                Site::Lms,
                Self::inner(permit)?,
                tokio::time::Instant::now() + READ_DEADLINE,
                &mut ReadUser(self),
            )
            .await
            .map_err(run_error)
    }
}

impl SubmissionTransport for LmsSubmissions {
    fn permit(&self) -> Result<WritePermit, TransportError> {
        let permit = self
            .sessions
            .permit(SchoolAuthProfile::Science)
            .ok_or_else(|| TransportError::NotSent("signing out or resetting".into()))?;
        Ok(WritePermit {
            generation: permit.generation(),
            inner: Some(permit),
        })
    }

    fn read(
        self: Arc<Self>,
        permit: Arc<WritePermit>,
        module: LmsModule,
        content: bool,
    ) -> BoxFuture<'static, Result<RemoteSubmission, TransportError>> {
        Box::pin(async move {
            self.sessions
                .run(
                    Site::Lms,
                    Self::inner(&permit)?,
                    tokio::time::Instant::now() + READ_DEADLINE,
                    &mut ReadSubmission {
                        submissions: &self,
                        module,
                        content,
                    },
                )
                .await
                .map_err(run_error)
        })
    }

    fn upload<'a>(
        &'a self,
        grant: &'a WriteGrant,
        permit: &'a WritePermit,
        files: &'a [UploadFile],
    ) -> BoxFuture<'a, Result<u64, TransportError>> {
        Box::pin(self.upload_files(grant, permit, files))
    }

    fn save<'a>(
        &'a self,
        grant: &'a WriteGrant,
        permit: &'a WritePermit,
        assign_id: u64,
        draft: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>> {
        Box::pin(async move {
            let value = self
                .post(
                    grant,
                    permit,
                    MoodleWrite::SaveSubmission,
                    &json!({
                        "assignmentid": assign_id,
                        "plugindata": {"files_filemanager": draft},
                    }),
                )
                .await?;
            // A list of warnings; empty is the only success, and the
            // service still reads the submission back.
            match value
                .as_array()
                .or_else(|| value.get("warnings").and_then(Value::as_array))
            {
                Some(warnings) if warnings.is_empty() => Ok(()),
                Some(warnings) => Err(TransportError::Rejected(warning_codes(warnings))),
                None => Err(TransportError::Uncertain("save answer".into())),
            }
        })
    }

    fn remove<'a>(
        &'a self,
        grant: &'a WriteGrant,
        permit: &'a WritePermit,
        assign_id: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>> {
        Box::pin(async move {
            let user_id = self.user_id(permit).await?;
            let value = self
                .post(
                    grant,
                    permit,
                    MoodleWrite::RemoveSubmission,
                    &json!({"userid": user_id, "assignid": assign_id}),
                )
                .await?;
            let warnings = value
                .get("warnings")
                .and_then(Value::as_array)
                .map(|warnings| warning_codes(warnings.as_slice()))
                .filter(|codes| !codes.is_empty());
            match (value.get("status").and_then(Value::as_bool), warnings) {
                (Some(true), None) => Ok(()),
                (Some(_), Some(codes)) => Err(TransportError::Rejected(codes)),
                (Some(false), None) => Err(TransportError::Rejected("not removed".into())),
                (None, _) => Err(TransportError::Uncertain("remove answer".into())),
            }
        })
    }
}

/// The current attempt's submission. Absent means nothing was ever saved;
/// a status this version does not know is refused rather than guessed.
fn remote_state(status: &Value) -> Result<RemoteState, MoodleError> {
    let Some(submission) = status.pointer("/lastattempt/submission") else {
        return Ok(RemoteState {
            attempt: 0,
            status: RemoteStatus::New,
            files: Vec::new(),
            time_modified: 0,
        });
    };
    let state = match submission.get("status").and_then(Value::as_str) {
        Some("new") => RemoteStatus::New,
        Some("draft") => RemoteStatus::Draft,
        Some("submitted") => RemoteStatus::Submitted,
        Some("reopened") => RemoteStatus::Reopened,
        _ => return Err(MoodleError::Malformed("submission status")),
    };
    let mut files = Vec::new();
    for plugin in array(submission.get("plugins")) {
        if plugin.get("type").and_then(Value::as_str) != Some("file") {
            continue;
        }
        for area in array(plugin.get("fileareas")) {
            if area.get("area").and_then(Value::as_str) != Some("submission_files") {
                continue;
            }
            for file in array(area.get("files")) {
                let name = file
                    .get("filename")
                    .and_then(Value::as_str)
                    .ok_or(MoodleError::Malformed("submission file"))?;
                let folder = file
                    .get("filepath")
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .trim_start_matches('/');
                files.push(RemoteFile {
                    name: format!("{folder}{name}"),
                    bytes: number(file.get("filesize")).max(0) as u64,
                });
            }
        }
    }
    Ok(RemoteState {
        attempt: number(submission.get("attemptnumber")).max(0) as u32,
        status: state,
        files,
        time_modified: number(submission.get("timemodified")),
    })
}

/// The settings a write depends on, from the assignment record, the
/// submission status and the site's upload limit.
fn rules(assignment: &Value, status: &Value, info: &Value) -> SubmissionRules {
    let config = |plugin: &str, name: &str| {
        array(assignment.get("configs"))
            .find(|config| {
                config.get("subtype").and_then(Value::as_str) == Some("assignsubmission")
                    && config.get("plugin").and_then(Value::as_str) == Some(plugin)
                    && config.get("name").and_then(Value::as_str) == Some(name)
            })
            .and_then(|config| config.get("value").and_then(Value::as_str))
            .unwrap_or("")
            .to_string()
    };
    let flag = |key: &str| number(assignment.get(key)) == 1;
    let last = status.get("lastattempt");
    let last_flag = |key: &str| {
        last.and_then(|last| last.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let accepting = last
        .and_then(|last| last.get("submissionsenabled"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let cutoff = number(assignment.get("cutoffdate"));
    let extension = number(last.and_then(|last| last.get("extensionduedate")));
    let assignment_limit = config("file", "maxsubmissionsizebytes")
        .parse::<u64>()
        .unwrap_or(0);
    SubmissionRules {
        can_edit: last_flag("canedit"),
        locked: last_flag("locked"),
        drafts: flag("submissiondrafts"),
        statement: flag("requiresubmissionstatement"),
        team: flag("teamsubmission"),
        time_limit: number(assignment.get("timelimit")) > 0,
        files_enabled: config("file", "enabled") == "1" && accepting && !flag("nosubmissions"),
        online_text: config("onlinetext", "enabled") == "1",
        max_files: config("file", "maxfilesubmissions").parse().unwrap_or(0),
        // 0 on the assignment means the site's limit for this user.
        max_file_bytes: if assignment_limit > 0 {
            assignment_limit
        } else {
            number(info.get("usermaxuploadfilesize")).max(0) as u64
        },
        file_types: config("file", "filetypeslist")
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect(),
        // An extension later than the cut-off moves it.
        cutoff: (cutoff > 0).then_some(cutoff.max(extension)),
    }
}

/// The submitted files with their sizes and download addresses, by the
/// same names `remote_state` gives them; `None` when one has no address.
fn submitted_files(status: &Value) -> Option<Vec<(String, u64, String)>> {
    let mut files = Vec::new();
    let Some(submission) = status.pointer("/lastattempt/submission") else {
        return Some(files);
    };
    for plugin in array(submission.get("plugins")) {
        if plugin.get("type").and_then(Value::as_str) != Some("file") {
            continue;
        }
        for area in array(plugin.get("fileareas")) {
            if area.get("area").and_then(Value::as_str) != Some("submission_files") {
                continue;
            }
            for file in array(area.get("files")) {
                let name = file.get("filename").and_then(Value::as_str)?;
                let url = file.get("fileurl").and_then(Value::as_str)?;
                let folder = file
                    .get("filepath")
                    .and_then(Value::as_str)
                    .unwrap_or("/")
                    .trim_start_matches('/');
                files.push((
                    format!("{folder}{name}"),
                    number(file.get("filesize")).max(0) as u64,
                    url.to_string(),
                ));
            }
        }
    }
    Some(files)
}

/// A submitted file's download: its size, plus Moodle's answer.
fn download_limits(bytes: u64) -> Limits {
    Limits {
        max_bytes: usize::try_from(bytes).unwrap_or(usize::MAX).saturating_add(64 * 1024),
        deadline: Duration::from_secs(60 + bytes / (128 * 1024)),
    }
}

fn array(value: Option<&Value>) -> impl Iterator<Item = &Value> {
    value.and_then(Value::as_array).into_iter().flatten()
}

/// A Moodle integer, which some fields send as a string; 0 when absent.
fn number(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_i64().unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// The warning codes of a refused write, for logs; never their messages.
fn warning_codes(warnings: &[Value]) -> String {
    warnings
        .iter()
        .filter_map(|warning| warning.get("warningcode").and_then(Value::as_str))
        .map(|code| code.chars().take(64).collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
}

/// A write's HTTP answer: the error envelope first, as for reads. A gateway
/// refusal (4xx) never reached Moodle; a 5xx or an unreadable body may have.
fn answer(attempt: &Attempt, response: &Response) -> Result<Value, TransportError> {
    // The maintenance page answered instead: the write did not run.
    MoodleClient::maintenance(attempt, response).map_err(read_error)?;
    match response.status {
        200 => rest_result(&response.body).map_err(|error| {
            match MoodleClient::envelope_error(attempt, error) {
                MoodleError::Malformed(what) => TransportError::Uncertain(what.into()),
                other => read_error(other),
            }
        }),
        status @ 400..=499 => Err(TransportError::Rejected(format!("HTTP {status}"))),
        status => Err(TransportError::Uncertain(format!("HTTP {status}"))),
    }
}

/// A failed read, or a write refused before it ran.
fn read_error(error: MoodleError) -> TransportError {
    match error {
        MoodleError::SignInRequired | MoodleError::InvalidToken | MoodleError::InvalidSesskey => {
            TransportError::SignInRequired
        }
        MoodleError::Api(code) | MoodleError::ServiceUnavailable(code) => {
            TransportError::Rejected(code)
        }
        other => TransportError::NotSent(other.to_string()),
    }
}

/// A write whose request failed: only a failure before anything left the
/// device proves nothing reached the LMS.
fn sent_error(error: NetError) -> TransportError {
    match error {
        NetError::Offline
        | NetError::Dns
        | NetError::Tls(_)
        | NetError::HostPaused { .. }
        | NetError::Invalid(_) => TransportError::NotSent(error.to_string()),
        other => TransportError::Uncertain(other.to_string()),
    }
}

/// Time for a file to go up on a slow connection, plus Moodle's answer.
fn upload_limits(bytes: u64) -> Limits {
    Limits {
        max_bytes: 1024 * 1024,
        deadline: Duration::from_secs(60 + bytes / (128 * 1024)),
    }
}

/// A `multipart/form-data` body: `fields`, then the file as `file_1`.
fn multipart(boundary: &str, fields: &[(&str, &str)], name: &str, contents: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(contents.len() + 1024);
    for (field, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file_1\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(contents);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}
