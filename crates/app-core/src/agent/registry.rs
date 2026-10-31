//! Every tool's contract in one list: name, description, side effect,
//! argument and answer types, and the changes it reads. The HTTP adapter
//! serves this list as `tools/list`; the JSON Schemas come from the Rust
//! types themselves.
use super::{assignments, reads, submissions, writes, AgentError, Context};
use crate::workspace_changes::WorkspaceChange;
use schemars::{generate::SchemaSettings, JsonSchema};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};
use std::sync::LazyLock;

/// What a call does besides answering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
    /// Reads; changes nothing.
    Read,
    /// Changes the person's own local data (plans, weekly events, hidden
    /// assignments), audited in the same transaction.
    LocalWrite,
    /// Changes the school's LMS: hands work in or removes it. Needs the
    /// person's LMS switch, and asks every client to confirm with the person
    /// each time, whatever its approval mode.
    LmsWrite,
}

/// The `_meta` key that has Claude Code ask the person before every call,
/// even in bypass mode (`approach/mcp-submit-confirmation`).
const REQUIRES_INTERACTION: &str = "anthropic/requiresUserInteraction";

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub effect: Effect,
    /// Changes nothing (MCP `readOnlyHint`).
    pub read_only: bool,
    /// May remove or overwrite what is there (MCP `destructiveHint`).
    pub destructive: bool,
    /// Repeating the call with the same arguments changes nothing more
    /// (MCP `idempotentHint`).
    pub idempotent: bool,
    /// Reaches beyond this device, to the school's LMS (MCP
    /// `openWorldHint`).
    pub open_world: bool,
    /// The tool's MCP `_meta`, as the adapter lists it.
    pub meta: Map<String, Value>,
    pub input_schema: Map<String, Value>,
    pub output_schema: Map<String, Value>,
    pub(super) reads: &'static [WorkspaceChange],
    run: fn(&Context<'_>, Value) -> Result<Value, AgentError>,
}

impl ToolSpec {
    pub(super) fn run(&self, cx: &Context<'_>, args: Value) -> Result<Value, AgentError> {
        (self.run)(cx, args)
    }
}

/// One tool. `Args` and `Output` are its whole contract: unknown arguments
/// are rejected, and only the fields `Output` names reach a client.
pub(super) trait Tool {
    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const EFFECT: Effect;
    const DESTRUCTIVE: bool = false;
    /// Reads are idempotent by nature; a write says so itself.
    const IDEMPOTENT: bool = false;
    /// The changes its answer is computed from, as the shared queries
    /// declare theirs; a tool that reads tables directly loads no view.
    const READS: &'static [WorkspaceChange] = &[];
    type Args: DeserializeOwned + JsonSchema;
    type Output: Serialize + JsonSchema;
    fn run(cx: &Context<'_>, args: Self::Args) -> Result<Self::Output, AgentError>;
}

static TOOLS: LazyLock<Vec<ToolSpec>> = LazyLock::new(|| {
    vec![
        entry::<reads::Today>(),
        entry::<reads::Schedule>(),
        entry::<reads::Courses>(),
        entry::<reads::Course>(),
        entry::<reads::SearchCatalog>(),
        entry::<reads::Registration>(),
        entry::<reads::Grades>(),
        entry::<reads::Gpa>(),
        entry::<reads::Requirements>(),
        entry::<reads::Recommended>(),
        entry::<assignments::Assignment>(),
        entry::<assignments::WorkspaceStatus>(),
        entry::<writes::PlanAdd>(),
        entry::<writes::PlanRemove>(),
        entry::<writes::WeeklyEventCreate>(),
        entry::<writes::WeeklyEventUpdate>(),
        entry::<writes::WeeklyEventDelete>(),
        entry::<writes::HideAssignment>(),
        entry::<writes::RestoreAssignment>(),
        entry::<assignments::WorkspaceCreate>(),
        entry::<submissions::SubmissionPrepare>(),
        entry::<submissions::SubmissionSave>(),
        entry::<submissions::SubmissionRemove>(),
        entry::<submissions::SubmissionStatus>(),
    ]
});

/// Every tool, in the order `tools/list` shows them.
pub fn tools() -> &'static [ToolSpec] {
    &TOOLS
}

pub(super) fn find(name: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// The side effect decides the hints and `_meta`; a tool adds only whether
/// it overwrites and whether repeating it is safe.
fn entry<T: Tool>() -> ToolSpec {
    let lms_write = T::EFFECT == Effect::LmsWrite;
    ToolSpec {
        name: T::NAME,
        description: T::DESCRIPTION,
        effect: T::EFFECT,
        read_only: T::EFFECT == Effect::Read,
        destructive: T::DESTRUCTIVE || lms_write,
        idempotent: T::EFFECT == Effect::Read || T::IDEMPOTENT,
        open_world: lms_write,
        meta: if lms_write {
            Map::from_iter([(REQUIRES_INTERACTION.to_string(), Value::Bool(true))])
        } else {
            Map::new()
        },
        input_schema: schema::<T::Args>(SchemaSettings::draft2020_12().for_deserialize()),
        output_schema: schema::<T::Output>(SchemaSettings::draft2020_12().for_serialize()),
        reads: T::READS,
        run: run::<T>,
    }
}

fn run<T: Tool>(cx: &Context<'_>, args: Value) -> Result<Value, AgentError> {
    let args = serde_json::from_value::<T::Args>(args)
        .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
    serde_json::to_value(T::run(cx, args)?)
        .map_err(|error| AgentError::Compatibility(error.to_string()))
}

/// A self-contained schema: subschemas inlined, since not every MCP client
/// resolves `$ref`.
fn schema<T: JsonSchema>(settings: SchemaSettings) -> Map<String, Value> {
    let generator = settings
        .with(|settings| {
            settings.inline_subschemas = true;
            settings.meta_schema = None;
        })
        .into_generator();
    match Value::from(generator.into_root_schema_for::<T>()) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
