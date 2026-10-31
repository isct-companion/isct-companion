//! Tools that hand work in to the LMS through Core's submission service: an
//! agent prepares a write, the person confirms its summary in their MCP
//! client, and Core executes exactly that, once. Core keeps each write
//! unique and settles one whose outcome is unknown; what an agent passes is
//! only a handle and the summary to compare.
use super::registry::{Effect, Tool};
use super::{non_empty, AgentError, Context};
use crate::submissions::{
    ExecuteRequest, Preparation, SubmissionAction, SubmissionError, SubmissionOperation,
    SubmissionService, SubmissionSummary,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

impl From<SubmissionError> for AgentError {
    fn from(error: SubmissionError) -> Self {
        Self::Submission(error)
    }
}

/// The service, once the school site is connected on this device.
fn service<'a>(cx: &Context<'a>) -> Result<&'a SubmissionService, AgentError> {
    cx.submissions.ok_or_else(|| {
        AgentError::Unavailable(
            "LMS submissions are available once the school site is connected.".into(),
        )
    })
}

/// LMS requests are async; a tool runs on a blocking thread of the shell's
/// runtime, so it waits for them there.
fn runtime() -> Result<tokio::runtime::Handle, AgentError> {
    tokio::runtime::Handle::try_current()
        .map_err(|_| AgentError::Compatibility("no async runtime for LMS requests".into()))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct PrepareArgs {
    /// An assignment id from `today` or `course`.
    assignment_id: String,
    /// `save` hands in the files now in the workspace's `submit/`; `remove`
    /// deletes the submission on the LMS.
    action: SubmissionAction,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Prepared {
    /// Pass to `submission_save` or `submission_remove`.
    preparation_id: String,
    /// What the write does. Pass it back unchanged: the client shows it to
    /// the person for confirmation, and Core refuses any difference.
    summary: SubmissionSummary,
    /// RFC 3339 with offset; prepare again after it.
    expires_at: String,
}

impl From<Preparation> for Prepared {
    fn from(preparation: Preparation) -> Self {
        let expires_at = chrono::FixedOffset::east_opt(9 * 3600)
            .zip(chrono::DateTime::from_timestamp(preparation.expires_at, 0))
            .map(|(jst, at)| at.with_timezone(&jst).to_rfc3339())
            .unwrap_or_default();
        Self {
            summary: preparation.summary(),
            preparation_id: preparation.id,
            expires_at,
        }
    }
}

pub(super) struct SubmissionPrepare;

impl Tool for SubmissionPrepare {
    const NAME: &'static str = "submission_prepare";
    const DESCRIPTION: &'static str = "Prepare an LMS write for one assignment; nothing is sent. For save, Core snapshots the files now in the workspace's submit/ (name, size, SHA-256) and reads the submission on the LMS; for remove, it reads what would be deleted. Returns a preparation id and a summary: the files, what is added, replaced or removed on the LMS, and whether the person can still change or withdraw it afterwards. A save that would replace a submission already on the LMS is refused with what it would replace. The same content against the same LMS state gives the same preparation. Only individual file assignments are supported.";
    const EFFECT: Effect = Effect::LocalWrite;
    const IDEMPOTENT: bool = true;
    type Args = PrepareArgs;
    type Output = Prepared;

    fn run(cx: &Context<'_>, args: PrepareArgs) -> Result<Prepared, AgentError> {
        let id = non_empty(&args.assignment_id, "assignmentId")?;
        let service = service(cx)?;
        Ok(runtime()?
            .block_on(service.prepare(id, args.action))?
            .into())
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct WriteArgs {
    /// From `submission_prepare`.
    preparation_id: String,
    /// The summary `submission_prepare` returned, unchanged.
    summary: SubmissionSummary,
}

/// Executes a confirmed preparation of `action`, and lists the call among
/// the person's recent agent changes once the write exists.
fn execute(
    cx: &Context<'_>,
    tool: &'static str,
    action: SubmissionAction,
    args: WriteArgs,
) -> Result<SubmissionOperation, AgentError> {
    let id = non_empty(&args.preparation_id, "preparationId")?;
    if args.summary.action != action {
        let other = match args.summary.action {
            SubmissionAction::Save => "submission_save",
            SubmissionAction::Remove => "submission_remove",
        };
        return Err(AgentError::InvalidInput(format!(
            "this preparation is executed with {other}"
        )));
    }
    let service = service(cx)?;
    let operation = runtime()?.block_on(service.execute(ExecuteRequest {
        preparation_id: id.to_owned(),
        summary: args.summary,
    }))?;
    // The write happened whatever this audit entry does; failing the call
    // now would invite a retry.
    if let Err(error) = cx.store.agent_write(tool, |_, _| Ok(())) {
        let error = AgentError::from(error);
        log::warn!(target: "app_core::agent", "{tool} was not added to recent agent changes: {error}");
    }
    Ok(operation)
}

pub(super) struct SubmissionSave;

impl Tool for SubmissionSave {
    const NAME: &'static str = "submission_save";
    const DESCRIPTION: &'static str = "Hand the prepared files in to the LMS. At this school saving is submitting: the files replace the submission's file area and count as handed in. Pass the preparation id and the summary from submission_prepare unchanged; the person confirms it in their client. Core refuses a changed summary, files changed since the preparation, another account, and a submission changed on the LMS meanwhile, and needs \"Allow AI agents to write to LMS\" in Settings. Core sends the write at most once: the same call again answers with the same operation, the result is read back from the LMS, and a write whose answer was lost stays outcome_unknown until Core settles it by reading. Never retry a failed write without asking the person.";
    const EFFECT: Effect = Effect::LmsWrite;
    type Args = WriteArgs;
    type Output = SubmissionOperation;

    fn run(cx: &Context<'_>, args: WriteArgs) -> Result<SubmissionOperation, AgentError> {
        execute(cx, Self::NAME, SubmissionAction::Save, args)
    }
}

pub(super) struct SubmissionRemove;

impl Tool for SubmissionRemove {
    const NAME: &'static str = "submission_remove";
    const DESCRIPTION: &'static str = "Remove the submission from the LMS: its files are deleted and the LMS cannot restore them, and the assignment counts as not handed in. Possible only while the LMS still accepts changes. Pass the preparation id and the summary from a submission_prepare with action remove, unchanged; the person confirms it in their client. Needs \"Allow AI agents to write to LMS\" in Settings. Like submission_save, Core sends it at most once, reads the result back and settles a lost answer by reading.";
    const EFFECT: Effect = Effect::LmsWrite;
    type Args = WriteArgs;
    type Output = SubmissionOperation;

    fn run(cx: &Context<'_>, args: WriteArgs) -> Result<SubmissionOperation, AgentError> {
        execute(cx, Self::NAME, SubmissionAction::Remove, args)
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct StatusArgs {
    /// An operation id from `submission_save` or `submission_remove`.
    operation_id: String,
}

pub(super) struct SubmissionStatus;

impl Tool for SubmissionStatus {
    const NAME: &'static str = "submission_status";
    const DESCRIPTION: &'static str = "One LMS write's state: prepared, uploading, saving, verifying, succeeded, failed or outcome_unknown, whether it was sent, and why it failed. A write whose outcome is unknown is read from the LMS first; nothing is ever sent again.";
    const EFFECT: Effect = Effect::Read;
    type Args = StatusArgs;
    type Output = SubmissionOperation;

    fn run(cx: &Context<'_>, args: StatusArgs) -> Result<SubmissionOperation, AgentError> {
        let id = non_empty(&args.operation_id, "operationId")?;
        let service = service(cx)?;
        Ok(runtime()?.block_on(service.status(id))?)
    }
}
