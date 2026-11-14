//! LMS writes of assignment submissions, with uniqueness, idempotency and
//! recovery owned by Core, never by the caller
//! (`approach/lms-submission-contract`).
//!
//! A write takes two steps. `prepare` reads what the LMS holds now,
//! snapshots `submit/` by name, size and sha256 (`snapshots`) and returns an
//! immutable [`Preparation`], published with its snapshot in one ledger
//! transaction; the same content against the same LMS state gives the same
//! preparation. `execute` takes the preparation's id with an echo of its
//! summary, the one the MCP client showed the person, and refuses any
//! difference; in one transaction it checks the preparation has not expired
//! and records the write with its snapshot. It reads the LMS again under
//! the account permit it then holds through the upload and the save,
//! refuses when the account, its generation or what the LMS holds moved,
//! and uploads only the snapshot. Core derives the idempotency key itself;
//! anything a caller passes is only a handle. One write per account and
//! assignment runs at a time, its stage is stored before anything is sent,
//! and a write whose outcome is unknown is settled by reading the LMS, never
//! by sending it again.
//!
//! Every LMS read here (preparation, preflight, verification) is a
//! `PlanEngine` read: the engine's holds, lanes and ownership apply, and a
//! verification is never joined with a read from before the write. A write
//! counts as applied only on evidence bound to the account, the attempt and
//! the content: the files the LMS holds are downloaded and their sha256
//! compared. Names and sizes alone are inconclusive, and so is time: an
//! unknown outcome keeps blocking new writes, read less and less often,
//! until the LMS shows one. A settled write commits with the assignment to
//! read again (`refresh_outbox`), which the engine reads past any refresh
//! that started before the write.
//!
//! Nothing can lock the LMS against a browser or another device; checking
//! the baseline again narrows that window and cannot close it.
mod ledger;
mod snapshots;

use crate::{
    data::{DataStore, StoreKind},
    error_code::{Coded, ErrorCode},
    net::BoxFuture,
    plan::{Credential, Held, Lane, PlanHints, ReadRefused, ReadRequest},
    runtime_tasks::RuntimeTasks,
    school_auth::SchoolAuthProfile,
    sync_state::Key,
    workspaces::{AssignmentWorkspaces, HandInError, SubmitFile, WorkspaceError},
};
use ledger::{Claim, Ledger, Refresh};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use snapshots::Snapshots;
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

/// How long a preparation can be executed: time for a client's
/// confirmation, while the LMS state it describes is still recent.
const PREPARATION_TTL: i64 = 60 * 60;
/// An unknown outcome is read again after this, three times longer after
/// each read that could not tell, at most `CHECK_MAX`.
const CHECK_FIRST: i64 = 30;
const CHECK_MAX: i64 = 6 * 60 * 60;
/// A refresh after a settled write that did not land is tried again after
/// this.
const REFRESH_RETRY: i64 = 5 * 60;
/// How long a refresh after a settled write may wait for its read.
const REFRESH_WAIT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// A read of the submission, sign-in and content downloads included.
const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// The LMS reads a submission over the token REST service only.
const TOKEN_PATH: &[Credential] = &[Credential::Token];
/// Settled writes stay as receipts only while their preparation could still
/// be executed, so a repeated execute finds the write it repeats; the ledger
/// keeps no history beyond that.
const RECEIPT_RETENTION: i64 = PREPARATION_TTL;
/// Hex characters of each sha256 the confirmation shows.
const DIGEST_PREFIX: usize = 12;

/// The site and user the transport reads and writes as.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsIdentity {
    /// The Moodle site root.
    pub site: String,
    /// The Moodle user id the site reports.
    pub user_id: u64,
}

/// An assignment's LMS module as its stored row names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsModule {
    pub cmid: u64,
    /// The LMS course id, when the row carries it.
    pub course: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteStatus {
    New,
    Draft,
    Submitted,
    Reopened,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    pub name: String,
    pub bytes: u64,
}

/// A file of the submission as the LMS served it, by its content.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteContent {
    pub name: String,
    pub sha256: String,
}

/// The student's submission of the current attempt, as the LMS reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteState {
    pub attempt: u32,
    pub status: RemoteStatus,
    /// The file area; Core keeps it by name.
    pub files: Vec<RemoteFile>,
    /// Unix seconds. A save moves it; a removal does not.
    pub time_modified: i64,
}

impl RemoteState {
    /// Whether this attempt holds a submission a save would replace.
    fn holds_content(&self) -> bool {
        !self.files.is_empty()
            || matches!(self.status, RemoteStatus::Draft | RemoteStatus::Submitted)
    }
}

/// The assignment settings a write depends on, read with the state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionRules {
    /// `lastattempt.canedit`: the LMS takes a save or removal now.
    pub can_edit: bool,
    pub locked: bool,
    /// `submissiondrafts`: a save stays a draft until submitted for grading.
    pub drafts: bool,
    pub statement: bool,
    pub team: bool,
    pub time_limit: bool,
    pub files_enabled: bool,
    pub online_text: bool,
    /// 0 when the assignment sets none.
    pub max_files: u32,
    /// Per file; 0 when the assignment sets none.
    pub max_file_bytes: u64,
    /// `filetypeslist` entries; empty accepts any.
    pub file_types: Vec<String>,
    /// Unix seconds. After it the submission is final.
    pub cutoff: Option<i64>,
}

/// One assignment as the transport reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSubmission {
    pub identity: LmsIdentity,
    pub assign_id: u64,
    pub cmid: u64,
    pub state: RemoteState,
    pub rules: SubmissionRules,
    /// The files' content, when the read was asked for it and could get
    /// every file; sorted by name.
    pub content: Option<Vec<RemoteContent>>,
}

/// The right to act on the LMS account for one account generation. A write
/// holds one from its preflight read through its upload and save, so the
/// account cannot change between them unnoticed.
pub struct WritePermit {
    /// The account generation it was taken at.
    pub generation: u64,
    /// The school sessions' permit; absent in a test transport.
    pub(crate) inner: Option<crate::school_session::Permit>,
}

/// A snapshot file the transport uploads, read from the app data directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadFile {
    pub name: String,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Why one transport call did not complete.
#[derive(Clone, Debug, thiserror::Error)]
pub enum TransportError {
    /// The request never reached the LMS: offline, a refused connection.
    #[error("The LMS was not reached: {0}")]
    NotSent(String),
    /// The LMS answered that it did not do it, with its warning codes.
    #[error("The LMS refused: {0}")]
    Rejected(String),
    /// The request may have reached the LMS and no answer says what it did:
    /// a timeout, a dropped connection, an unreadable reply.
    #[error("The LMS did not answer: {0}")]
    Uncertain(String),
    /// The token was refused before anything was done.
    #[error("Signing in to the LMS again is needed")]
    SignInRequired,
}

/// Proof that the caller is the submission service. Only this module makes
/// one, so code that can reach a transport still cannot drive its writes.
pub struct WriteGrant {
    _sealed: (),
}

static GRANT: WriteGrant = WriteGrant { _sealed: () };

/// What the submission service needs from the LMS and nothing more. It is
/// apart from `net::moodle::MoodleFunction`, whose closed list stays
/// read-only, and each write takes a [`WriteGrant`], which only
/// [`SubmissionService`] holds.
pub trait SubmissionTransport: Send + Sync {
    /// A permit for the LMS account now.
    fn permit(&self) -> Result<WritePermit, TransportError>;
    /// One assignment's submission, as the LMS holds it now, with the
    /// content of its files when `content` asks for it.
    fn read(
        self: Arc<Self>,
        permit: Arc<WritePermit>,
        module: LmsModule,
        content: bool,
    ) -> BoxFuture<'static, Result<RemoteSubmission, TransportError>>;
    /// Uploads `files` into one new draft area and returns its item id.
    fn upload<'a>(
        &'a self,
        grant: &'a WriteGrant,
        permit: &'a WritePermit,
        files: &'a [UploadFile],
    ) -> BoxFuture<'a, Result<u64, TransportError>>;
    /// Saves the submission from draft area `draft`, which replaces the
    /// whole file area. With drafts off, this is the submission.
    fn save<'a>(
        &'a self,
        grant: &'a WriteGrant,
        permit: &'a WritePermit,
        assign_id: u64,
        draft: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>>;
    /// Removes the submission's content.
    fn remove<'a>(
        &'a self,
        grant: &'a WriteGrant,
        permit: &'a WritePermit,
        assign_id: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum SubmissionAction {
    /// Upload the snapshot and save it.
    Save,
    /// Remove the submission's content.
    Remove,
}

impl SubmissionAction {
    fn name(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Remove => "remove",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// What a write changes on the LMS, by file name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Effect {
    /// Not on the LMS yet.
    pub added: Vec<String>,
    /// On the LMS, written again.
    pub replaced: Vec<String>,
    /// On the LMS, deleted.
    pub removed: Vec<String>,
    /// The LMS already holds exactly this, as Core confirmed it; nothing is
    /// sent.
    pub unchanged: bool,
}

/// Whether the person can still change their mind after the write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Reversibility {
    /// Can be changed or removed again until the cut-off, in Unix seconds.
    UntilCutoff { at: i64 },
    /// No cut-off: can be changed or removed again while the assignment
    /// stays open.
    WhileOpen,
    /// Deletes the submitted files, which the LMS cannot restore.
    Irreversible,
}

/// One write, fixed when it was prepared: the snapshot, whose LMS it goes
/// to and what that LMS held.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preparation {
    pub id: String,
    pub assignment_id: String,
    pub assignment_title: String,
    pub identity: LmsIdentity,
    /// The LMS account's generation when it was read; a sign-out or a new
    /// account since refuses the write.
    pub account_generation: u64,
    pub module: LmsModule,
    pub assign_id: u64,
    pub action: SubmissionAction,
    /// By name; empty for a removal.
    pub files: Vec<SnapshotFile>,
    /// What the LMS held, attempt included; `execute` refuses once it moved.
    pub baseline: RemoteState,
    pub effect: Effect,
    pub reversibility: Reversibility,
    /// Unix seconds.
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct SummaryFile {
    pub name: String,
    pub bytes: u64,
    pub sha256_prefix: String,
}

/// What the client's confirmation shows, and what `execute` must receive
/// back unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct SubmissionSummary {
    pub assignment_title: String,
    pub action: SubmissionAction,
    pub files: Vec<SummaryFile>,
    pub effect: Effect,
    pub reversibility: Reversibility,
}

impl Preparation {
    pub fn summary(&self) -> SubmissionSummary {
        SubmissionSummary {
            assignment_title: self.assignment_title.clone(),
            action: self.action,
            files: self
                .files
                .iter()
                .map(|file| SummaryFile {
                    name: file.name.clone(),
                    bytes: file.bytes,
                    sha256_prefix: file.sha256.chars().take(DIGEST_PREFIX).collect(),
                })
                .collect(),
            effect: self.effect.clone(),
            reversibility: self.reversibility,
        }
    }

    /// The write's identity, from what Core recorded and nothing a caller
    /// passed: site, user, assignment, attempt, action and snapshot.
    fn key(&self) -> String {
        let mut material = format!(
            "submission-v2\n{:?}\n{}\n{}\n{}\n{}\n",
            self.identity.site,
            self.identity.user_id,
            self.assign_id,
            self.baseline.attempt,
            self.action.name(),
        );
        for file in &self.files {
            material.push_str(&format!(
                "{:?}\t{}\t{}\n",
                file.name, file.bytes, file.sha256
            ));
        }
        hex_digest(material.as_bytes())
    }
}

/// `execute`'s input: a handle and the summary the client confirmed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRequest {
    pub preparation_id: String,
    pub summary: SubmissionSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// Recorded; nothing sent.
    Prepared,
    /// Uploading the snapshot into a new draft area; the submission is
    /// untouched.
    Uploading,
    /// The save or removal is being sent.
    Saving,
    /// The LMS answered; reading it back.
    Verifying,
    Succeeded,
    Failed,
    /// Sent, and nothing says yet whether the LMS applied it.
    OutcomeUnknown,
}

impl OperationState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Uploading => "uploading",
            Self::Saving => "saving",
            Self::Verifying => "verifying",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }

    pub fn settled(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }
}

/// One write as callers see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "agent", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct SubmissionOperation {
    pub id: String,
    pub preparation_id: String,
    pub assignment_id: String,
    pub action: SubmissionAction,
    pub state: OperationState,
    /// Whether the save or removal may have reached the LMS. Once it may
    /// have, nothing local undoes it.
    pub sent: bool,
    pub failure: Option<ErrorCode>,
    /// Unix seconds.
    pub created_at: i64,
    pub updated_at: i64,
}

/// What `cancel` did. A write already sent is never reported undone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelOutcome {
    pub cancelled: bool,
    pub operation: SubmissionOperation,
}

/// One write as the ledger records it, before anything is sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Operation {
    id: String,
    preparation_id: String,
    assignment_id: String,
    key: String,
    identity: LmsIdentity,
    account_generation: u64,
    module: LmsModule,
    assign_id: u64,
    action: SubmissionAction,
    files: Vec<SnapshotFile>,
    baseline: RemoteState,
    state: OperationState,
    sent: bool,
    /// When the save or removal was sent, in Unix seconds.
    sent_at: Option<i64>,
    /// Whether the LMS answered the save or removal without an error.
    answered: bool,
    failure: Option<ErrorCode>,
    /// Diagnostic text for logs.
    detail: Option<String>,
    /// What the LMS held once the write was confirmed.
    verified: Option<RemoteState>,
    /// The assignment's resource revision the write raised when it sent
    /// (`PlanEngine::bump_resource`).
    revision: u64,
    created_at: i64,
    updated_at: i64,
}

impl Operation {
    fn new(preparation: &Preparation, now: i64) -> Self {
        Self {
            id: uuid::Uuid::now_v7().to_string(),
            preparation_id: preparation.id.clone(),
            assignment_id: preparation.assignment_id.clone(),
            key: preparation.key(),
            identity: preparation.identity.clone(),
            account_generation: preparation.account_generation,
            module: preparation.module,
            assign_id: preparation.assign_id,
            action: preparation.action,
            files: preparation.files.clone(),
            baseline: preparation.baseline.clone(),
            state: OperationState::Prepared,
            sent: false,
            sent_at: None,
            answered: false,
            failure: None,
            detail: None,
            verified: None,
            revision: 0,
            created_at: now,
            updated_at: now,
        }
    }

    fn public(&self) -> SubmissionOperation {
        SubmissionOperation {
            id: self.id.clone(),
            preparation_id: self.preparation_id.clone(),
            assignment_id: self.assignment_id.clone(),
            action: self.action,
            state: self.state,
            sent: self.sent,
            failure: self.failure,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// What one read of the LMS proves about this write.
    fn verdict(&self, remote: &RemoteSubmission) -> Verdict {
        // Another account or assignment says nothing about this write.
        if remote.identity != self.identity || remote.assign_id != self.assign_id {
            return Verdict::Inconclusive;
        }
        let state = &remote.state;
        if state == &self.baseline {
            // Nothing moved: only a refusal the LMS answered proves it did
            // not apply; a lost or late answer may still land.
            return match (self.answered, self.failure) {
                (true, Some(failure)) => Verdict::NotApplied(failure),
                _ => Verdict::Inconclusive,
            };
        }
        let shown = state.attempt == self.baseline.attempt
            && match self.action {
                SubmissionAction::Save => {
                    state.status == RemoteStatus::Submitted
                        && state.time_modified > self.baseline.time_modified
                        && state.files.len() == self.files.len()
                        && state.files.iter().zip(&self.files).all(|(remote, local)| {
                            remote.name == local.name && remote.bytes == local.bytes
                        })
                }
                SubmissionAction::Remove => {
                    matches!(state.status, RemoteStatus::New | RemoteStatus::Reopened)
                        && state.files.is_empty()
                }
            };
        if !shown {
            // The LMS holds something this write does not leave.
            return Verdict::NotApplied(ErrorCode::SubmissionMismatch);
        }
        if self.action == SubmissionAction::Remove {
            return Verdict::Applied;
        }
        match &remote.content {
            Some(content) => {
                let same = content.len() == self.files.len()
                    && content
                        .iter()
                        .zip(&self.files)
                        .all(|(remote, local)| remote.name == local.name && remote.sha256 == local.sha256);
                if same {
                    Verdict::Applied
                } else {
                    Verdict::NotApplied(ErrorCode::SubmissionMismatch)
                }
            }
            // Same names and sizes, content unseen.
            None => Verdict::Inconclusive,
        }
    }

    /// The assignment's key the write changes, by the ledger's own record.
    fn resource(&self) -> Option<(u64, Key)> {
        self.module
            .course
            .map(|course| (course, Key::lms_assignments(course)))
    }
}

/// What one read proves about a write.
enum Verdict {
    Applied,
    NotApplied(ErrorCode),
    /// Until a read can tell, the write stays unknown.
    Inconclusive,
}

/// What an LMS read is for; reads join only within one purpose and fence.
#[derive(Clone, Copy)]
enum Purpose<'a> {
    Prepare,
    Preflight { operation: &'a str },
    Verify { operation: &'a str, check: u32 },
}

/// An assignment kind or state this version does not write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unsupported {
    /// The assignment takes no files.
    NoFiles,
    /// Online text would have to be sent back with every save.
    OnlineText,
    /// A save would stay a draft until submitted for grading.
    Drafts,
    Statement,
    Team,
    TimeLimit,
    /// The LMS takes no save or removal now: closed, locked or past the
    /// cut-off.
    Closed,
    /// The workspace hands in a ZIP, whose bytes are not the same each time
    /// it is packed.
    Zip,
    /// Accepted types are named by group, which Core cannot check yet.
    FileTypeGroups,
}

impl Unsupported {
    pub fn name(self) -> &'static str {
        match self {
            Self::NoFiles => "no-files",
            Self::OnlineText => "online-text",
            Self::Drafts => "drafts",
            Self::Statement => "statement",
            Self::Team => "team",
            Self::TimeLimit => "time-limit",
            Self::Closed => "closed",
            Self::Zip => "zip",
            Self::FileTypeGroups => "file-type-groups",
        }
    }
}

/// A limit of the assignment the files exceed. The LMS skips such files
/// silently or does not check at all, so Core refuses before uploading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Limit {
    Files,
    FileSize,
    FileType,
    /// A name the LMS would store under another name, so the write could
    /// never be read back as applied.
    FileName,
}

impl Limit {
    pub fn name(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::FileSize => "file-size",
            Self::FileType => "file-type",
            Self::FileName => "file-name",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SubmissionError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    HandIn(#[from] HandInError),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Input(String),
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("This assignment cannot be written from here ({})", .0.name())]
    Unsupported(Unsupported),
    #[error("The files exceed the assignment's limit ({})", .0.name())]
    Limit(Limit),
    #[error("The LMS already holds a submission: {replaced:?} would be replaced, {removed:?} removed, {added:?} added")]
    RemoteNotEmpty {
        added: Vec<String>,
        replaced: Vec<String>,
        removed: Vec<String>,
    },
    #[error("The LMS holds no submission to remove")]
    NothingToRemove,
    #[error("The LMS submission changed after it was prepared")]
    RemoteConflict,
    #[error("The files to hand in changed after they were prepared")]
    FilesChanged,
    #[error("The confirmation does not match the prepared submission ({0})")]
    ConfirmationMismatch(&'static str),
    #[error("The preparation expired or is unknown")]
    Expired,
    #[error("Another write to this assignment is in progress")]
    InProgress,
    #[error("Whether an earlier write reached the LMS is not known yet")]
    OutcomeUnknown,
    /// The person's switch, "Allow AI agents to write to LMS", is off.
    #[error("Writing to the LMS is turned off in Settings")]
    WritesDisabled,
    /// The LMS account or its generation changed since the preparation.
    #[error("The school account changed after the submission was prepared")]
    IdentityChanged,
    /// A ledger this version does not read; it is kept as it is and LMS
    /// writes stay off.
    #[error("The submission ledger has format {found}, which this version cannot read (it reads {expected}); LMS writes are off")]
    UnsupportedLedger { found: i64, expected: i64 },
}

impl From<rusqlite::Error> for SubmissionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<std::io::Error> for SubmissionError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(crate::storage::failure_text(error))
    }
}

impl From<serde_json::Error> for SubmissionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<crate::data::DataError> for SubmissionError {
    fn from(error: crate::data::DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<Unsupported> for SubmissionError {
    fn from(reason: Unsupported) -> Self {
        Self::Unsupported(reason)
    }
}

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Prepares, executes and settles LMS writes for every shell and the MCP
/// tools.
#[derive(Clone)]
pub struct SubmissionService {
    store: DataStore,
    workspaces: AssignmentWorkspaces,
    transport: Arc<dyn SubmissionTransport>,
    ledger: Arc<Ledger>,
    snapshots: Arc<Snapshots>,
    /// The engine every read goes through. Weak: the engine owns this
    /// service's duty.
    plan: PlanHints,
    /// Core's owned tasks: a claimed write runs to its end in them, past a
    /// caller that stopped waiting, and shutdown drains it.
    tasks: RuntimeTasks,
    /// This run, the owner of every write it starts.
    owner: String,
    now: Clock,
}

impl SubmissionService {
    /// Opens the ledger's active generation, or builds a new, empty one:
    /// for a folder without one, or in place of a ledger of an older known
    /// format (`approach/short-lived-store-rebuild`: hand-ins are confirmed
    /// before upgrading). A newer or unknown format is refused and kept.
    /// Then recovers what earlier runs left unsettled.
    pub fn open(
        store: &DataStore,
        workspaces: AssignmentWorkspaces,
        transport: Arc<dyn SubmissionTransport>,
        plan: PlanHints,
        tasks: RuntimeTasks,
    ) -> Result<Self, SubmissionError> {
        Self::with_clock(
            store,
            workspaces,
            transport,
            plan,
            tasks,
            Arc::new(|| chrono::Utc::now().timestamp()),
        )
    }

    fn with_clock(
        store: &DataStore,
        workspaces: AssignmentWorkspaces,
        transport: Arc<dyn SubmissionTransport>,
        plan: PlanHints,
        tasks: RuntimeTasks,
        now: Clock,
    ) -> Result<Self, SubmissionError> {
        let root = store.root();
        let active = root
            .active(StoreKind::Submissions)
            .map_err(|error| SubmissionError::Storage(error.to_string()))?;
        // Before generations the ledger lay flat in the folder; of the
        // formats it had, only the known older ones are replaced.
        let flat = root.paths().root().join(ledger::FILE);
        let older = match &active {
            Some(active) if active.format == ledger::FORMAT => None,
            Some(active) if ledger::OLDER_FORMATS.contains(&active.format) => Some(active.format),
            Some(active) => {
                return Err(SubmissionError::UnsupportedLedger {
                    found: active.format,
                    expected: ledger::FORMAT,
                })
            }
            None => match crate::data::root::file_version(&flat) {
                Ok(found) if found != 0 && !ledger::OLDER_FORMATS.contains(&found) => {
                    return Err(SubmissionError::UnsupportedLedger {
                        found,
                        expected: ledger::FORMAT,
                    })
                }
                Ok(found) => Some(found),
                Err(_) => Some(0),
            },
        };
        let generation = match (active, older) {
            (Some(active), None) => root.paths().generation(StoreKind::Submissions, &active.id),
            (_, older) => {
                let (id, directory) = root
                    .new_generation(StoreKind::Submissions)
                    .map_err(|error| SubmissionError::Storage(error.to_string()))?;
                let built = Ledger::create(&directory.join(ledger::FILE))
                    .and_then(|()| Snapshots::open(&directory, root.clone()).map(|_| ()))
                    .map_err(|error| crate::data::StoreOpenError::Io(error.to_string()));
                crate::data::root::finish_candidate(&directory, built)
                    .and_then(|()| {
                        root.publish(
                            &[crate::data::root::Candidate {
                                kind: StoreKind::Submissions,
                                generation: crate::data::root::Generation {
                                    id,
                                    format: ledger::FORMAT,
                                },
                            }],
                            false,
                        )
                    })
                    .map_err(|error| SubmissionError::Storage(error.to_string()))?;
                if let Some(older) = older.filter(|found| *found != 0) {
                    log::info!(target: "app_core::submissions", "The ledger of format {older} was replaced by an empty one");
                }
                directory
            }
        };
        let service = Self {
            store: store.clone(),
            workspaces,
            transport,
            ledger: Arc::new(Ledger::open(&generation.join(ledger::FILE))?),
            snapshots: Arc::new(Snapshots::open(&generation, root.clone())?),
            plan,
            tasks,
            owner: root.boot().to_owned(),
            now,
        };
        service.recover()?;
        Ok(service)
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    /// Every LMS write today comes from an agent, so the person's switch
    /// gates the write path itself, not only the tool that asks for it. It
    /// is read again before each request that changes the LMS.
    fn allowed(&self) -> Result<(), SubmissionError> {
        if self.store.lms_writes_enabled()? {
            Ok(())
        } else {
            Err(SubmissionError::WritesDisabled)
        }
    }

    /// Settles what earlier runs left unsettled; their executor is gone
    /// (only one Core holds the data folder). A write that never sent its
    /// save or removal failed; one that may have is read until the LMS
    /// shows what it did.
    fn recover(&self) -> Result<(), SubmissionError> {
        for mut operation in self.ledger.orphaned(&self.owner)? {
            let from = operation.state;
            match from {
                OperationState::Prepared | OperationState::Uploading => {
                    operation.failure = Some(ErrorCode::Stopped);
                    self.advance(&mut operation, OperationState::Failed, from)?;
                }
                OperationState::Saving | OperationState::Verifying => {
                    self.advance(&mut operation, OperationState::OutcomeUnknown, from)?;
                }
                // Read on its schedule, now owned by this run.
                OperationState::OutcomeUnknown => {
                    self.advance(&mut operation, OperationState::OutcomeUnknown, from)?;
                }
                OperationState::Succeeded | OperationState::Failed => {}
            }
        }
        Ok(())
    }

    /// When the `PlanEngine` next reads an unknown write or an assignment
    /// a settled write changed.
    pub(crate) fn next_due(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        let at = self.ledger.next_due().ok().flatten()?;
        chrono::DateTime::from_timestamp(at, 0)
    }

    /// The `PlanEngine`'s pass: reads every unknown write whose check has
    /// come, then the assignments settled writes changed.
    pub(crate) async fn run_due(&self) -> Result<(), SubmissionError> {
        let now = self.now();
        for (operation, checks) in self.ledger.checks_due(now)? {
            let id = operation.id.clone();
            let settled = self.verify(operation, checks).await?;
            if !settled.state.settled() {
                let wait = CHECK_FIRST
                    .saturating_mul(3_i64.saturating_pow(checks.min(8)))
                    .min(CHECK_MAX);
                self.ledger.schedule_check(&id, self.now() + wait, false)?;
            }
        }
        for refresh in self.ledger.refreshes_due(now)? {
            let key = Key::lms_assignments(refresh.lms_course_id);
            let Some(plan) = self.plan.engine() else {
                return Ok(());
            };
            let landed =
                tokio::time::timeout(REFRESH_WAIT, plan.refresh_key(key, refresh.revision)).await;
            let retry = match landed {
                Ok(Ok(())) => None,
                Ok(Err(ReadRefused::Stopped)) => return Ok(()),
                _ => Some(self.now() + REFRESH_RETRY),
            };
            self.ledger.refreshed(&refresh.operation, retry)?;
        }
        Ok(())
    }

    /// Maintenance: expired preparations, receipts past their retention and
    /// the snapshot objects nothing names any more go.
    pub(crate) fn maintain(&self, now: i64) -> Result<(), SubmissionError> {
        let ledger = self.ledger.clone();
        let removed = self.snapshots.collect(move || ledger.expire(now))?;
        if removed > 0 {
            log::info!(target: "app_core::submissions", "{removed} snapshot objects collected");
        }
        Ok(())
    }

    /// Reads the LMS, snapshots `submit/` for a save, and returns the write
    /// as it would happen. Nothing is sent to the LMS. Refuses while an
    /// earlier write to the assignment is unsettled, and refuses to replace
    /// a submission the LMS already holds: it lists what would be replaced.
    pub async fn prepare(
        &self,
        assignment: &str,
        action: SubmissionAction,
    ) -> Result<Preparation, SubmissionError> {
        let (detail, _) = self.workspaces.detail(assignment)?;
        let module = self.workspaces.lms_module(assignment)?;
        let permit = Arc::new(self.transport.permit()?);
        let account_generation = permit.generation;
        let remote = self.read(Purpose::Prepare, module, permit, false).await?;
        // An earlier write still decides what the LMS holds; an unknown one
        // is read on its schedule, and soon.
        if let Some(operation) = self
            .ledger
            .unsettled(&remote.identity, remote.assign_id)?
            .into_iter()
            .next()
        {
            self.ensure_checked(&operation)?;
            return Err(refusal(&operation));
        }
        supported(&remote.rules)?;
        let capture = match action {
            SubmissionAction::Save => Some(self.capture(assignment).await?),
            SubmissionAction::Remove => None,
        };
        let files = capture.as_ref().map(snapshots::Capture::files).unwrap_or_default();
        if action == SubmissionAction::Save {
            within_limits(&files, &remote.rules)?;
        }
        let reversibility = reversibility(action, &remote.rules);
        let mut preparation = Preparation {
            id: String::new(),
            assignment_id: assignment.into(),
            assignment_title: detail.title,
            identity: remote.identity,
            account_generation,
            module,
            assign_id: remote.assign_id,
            action,
            files,
            baseline: remote.state,
            effect: Effect::default(),
            reversibility,
            expires_at: 0,
        };
        let receipt = self
            .ledger
            .receipt(&preparation.key(), &preparation.baseline)?;
        preparation.effect = effect(&preparation, receipt.is_some())?;
        // Content-addressed: the same snapshot, account, LMS state and
        // effect give the same id.
        preparation.id = hex_digest(&serde_json::to_vec(&preparation)?);
        let now = self.now();
        preparation.expires_at = now + PREPARATION_TTL;
        let snapshots = self.snapshots.clone();
        let ledger = self.ledger.clone();
        tokio::task::spawn_blocking(move || {
            snapshots.publish(capture, || ledger.publish_preparation(&preparation, now))
        })
        .await
        .map_err(|_| SubmissionError::Storage("The snapshot was interrupted".into()))?
    }

    /// Executes a preparation whose summary the client confirmed. The same
    /// write again, however it is asked for, answers with the operation it
    /// already is; a write is sent at most once per execution, and only the
    /// snapshot is uploaded.
    pub async fn execute(
        &self,
        request: ExecuteRequest,
    ) -> Result<SubmissionOperation, SubmissionError> {
        self.allowed()?;
        let now = self.now();
        let Some(prepared) = self
            .ledger
            .preparation(&request.preparation_id)?
            .filter(|preparation| preparation.expires_at > now)
        else {
            return self.repeated(&request.preparation_id);
        };
        confirm(&prepared.summary(), &request.summary)?;
        if prepared.action == SubmissionAction::Save {
            self.check_files(&prepared).await?;
        }
        let claimed = self.ledger.claim(
            &request.preparation_id,
            now,
            &self.owner,
            |preparation| confirm(&preparation.summary(), &request.summary),
            |preparation| Operation::new(preparation, now),
        )?;
        let (preparation, operation) = match claimed {
            Claim::Claimed(preparation, operation) => (*preparation, *operation),
            Claim::Busy(existing) if existing.key == Operation::new(&prepared, now).key => {
                return Ok(existing.public());
            }
            Claim::Busy(existing) => return Err(refusal(&existing)),
            Claim::Expired => return self.repeated(&request.preparation_id),
        };
        // Core owns a claimed write to its end: a caller that stops waiting
        // does not stop it, and shutdown waits for it.
        let service = self.clone();
        self.tasks
            .run(async move { service.run(preparation, operation).await })
            .await
            .map_err(|_| SubmissionError::Storage("The submission task stopped".into()))?
    }

    /// A preparation is dropped once its write succeeded; repeating it
    /// answers with that write.
    fn repeated(&self, preparation: &str) -> Result<SubmissionOperation, SubmissionError> {
        match self.ledger.latest_for(preparation)? {
            Some(operation) if operation.state != OperationState::Failed => {
                self.ensure_checked(&operation)?;
                Ok(operation.public())
            }
            _ => Err(SubmissionError::Expired),
        }
    }

    /// One write's state as the ledger has it; an unknown one is read on
    /// its schedule (queued now if it had none). Never reads the LMS itself.
    pub fn status(&self, operation: &str) -> Result<SubmissionOperation, SubmissionError> {
        let operation = self.known(operation)?;
        self.ensure_checked(&operation)?;
        Ok(operation.public())
    }

    /// An unknown write has a check queued.
    fn ensure_checked(&self, operation: &Operation) -> Result<(), SubmissionError> {
        if operation.state == OperationState::OutcomeUnknown {
            self.ledger.schedule_check(&operation.id, self.now(), true)?;
            self.plan.duties_changed();
        }
        Ok(())
    }

    /// Stops a write that has not sent its save or removal. One that has
    /// runs to its end and is settled as usual: a local cancel never claims
    /// that the LMS undid it.
    pub fn cancel(&self, operation: &str) -> Result<CancelOutcome, SubmissionError> {
        let mut operation = self.known(operation)?;
        let from = operation.state;
        if matches!(from, OperationState::Prepared | OperationState::Uploading) {
            operation.failure = Some(ErrorCode::Cancelled);
            if self.advance(&mut operation, OperationState::Failed, from)? {
                return Ok(CancelOutcome {
                    cancelled: true,
                    operation: operation.public(),
                });
            }
        }
        Ok(CancelOutcome {
            cancelled: false,
            operation: self.known(&operation.id)?.public(),
        })
    }

    /// One LMS read through the `PlanEngine`: its holds and lanes, owned by
    /// the engine, and joined only with a read of the same purpose and
    /// fence.
    async fn read(
        &self,
        purpose: Purpose<'_>,
        module: LmsModule,
        permit: Arc<WritePermit>,
        content: bool,
    ) -> Result<RemoteSubmission, TransportError> {
        let fence = match purpose {
            Purpose::Prepare => format!("prepare:{}", permit.generation),
            Purpose::Preflight { operation } => format!("preflight:{operation}"),
            Purpose::Verify { operation, check } => format!("verify:{operation}:{check}"),
        };
        let transport = self.transport.clone();
        let Some(plan) = self.plan.engine() else {
            return Err(TransportError::NotSent("Core has stopped".into()));
        };
        let read = plan
            .read(
                ReadRequest {
                    key: format!("submission:{}:{}:{fence}", module.cmid, module.course.unwrap_or(0)),
                    lane: Lane::Moodle,
                    account: Some((SchoolAuthProfile::Science, TOKEN_PATH)),
                    deadline: READ_DEADLINE,
                },
                move |_cancel| transport.read(permit, module, content),
            )
            .await;
        let mut remote = match read {
            Ok(read) => read?,
            Err(ReadRefused::Held(Held::Account {
                wait: crate::plan::AccountWait::Person,
                ..
            })) => return Err(TransportError::SignInRequired),
            Err(refused) => return Err(TransportError::NotSent(refused.to_string())),
        };
        remote.state.files.sort();
        if let Some(content) = &mut remote.content {
            content.sort();
        }
        Ok(remote)
    }

    async fn capture(&self, assignment: &str) -> Result<snapshots::Capture, SubmissionError> {
        let workspace = self
            .workspaces
            .find(assignment)?
            .ok_or(HandInError::Empty)?;
        if workspace.zip {
            return Err(Unsupported::Zip.into());
        }
        let submit = PathBuf::from(&workspace.path).join(crate::workspaces::SUBMIT_DIR);
        let snapshots = self.snapshots.clone();
        tokio::task::spawn_blocking(move || snapshots.capture(&submit, &workspace.submit_files))
            .await
            .map_err(|_| SubmissionError::Storage("The snapshot was interrupted".into()))?
    }

    /// The snapshot is what was confirmed, so a damaged one is never sent;
    /// and `submit/` changed since means the person now wants to hand in
    /// something they have not confirmed.
    async fn check_files(&self, preparation: &Preparation) -> Result<(), SubmissionError> {
        let snapshots = self.snapshots.clone();
        let workspaces = self.workspaces.clone();
        let assignment = preparation.assignment_id.clone();
        let files = preparation.files.clone();
        let (intact, current) = tokio::task::spawn_blocking(move || {
            Ok::<_, SubmissionError>((snapshots.intact(&files), workspaces.status(&assignment)?))
        })
        .await
        .map_err(|_| SubmissionError::Storage("The file check was interrupted".into()))??;
        match current {
            Some((workspace, current))
                if intact && !workspace.zip && same_files(&current, &preparation.files) =>
            {
                Ok(())
            }
            _ => Err(SubmissionError::FilesChanged),
        }
    }

    /// One claimed write, from the preflight read to its settled or unknown
    /// end, under one account permit.
    async fn run(
        &self,
        preparation: Preparation,
        mut operation: Operation,
    ) -> Result<SubmissionOperation, SubmissionError> {
        let permit = match self.transport.permit() {
            Ok(permit) if permit.generation == preparation.account_generation => Arc::new(permit),
            Ok(_) => {
                self.ledger.withdraw(&operation.id)?;
                return Err(SubmissionError::IdentityChanged);
            }
            Err(error) => {
                self.ledger.withdraw(&operation.id)?;
                return Err(error.into());
            }
        };
        let checked = match self
            .read(
                Purpose::Preflight {
                    operation: &operation.id,
                },
                preparation.module,
                permit.clone(),
                false,
            )
            .await
        {
            Ok(remote) => self
                .recheck(&preparation, &operation, &remote)
                .and_then(|receipt| self.allowed().map(|()| receipt)),
            Err(error) => Err(error.into()),
        };
        match checked {
            Ok(None) => {}
            Ok(Some(receipt)) => {
                self.ledger.withdraw(&operation.id)?;
                return Ok(receipt.public());
            }
            Err(error) => {
                self.ledger.withdraw(&operation.id)?;
                return Err(error);
            }
        }
        let draft = match preparation.action {
            SubmissionAction::Save => {
                if !self.advance(
                    &mut operation,
                    OperationState::Uploading,
                    OperationState::Prepared,
                )? {
                    return Ok(self.known(&operation.id)?.public());
                }
                let files = self.snapshots.uploads(&preparation.files);
                match self.transport.upload(&GRANT, &permit, &files).await {
                    Ok(draft) => Some(draft),
                    // A draft area alone changes nothing on the submission.
                    Err(error) => {
                        return self
                            .fail(operation, &error)
                            .map(|operation| operation.public())
                    }
                }
            }
            SubmissionAction::Remove => None,
        };
        // The switch may have been turned off during a long upload.
        if let Err(error) = self.allowed() {
            let from = operation.state;
            operation.failure = Some(error.code());
            self.advance(&mut operation, OperationState::Failed, from)?;
            return Err(error);
        }
        // A read of the assignment leased before this point never lands
        // over what the write leaves.
        operation.revision = match (operation.resource(), self.plan.engine()) {
            (Some((_, key)), Some(plan)) => plan.bump_resource(&key),
            _ => 0,
        };
        let from = operation.state;
        operation.sent = true;
        operation.sent_at = Some(self.now());
        // Stored before the request leaves: a crash after this point is
        // settled by reading, never by sending again.
        if !self.advance(&mut operation, OperationState::Saving, from)? {
            return Ok(self.known(&operation.id)?.public());
        }
        let sent = match draft {
            Some(draft) => {
                self.transport
                    .save(&GRANT, &permit, preparation.assign_id, draft)
                    .await
            }
            None => {
                self.transport
                    .remove(&GRANT, &permit, preparation.assign_id)
                    .await
            }
        };
        drop(permit);
        match sent {
            Ok(()) => operation.answered = true,
            Err(TransportError::Uncertain(detail)) => operation.detail = Some(detail),
            // A refusal is read back too: what the LMS holds decides, and
            // the refusal names the failure while nothing changed.
            Err(error) => {
                operation.answered = true;
                operation.failure = Some(error.code());
                operation.detail = Some(error.to_string());
            }
        }
        let to = if operation.answered {
            OperationState::Verifying
        } else {
            OperationState::OutcomeUnknown
        };
        if !self.advance(&mut operation, to, OperationState::Saving)? {
            return Ok(self.known(&operation.id)?.public());
        }
        // Read right away; a write is never sent again.
        Ok(self.verify(operation, 0).await?.public())
    }

    /// The last checks before sending, against a fresh read. `Some` is a
    /// receipt: the LMS already holds this write.
    fn recheck(
        &self,
        preparation: &Preparation,
        operation: &Operation,
        remote: &RemoteSubmission,
    ) -> Result<Option<Operation>, SubmissionError> {
        if remote.identity != preparation.identity {
            return Err(SubmissionError::IdentityChanged);
        }
        if remote.assign_id != preparation.assign_id || remote.cmid != preparation.module.cmid {
            return Err(SubmissionError::RemoteConflict);
        }
        if let Some(receipt) = self.ledger.receipt(&operation.key, &remote.state)? {
            return Ok(Some(receipt));
        }
        // A new cut-off changes what the person confirmed they can undo.
        if remote.state != preparation.baseline
            || reversibility(preparation.action, &remote.rules) != preparation.reversibility
        {
            return Err(SubmissionError::RemoteConflict);
        }
        supported(&remote.rules)?;
        if preparation.action == SubmissionAction::Save {
            within_limits(&preparation.files, &remote.rules)?;
        }
        Ok(None)
    }

    /// Reads the LMS for a write that was answered or whose outcome is
    /// unknown, with the content of what it holds, and concludes it if the
    /// read can tell. A read that cannot leaves it unknown.
    async fn verify(&self, mut operation: Operation, check: u32) -> Result<Operation, SubmissionError> {
        let read = match self.transport.permit() {
            Ok(permit) => {
                self.read(
                    Purpose::Verify {
                        operation: &operation.id,
                        check,
                    },
                    operation.module,
                    Arc::new(permit),
                    operation.action == SubmissionAction::Save,
                )
                .await
            }
            Err(error) => Err(error),
        };
        let verdict = match read {
            Ok(remote) => {
                let verdict = operation.verdict(&remote);
                if matches!(verdict, Verdict::Applied) {
                    operation.verified = Some(remote.state.clone());
                }
                verdict
            }
            Err(error) => {
                operation.detail = Some(error.to_string());
                Verdict::Inconclusive
            }
        };
        let from = operation.state;
        let (to, failure) = match verdict {
            Verdict::Applied => (OperationState::Succeeded, None),
            Verdict::NotApplied(failure) => (OperationState::Failed, Some(failure)),
            Verdict::Inconclusive => (OperationState::OutcomeUnknown, operation.failure),
        };
        if to == from {
            return Ok(operation);
        }
        operation.failure = failure;
        if !self.advance(&mut operation, to, from)? {
            return self.known(&operation.id);
        }
        if to == OperationState::Succeeded {
            self.ledger.release(&operation.preparation_id)?;
        }
        Ok(operation)
    }

    /// A draft upload that failed: the submission was never touched.
    fn fail(
        &self,
        mut operation: Operation,
        error: &TransportError,
    ) -> Result<Operation, SubmissionError> {
        let from = operation.state;
        operation.failure = Some(error.code());
        operation.detail = Some(error.to_string());
        if !self.advance(&mut operation, OperationState::Failed, from)? {
            return self.known(&operation.id);
        }
        Ok(operation)
    }

    /// Moves `operation` to `to` and stores it, only while the ledger still
    /// has it at `from`. A write that may have reached the LMS commits its
    /// settled state with the assignment to read again.
    fn advance(
        &self,
        operation: &mut Operation,
        to: OperationState,
        from: OperationState,
    ) -> Result<bool, SubmissionError> {
        operation.state = to;
        operation.updated_at = self.now();
        let refresh = (to.settled() && operation.sent)
            .then(|| operation.resource())
            .flatten()
            .map(|(lms_course_id, _)| Refresh {
                operation: operation.id.clone(),
                lms_course_id,
                revision: operation.revision,
            });
        let moved =
            self.ledger
                .transition(operation, from, &self.owner, refresh, operation.updated_at)?;
        // A check (unknown) or a refresh (settled) is now due: the engine
        // looks at the deadlines again.
        if moved && (to == OperationState::OutcomeUnknown || to.settled()) {
            self.plan.duties_changed();
        }
        Ok(moved)
    }

    fn known(&self, operation: &str) -> Result<Operation, SubmissionError> {
        self.ledger
            .operation(operation)?
            .ok_or_else(|| SubmissionError::Input("The submission operation is unknown.".into()))
    }
}

/// Why a write to an assignment with an unsettled one is refused.
fn refusal(operation: &Operation) -> SubmissionError {
    match operation.state {
        OperationState::OutcomeUnknown => SubmissionError::OutcomeUnknown,
        _ => SubmissionError::InProgress,
    }
}

/// The individual file assignments the write survey verified, and nothing
/// else (`approach/submission-capabilities`).
fn supported(rules: &SubmissionRules) -> Result<(), Unsupported> {
    let reason = if !rules.files_enabled {
        Unsupported::NoFiles
    } else if rules.online_text {
        Unsupported::OnlineText
    } else if rules.drafts {
        Unsupported::Drafts
    } else if rules.statement {
        Unsupported::Statement
    } else if rules.team {
        Unsupported::Team
    } else if rules.time_limit {
        Unsupported::TimeLimit
    } else if rules.locked || !rules.can_edit {
        Unsupported::Closed
    } else {
        return Ok(());
    };
    Err(reason)
}

/// With drafts off, a save is final at the cut-off, and a removal deletes
/// what it removes for good.
fn reversibility(action: SubmissionAction, rules: &SubmissionRules) -> Reversibility {
    match action {
        SubmissionAction::Save => {
            rules
                .cutoff
                .map_or(Reversibility::WhileOpen, |at| Reversibility::UntilCutoff {
                    at,
                })
        }
        SubmissionAction::Remove => Reversibility::Irreversible,
    }
}

fn within_limits(files: &[SnapshotFile], rules: &SubmissionRules) -> Result<(), SubmissionError> {
    if rules.max_files > 0 && files.len() > rules.max_files as usize {
        return Err(SubmissionError::Limit(Limit::Files));
    }
    if rules.max_file_bytes > 0 && files.iter().any(|file| file.bytes > rules.max_file_bytes) {
        return Err(SubmissionError::Limit(Limit::FileSize));
    }
    // Moodle's `PARAM_FILE` drops these characters from an uploaded name.
    let kept = |name: &str| {
        name != "."
            && name != ".."
            && !name
                .chars()
                .any(|character| character.is_control() || "&<>\"`|':\\/".contains(character))
    };
    if !files.iter().all(|file| kept(&file.name)) {
        return Err(SubmissionError::Limit(Limit::FileName));
    }
    let types: Vec<String> = rules
        .file_types
        .iter()
        .flat_map(|entry| entry.split([',', ';', ' ']))
        .map(|entry| entry.trim().to_lowercase())
        .filter(|entry| !entry.is_empty())
        .collect();
    if types.is_empty() || types.iter().any(|entry| entry == "*") {
        return Ok(());
    }
    if types.iter().any(|entry| !entry.starts_with('.')) {
        return Err(Unsupported::FileTypeGroups.into());
    }
    let accepted = |name: &str| {
        let name = name.to_lowercase();
        types
            .iter()
            .any(|extension| name.ends_with(extension.as_str()))
    };
    if files.iter().all(|file| accepted(&file.name)) {
        Ok(())
    } else {
        Err(SubmissionError::Limit(Limit::FileType))
    }
}

/// What the write changes. A save never replaces a submission the LMS
/// already holds unless Core's own receipt shows it is this very content.
fn effect(preparation: &Preparation, receipt: bool) -> Result<Effect, SubmissionError> {
    if receipt {
        return Ok(Effect {
            unchanged: true,
            ..Effect::default()
        });
    }
    let baseline = &preparation.baseline;
    let remote: BTreeSet<&str> = baseline
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    let local: BTreeSet<&str> = preparation
        .files
        .iter()
        .map(|file| file.name.as_str())
        .collect();
    let names = |set: BTreeSet<&&str>| {
        set.into_iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    match preparation.action {
        SubmissionAction::Save if baseline.holds_content() => {
            Err(SubmissionError::RemoteNotEmpty {
                added: names(local.difference(&remote).collect()),
                replaced: names(local.intersection(&remote).collect()),
                removed: names(remote.difference(&local).collect()),
            })
        }
        SubmissionAction::Save => Ok(Effect {
            added: names(local.iter().collect()),
            ..Effect::default()
        }),
        SubmissionAction::Remove if baseline.holds_content() => Ok(Effect {
            removed: names(remote.iter().collect()),
            ..Effect::default()
        }),
        SubmissionAction::Remove => Err(SubmissionError::NothingToRemove),
    }
}

/// The echo must be the summary, field by field.
fn confirm(prepared: &SubmissionSummary, echo: &SubmissionSummary) -> Result<(), SubmissionError> {
    let field = if prepared.assignment_title != echo.assignment_title {
        "assignmentTitle"
    } else if prepared.action != echo.action {
        "action"
    } else if prepared.files != echo.files {
        "files"
    } else if prepared.effect != echo.effect {
        "effect"
    } else if prepared.reversibility != echo.reversibility {
        "reversibility"
    } else {
        return Ok(());
    };
    Err(SubmissionError::ConfirmationMismatch(field))
}

/// Whether `submit/` still holds exactly the snapshot; both are by name.
fn same_files(current: &[SubmitFile], snapshot: &[SnapshotFile]) -> bool {
    current.len() == snapshot.len()
        && current.iter().zip(snapshot).all(|(current, kept)| {
            current.name == kept.name
                && current.bytes == Some(kept.bytes)
                && current.sha256.as_deref() == Some(kept.sha256.as_str())
        })
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

#[cfg(test)]
#[path = "tests/submissions.rs"]
mod tests;
