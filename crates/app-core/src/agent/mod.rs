//! The local MCP server's tools. Each tool is one contract in [`registry`]:
//! its arguments, its answer, its side effect and the changes it reads. The
//! desktop's HTTP adapter only exposes that list; it holds no tool names or
//! schemas of its own. Only the desktop serves MCP, so only it compiles
//! this module (the `agent` feature).

mod assignments;
mod dto;
mod reads;
mod registry;
mod submissions;
mod writes;

pub use registry::{tools, Effect, ToolSpec};

use crate::data::{AgentWriteError, DataError, DataStore};
use crate::submissions::{SubmissionError, SubmissionService};
use crate::workspace_changes::WorkspaceChange;
use crate::workspaces::AssignmentWorkspaces;
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("IsctCompanion agent access is disabled in Settings.")]
    Disabled,
    #[error("unknown tool")]
    UnknownTool,
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Compatibility(String),
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Unavailable(String),
    /// An LMS write tool's failure, under its `submission-*` code.
    #[error(transparent)]
    Submission(SubmissionError),
}

impl From<&str> for AgentError {
    fn from(message: &str) -> Self {
        Self::InvalidInput(message.to_string())
    }
}
impl From<String> for AgentError {
    fn from(message: String) -> Self {
        Self::InvalidInput(message)
    }
}
impl From<DataError> for AgentError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::InvalidInput(message) => Self::InvalidInput(message),
            DataError::Conflict(message) => Self::Conflict(message),
            DataError::Compatibility(message) => Self::Compatibility(message),
            DataError::Storage(message) => Self::Storage(message),
        }
    }
}
impl From<AgentWriteError> for AgentError {
    fn from(error: AgentWriteError) -> Self {
        match error {
            AgentWriteError::Disabled => Self::Disabled,
            AgentWriteError::Data(error) => error.into(),
        }
    }
}

#[derive(Clone)]
pub struct AgentService {
    store: DataStore,
    workspaces: AssignmentWorkspaces,
    /// Core's submission service, once the school transport is attached.
    submissions: Arc<OnceLock<SubmissionService>>,
}

impl AgentService {
    pub fn new(
        store: DataStore,
        workspaces: AssignmentWorkspaces,
        submissions: Arc<OnceLock<SubmissionService>>,
    ) -> Self {
        Self {
            store,
            workspaces,
            submissions,
        }
    }

    /// Runs one tool with its JSON arguments and answers with its JSON
    /// output, always an object. Access is checked on every call; a write
    /// checks it again inside its own transaction, and an LMS write also
    /// needs the person's LMS switch, which the submission service reads
    /// again before each request that changes the LMS.
    pub fn call(&self, name: &str, args: Value) -> Result<Value, AgentError> {
        let tool = registry::find(name).ok_or(AgentError::UnknownTool)?;
        if !self.store.agent_access_enabled()? {
            return Err(AgentError::Disabled);
        }
        if tool.effect == Effect::LmsWrite && !self.store.lms_writes_enabled()? {
            return Err(AgentError::Submission(SubmissionError::WritesDisabled));
        }
        let view = || load_view(&self.store, tool.reads);
        tool.run(
            &Context {
                store: &self.store,
                workspaces: &self.workspaces,
                submissions: self.submissions.get(),
                view: &view,
            },
            args,
        )
    }
}

type View = (Arc<Value>, Arc<crate::schedule::SchoolReads>);

/// What a tool runs against: the store, and the view its declared changes
/// load (tests swap in a view over every field).
pub(crate) struct Context<'a> {
    store: &'a DataStore,
    workspaces: &'a AssignmentWorkspaces,
    submissions: Option<&'a SubmissionService>,
    view: &'a dyn Fn() -> Result<View, AgentError>,
}

impl Context<'_> {
    fn view(&self) -> Result<View, AgentError> {
        (self.view)()
    }
}

/// The fields of the shared view cache that `reads` name.
fn load_view(store: &DataStore, reads: &[WorkspaceChange]) -> Result<View, AgentError> {
    let fields = crate::workspace_changes::loaded_fields(reads.iter().copied());
    let (workspace, school, _) = store.view(&fields)?;
    Ok((workspace, school))
}

use crate::schedule::text;

/// `value`, or an error naming the empty argument.
fn non_empty<'a>(value: &'a str, name: &str) -> Result<&'a str, AgentError> {
    if value.trim().is_empty() {
        return Err(AgentError::InvalidInput(format!("{name} is required")));
    }
    Ok(value)
}

fn visible_rows(workspace: &Value, key: &str) -> Vec<Value> {
    workspace
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("deletedAt").is_none() && row.get("missingSince").is_none())
        .cloned()
        .collect()
}

/// `YYYY-nQ` → (year, quarter).
fn parse_term(term: &str) -> Result<(i64, &str), AgentError> {
    let invalid = || format!("term must look like 2026-3Q, got {term}");
    let (year, quarter) = term.split_once('-').ok_or_else(invalid)?;
    let year = year
        .parse::<i64>()
        .ok()
        .filter(|year| (1000..=9999).contains(year))
        .ok_or_else(invalid)?;
    if !matches!(quarter, "1Q" | "2Q" | "3Q" | "4Q") {
        return Err(invalid().into());
    }
    Ok((year, quarter))
}

fn now_jst() -> Result<chrono::DateTime<chrono::FixedOffset>, AgentError> {
    let offset = chrono::FixedOffset::east_opt(9 * 3600)
        .ok_or_else(|| AgentError::Compatibility("Japan time offset is unavailable".into()))?;
    Ok(chrono::Utc::now().with_timezone(&offset))
}

fn fallback_term(date: chrono::NaiveDate) -> String {
    crate::schedule::Term::by_month(date).to_string()
}

#[cfg(test)]
#[path = "../tests/agent.rs"]
mod tests;
