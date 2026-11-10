//! Tools for one assignment and its workspace folder. An agent reads the
//! assignment, has Core create the workspace (attachments downloaded,
//! `ASSIGNMENT.md` written) and then writes its own files into `submit/`.
use super::registry::{Effect, Tool};
use super::{non_empty, AgentError, Context};
use crate::workspaces::{SubmitFile, WorkspaceError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

impl From<WorkspaceError> for AgentError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::Input(message) | WorkspaceError::Archive(message) => {
                Self::InvalidInput(message)
            }
            WorkspaceError::Missing => Self::Conflict(error.to_string()),
            WorkspaceError::Storage(message) => Self::Storage(message),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct AssignmentArgs {
    /// An assignment id from `today` or `course`.
    assignment_id: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Attachment {
    name: String,
    url: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Workspace {
    /// The folder, absolute; put the files to hand in under `submit/`.
    path: String,
    /// Names directly inside `submit/`.
    submit_files: Vec<String>,
}

pub(super) struct Assignment;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssignmentAnswer {
    id: String,
    course_id: String,
    course_title: String,
    title: String,
    /// RFC 3339 with offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_url: Option<String>,
    /// The LMS state: unknown, open, submitted or graded.
    #[serde(skip_serializing_if = "Option::is_none")]
    submission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    submission_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grade: Option<String>,
    /// The LMS description, as plain text.
    description: String,
    /// Notes the assignment carries besides the LMS text, as plain text.
    #[serde(skip_serializing_if = "String::is_empty")]
    notes: String,
    attachments: Vec<Attachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<Workspace>,
}

impl Tool for Assignment {
    const NAME: &'static str = "assignment";
    const DESCRIPTION: &'static str = "One assignment: its LMS description as plain text, due date, LMS state, attachments and, once created, its workspace folder.";
    const EFFECT: Effect = Effect::Read;
    type Args = AssignmentArgs;
    type Output = AssignmentAnswer;

    fn run(cx: &Context<'_>, args: AssignmentArgs) -> Result<AssignmentAnswer, AgentError> {
        let id = non_empty(&args.assignment_id, "assignmentId")?;
        let (detail, course_title) = cx.workspaces.detail(id)?;
        let workspace = match &detail.files {
            Some(course) => cx.workspaces.at()?.find(course, id)?,
            None => None,
        };
        Ok(AssignmentAnswer {
            id: detail.assignment_id,
            course_id: detail.course_id,
            course_title,
            title: detail.title,
            due_at: detail.due_at,
            source_url: detail.source_url,
            submission: detail.submission,
            submission_status: detail.submission_status,
            grade: detail.grade,
            description: crate::html_text::plain_text(&detail.body_html),
            notes: crate::html_text::plain_text(&detail.details),
            attachments: detail
                .attachments
                .into_iter()
                .map(|attachment| Attachment {
                    name: attachment.name,
                    url: attachment.url,
                })
                .collect(),
            workspace: workspace.map(|info| Workspace {
                path: info.path,
                submit_files: info.submit_files,
            }),
        })
    }
}

pub(super) struct WorkspaceCreate;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Missing {
    name: String,
    reason: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkspaceCreated {
    /// The folder, absolute: `ASSIGNMENT.md`, the attachments and `submit/`.
    path: String,
    submit_files: Vec<String>,
    /// Attachments that could not be downloaded; `ASSIGNMENT.md` lists them.
    missing: Vec<Missing>,
}

impl Tool for WorkspaceCreate {
    const NAME: &'static str = "workspace_create";
    const DESCRIPTION: &'static str = "Create the assignment's workspace folder once: Core downloads its attachments, writes ASSIGNMENT.md and an empty submit/ folder, and returns the absolute path. An existing workspace is returned as it is. Write the files to hand in into submit/ yourself.";
    const EFFECT: Effect = Effect::LocalWrite;
    const IDEMPOTENT: bool = true;
    type Args = AssignmentArgs;
    type Output = WorkspaceCreated;

    fn run(cx: &Context<'_>, args: AssignmentArgs) -> Result<WorkspaceCreated, AgentError> {
        let id = non_empty(&args.assignment_id, "assignmentId")?;
        // Downloads are async; a tool runs on a blocking thread of the
        // shell's runtime, so it waits for them there.
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| AgentError::Compatibility("no async runtime for downloads".into()))?;
        let created = runtime.block_on(cx.workspaces.create(id))?;
        Ok(WorkspaceCreated {
            path: created.workspace.path,
            submit_files: created.workspace.submit_files,
            missing: created
                .missing
                .into_iter()
                .map(|missing| Missing {
                    name: missing.name,
                    reason: missing.reason,
                })
                .collect(),
        })
    }
}

pub(super) struct WorkspaceStatus;

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct File {
    name: String,
    /// Absent for a folder, which is handed in only as ZIP.
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusAnswer {
    /// Whether the assignment has a workspace yet.
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    /// What `submit/` holds now, file by file.
    files: Vec<File>,
    /// Whether `submit/` is handed in packed as one ZIP.
    zip: bool,
}

impl Tool for WorkspaceStatus {
    const NAME: &'static str = "workspace_status";
    const DESCRIPTION: &'static str =
        "What the assignment's submit/ folder holds now: each file's name, size and SHA-256.";
    const EFFECT: Effect = Effect::Read;
    type Args = AssignmentArgs;
    type Output = StatusAnswer;

    fn run(cx: &Context<'_>, args: AssignmentArgs) -> Result<StatusAnswer, AgentError> {
        let id = non_empty(&args.assignment_id, "assignmentId")?;
        Ok(match cx.workspaces.status(id)? {
            Some((workspace, files)) => StatusAnswer {
                exists: true,
                path: Some(workspace.path),
                files: files
                    .into_iter()
                    .map(
                        |SubmitFile {
                             name,
                             bytes,
                             sha256,
                         }| File {
                            name,
                            bytes,
                            sha256,
                        },
                    )
                    .collect(),
                zip: workspace.zip,
            },
            None => StatusAnswer {
                exists: false,
                path: None,
                files: Vec::new(),
                zip: false,
            },
        })
    }
}
