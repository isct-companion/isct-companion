//! LMS writes of assignment submissions, with uniqueness, idempotency and
//! recovery owned by Core, never by the caller
//! (`approach/lms-submission-contract`).
//!
//! A write takes two steps. `prepare` snapshots `submit/` into the app data
//! directory by name, size and sha256, reads what the LMS holds now and
//! returns an immutable [`Preparation`]; the same content against the same
//! LMS state gives the same preparation. `execute` takes the preparation's
//! id with an echo of its summary, the one the MCP client showed the person,
//! and refuses any difference. It re-reads the LMS, refuses when the account
//! or what the LMS holds moved, and uploads only the snapshot. Core derives
//! the idempotency key itself; anything a caller passes is only a handle.
//! One write per account and assignment runs at a time, its record is stored
//! before anything is sent, and a write whose outcome is unknown is settled
//! by reading the LMS, never by sending it again.
//!
//! The LMS reports file names and sizes, not digests. So Core counts "the
//! LMS already holds this" only on a receipt of its own: a write of the same
//! content it confirmed, while the LMS still holds what it found then.
//! Nothing can lock the LMS against a browser or another device; checking
//! the baseline again narrows that window and cannot close it.
mod ledger;

use crate::{
    data::DataStore,
    error_code::{Coded, ErrorCode},
    net::BoxFuture,
    workspaces::{AssignmentWorkspaces, HandInError, SubmitFile, WorkspaceError},
};
use ledger::{Claim, Ledger};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

/// How long a preparation can be executed: time for a client's
/// confirmation, while the LMS state it describes is still recent.
const PREPARATION_TTL: i64 = 60 * 60;
/// A write sent without an answer counts as not applied only once the LMS
/// has had this long to finish it; until then it stays unknown and blocks
/// new writes to the assignment.
const SETTLE_AFTER: i64 = 3 * 60;
/// Settled writes stay as receipts this long, so a repeated execute finds
/// the write it repeats.
const RECEIPT_RETENTION: i64 = 14 * 24 * 60 * 60;
/// Hex characters of each sha256 the confirmation shows.
const DIGEST_PREFIX: usize = 12;

/// The account and site the transport reads and writes as.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsIdentity {
    /// As `sync_state::account` writes it, e.g. `science:<username>`.
    pub account: String,
    /// The Moodle site root.
    pub site: String,
    /// The Moodle user id the site reports for the account.
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
    /// One assignment's submission, read as the account signed in now.
    fn read(&self, module: LmsModule) -> BoxFuture<'_, Result<RemoteSubmission, TransportError>>;
    /// Uploads `files` into one new draft area and returns its item id.
    fn upload<'a>(
        &'a self,
        grant: &'a WriteGrant,
        files: &'a [UploadFile],
    ) -> BoxFuture<'a, Result<u64, TransportError>>;
    /// Saves the submission from draft area `draft`, which replaces the
    /// whole file area. With drafts off, this is the submission.
    fn save<'a>(
        &'a self,
        grant: &'a WriteGrant,
        assign_id: u64,
        draft: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>>;
    /// Removes the submission's content.
    fn remove<'a>(
        &'a self,
        grant: &'a WriteGrant,
        assign_id: u64,
    ) -> BoxFuture<'a, Result<(), TransportError>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[serde(rename_all = "camelCase")]
pub struct SummaryFile {
    pub name: String,
    pub bytes: u64,
    pub sha256_prefix: String,
}

/// What the client's confirmation shows, and what `execute` must receive
/// back unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// passed: account, assignment, attempt, action and snapshot.
    fn key(&self) -> String {
        let mut material = format!(
            "submission-v1\n{:?}\n{:?}\n{}\n{}\n{}\n{}\n",
            self.identity.account,
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
    fn name(self) -> &'static str {
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

    /// Whether `state` is what this write leaves on the LMS.
    fn applied(&self, state: &RemoteState) -> bool {
        if state.attempt != self.baseline.attempt {
            return false;
        }
        match self.action {
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
        }
    }
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
    #[error("The LMS account changed after the submission was prepared")]
    AccountChanged,
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
    workspaces: AssignmentWorkspaces,
    transport: Arc<dyn SubmissionTransport>,
    ledger: Arc<Ledger>,
    now: Clock,
}

impl SubmissionService {
    /// Opens the device-local ledger beside `store`'s files.
    pub fn open(
        store: &DataStore,
        workspaces: AssignmentWorkspaces,
        transport: Arc<dyn SubmissionTransport>,
    ) -> Result<Self, SubmissionError> {
        Self::with_clock(
            store,
            workspaces,
            transport,
            Arc::new(|| chrono::Utc::now().timestamp()),
        )
    }

    fn with_clock(
        store: &DataStore,
        workspaces: AssignmentWorkspaces,
        transport: Arc<dyn SubmissionTransport>,
        now: Clock,
    ) -> Result<Self, SubmissionError> {
        let directory = store
            .user_path()
            .parent()
            .ok_or_else(|| SubmissionError::Storage("The data folder is unknown".into()))?;
        Ok(Self {
            workspaces,
            transport,
            ledger: Arc::new(Ledger::open(directory)?),
            now,
        })
    }

    fn now(&self) -> i64 {
        (self.now)()
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
        let remote = self.read(module).await?;
        // An earlier write still decides what the LMS holds; this read may
        // settle it.
        for operation in self
            .ledger
            .unsettled(Some((&remote.identity, remote.assign_id)))?
        {
            let operation = match operation.state {
                OperationState::OutcomeUnknown => self.conclude(operation, &remote)?,
                _ => operation,
            };
            if !operation.state.settled() {
                return Err(refusal(&operation));
            }
        }
        supported(&remote.rules)?;
        let files = match action {
            SubmissionAction::Save => {
                let files = self.snapshot(assignment).await?;
                within_limits(&files, &remote.rules)?;
                files
            }
            SubmissionAction::Remove => Vec::new(),
        };
        let reversibility = reversibility(action, &remote.rules);
        let mut preparation = Preparation {
            id: String::new(),
            assignment_id: assignment.into(),
            assignment_title: detail.title,
            identity: remote.identity,
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
        // Content-addressed: the same snapshot, LMS state and effect give
        // the same id.
        preparation.id = hex_digest(&serde_json::to_vec(&preparation)?);
        let now = self.now();
        if let Some(existing) = self.ledger.preparation(&preparation.id)? {
            if existing.expires_at > now {
                return Ok(existing);
            }
        }
        preparation.expires_at = now + PREPARATION_TTL;
        self.ledger.put_preparation(&preparation)?;
        self.ledger.clean(now)?;
        Ok(preparation)
    }

    /// Executes a preparation whose summary the client confirmed. The same
    /// write again, however it is asked for, answers with the operation it
    /// already is; a write is sent at most once per execution, and only the
    /// snapshot is uploaded.
    pub async fn execute(
        &self,
        request: ExecuteRequest,
    ) -> Result<SubmissionOperation, SubmissionError> {
        let now = self.now();
        let Some(preparation) = self
            .ledger
            .preparation(&request.preparation_id)?
            .filter(|preparation| preparation.expires_at > now)
        else {
            // A preparation is dropped once its write succeeded; repeating
            // it answers with that write.
            return match self.ledger.latest_for(&request.preparation_id)? {
                Some(operation) if operation.state != OperationState::Failed => {
                    self.answer(operation).await
                }
                _ => Err(SubmissionError::Expired),
            };
        };
        confirm(&preparation.summary(), &request.summary)?;
        if preparation.action == SubmissionAction::Save {
            self.check_files(&preparation).await?;
        }
        let operation = Operation::new(&preparation, now);
        match self.ledger.claim(&operation)? {
            Claim::Claimed => {}
            Claim::Busy(existing) if existing.key == operation.key => {
                return self.answer(*existing).await;
            }
            Claim::Busy(existing) => return Err(refusal(&existing)),
        }
        // Core owns a claimed write to its end: a caller that stops waiting
        // does not stop it.
        let service = self.clone();
        tokio::spawn(async move { service.run(preparation, operation).await })
            .await
            .map_err(|_| SubmissionError::Storage("The submission task stopped".into()))?
    }

    /// One write's state; a write whose outcome is unknown is read for
    /// first.
    pub async fn status(&self, operation: &str) -> Result<SubmissionOperation, SubmissionError> {
        self.answer(self.known(operation)?).await
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

    /// On Core start: settles what an earlier run left unsettled. A write
    /// that never sent its save or removal failed; one that may have is
    /// settled by reading the LMS. Returns how many writes settled.
    pub async fn recover(&self) -> Result<usize, SubmissionError> {
        for mut operation in self.ledger.unsettled(None)? {
            let from = operation.state;
            match from {
                OperationState::Prepared | OperationState::Uploading => {
                    operation.failure = Some(ErrorCode::Stopped);
                    self.advance(&mut operation, OperationState::Failed, from)?;
                }
                OperationState::Saving | OperationState::Verifying => {
                    self.advance(&mut operation, OperationState::OutcomeUnknown, from)?;
                }
                _ => {}
            }
        }
        self.reconcile().await
    }

    /// Settles every write whose outcome is unknown by reading the LMS,
    /// right after a timeout, on start and in the runtime wake. Only reads
    /// are retried; a write is never sent again. Returns how many settled.
    pub async fn reconcile(&self) -> Result<usize, SubmissionError> {
        let mut settled = 0;
        for operation in self.ledger.unsettled(None)? {
            if operation.state == OperationState::OutcomeUnknown
                && self.settle(operation).await?.state.settled()
            {
                settled += 1;
            }
        }
        self.ledger.clean(self.now())?;
        Ok(settled)
    }

    async fn read(&self, module: LmsModule) -> Result<RemoteSubmission, TransportError> {
        let mut remote = self.transport.read(module).await?;
        remote.state.files.sort();
        Ok(remote)
    }

    async fn snapshot(&self, assignment: &str) -> Result<Vec<SnapshotFile>, SubmissionError> {
        let workspace = self
            .workspaces
            .find(assignment)?
            .ok_or(HandInError::Empty)?;
        if workspace.zip {
            return Err(Unsupported::Zip.into());
        }
        let submit = PathBuf::from(&workspace.path).join(crate::workspaces::SUBMIT_DIR);
        let ledger = self.ledger.clone();
        tokio::task::spawn_blocking(move || ledger.snapshot(&submit, &workspace.submit_files))
            .await
            .map_err(|_| SubmissionError::Storage("The snapshot was interrupted".into()))?
    }

    /// The snapshot is what was confirmed, so a damaged one is never sent;
    /// and `submit/` changed since means the person now wants to hand in
    /// something they have not confirmed.
    async fn check_files(&self, preparation: &Preparation) -> Result<(), SubmissionError> {
        let ledger = self.ledger.clone();
        let workspaces = self.workspaces.clone();
        let assignment = preparation.assignment_id.clone();
        let files = preparation.files.clone();
        let (intact, current) = tokio::task::spawn_blocking(move || {
            Ok::<_, SubmissionError>((ledger.intact(&files), workspaces.status(&assignment)?))
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

    /// One claimed write, from the last read to its settled or unknown end.
    async fn run(
        &self,
        preparation: Preparation,
        mut operation: Operation,
    ) -> Result<SubmissionOperation, SubmissionError> {
        let checked = match self.read(preparation.module).await {
            Ok(remote) => self.recheck(&preparation, &operation, &remote),
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
                let files = self.ledger.uploads(&preparation.files);
                match self.transport.upload(&GRANT, &files).await {
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
                    .save(&GRANT, preparation.assign_id, draft)
                    .await
            }
            None => self.transport.remove(&GRANT, preparation.assign_id).await,
        };
        let operation = match sent {
            Ok(()) => {
                operation.answered = true;
                if !self.advance(
                    &mut operation,
                    OperationState::Verifying,
                    OperationState::Saving,
                )? {
                    return Ok(self.known(&operation.id)?.public());
                }
                self.settle(operation).await?
            }
            Err(TransportError::Uncertain(detail)) => {
                operation.detail = Some(detail);
                if !self.advance(
                    &mut operation,
                    OperationState::OutcomeUnknown,
                    OperationState::Saving,
                )? {
                    return Ok(self.known(&operation.id)?.public());
                }
                // Read right away; a write is never sent again.
                self.settle(operation).await?
            }
            Err(error) => self.fail(operation, &error)?,
        };
        Ok(operation.public())
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
            return Err(SubmissionError::AccountChanged);
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
    /// unknown, and concludes it if the read can.
    async fn settle(&self, mut operation: Operation) -> Result<Operation, SubmissionError> {
        match self.read(operation.module).await {
            Ok(remote) => self.conclude(operation, &remote),
            Err(error) => {
                let from = operation.state;
                if from != OperationState::OutcomeUnknown {
                    operation.detail = Some(error.to_string());
                    if !self.advance(&mut operation, OperationState::OutcomeUnknown, from)? {
                        return self.known(&operation.id);
                    }
                }
                Ok(operation)
            }
        }
    }

    /// Settles a write from one read of the LMS. Applied succeeds at once.
    /// Not applied fails only when the LMS answered, or once it has had
    /// `SETTLE_AFTER` to finish a write it did not answer; until then the
    /// write stays unknown.
    fn conclude(
        &self,
        mut operation: Operation,
        remote: &RemoteSubmission,
    ) -> Result<Operation, SubmissionError> {
        let from = operation.state;
        let now = self.now();
        let readable =
            remote.identity == operation.identity && remote.assign_id == operation.assign_id;
        let verdict = if !readable {
            // Another account's LMS says nothing about this write.
            None
        } else if operation.applied(&remote.state) {
            Some((OperationState::Succeeded, None))
        } else if operation.answered
            || operation
                .sent_at
                .is_none_or(|sent| now - sent >= SETTLE_AFTER)
        {
            let failure = if remote.state == operation.baseline && !operation.answered {
                ErrorCode::Timeout
            } else {
                ErrorCode::SubmissionMismatch
            };
            Some((OperationState::Failed, Some(failure)))
        } else {
            None
        };
        let Some((state, failure)) = verdict else {
            if from != OperationState::OutcomeUnknown
                && !self.advance(&mut operation, OperationState::OutcomeUnknown, from)?
            {
                return self.known(&operation.id);
            }
            return Ok(operation);
        };
        operation.failure = failure;
        if state == OperationState::Succeeded {
            operation.verified = Some(remote.state.clone());
        }
        if !self.advance(&mut operation, state, from)? {
            return self.known(&operation.id);
        }
        if state == OperationState::Succeeded {
            self.ledger.release(&operation.preparation_id)?;
        }
        self.ledger.clean(now)?;
        Ok(operation)
    }

    /// A write that did not reach the submission, or that the LMS refused.
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
        self.ledger.clean(self.now())?;
        Ok(operation)
    }

    /// Moves `operation` to `to` and stores it, only while the ledger still
    /// has it at `from`.
    fn advance(
        &self,
        operation: &mut Operation,
        to: OperationState,
        from: OperationState,
    ) -> Result<bool, SubmissionError> {
        operation.state = to;
        operation.updated_at = self.now();
        self.ledger.transition(operation, &[from])
    }

    /// Answers with an existing write, reading the LMS first when its
    /// outcome is unknown.
    async fn answer(&self, operation: Operation) -> Result<SubmissionOperation, SubmissionError> {
        let operation = match operation.state {
            OperationState::OutcomeUnknown => self.settle(operation).await?,
            _ => operation,
        };
        Ok(operation.public())
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
