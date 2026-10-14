//! Authenticated MCP Streamable HTTP endpoint owned by the desktop process.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use isct_core::ports::EventSink;
use isct_core::{
    agent::{is_write_tool, AgentPolicy},
    events::{CoreEvent, WorkspaceChangeSource},
    runtime_tasks::RuntimeTasks,
    Core,
};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{
    fs::OpenOptions,
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use subtle::ConstantTimeEq;
use tauri::Manager;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

const PORT: u16 = 47_832;
const MAX_REQUEST: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(65);
const MAX_CONCURRENT_REQUESTS: usize = 8;

#[derive(Clone, Serialize)]
pub(crate) struct ConnectionInfo {
    pub(crate) url: String,
    pub(crate) token: String,
}

pub(crate) struct AgentServer {
    connection: ConnectionInfo,
    stop: CancellationToken,
    calls: RuntimeTasks,
    server: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl AgentServer {
    pub(crate) async fn start(app: tauri::AppHandle) -> Result<Self, String> {
        let token_path = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("agent-token");
        let core = crate::core_ports::core(&app);
        Self::bind(core, token_path, Some(app), PORT).await
    }

    async fn bind(
        core: Arc<Core>,
        token_path: PathBuf,
        app: Option<tauri::AppHandle>,
        port: u16,
    ) -> Result<Self, String> {
        let token = load_or_create_token(&token_path)?;
        let listener =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).map_err(|error| {
                if error.kind() == ErrorKind::AddrInUse {
                    format!("The MCP port {port} is already in use.")
                } else {
                    format!("Could not bind the MCP server: {error}")
                }
            })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("Could not configure the MCP listener: {error}"))?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|error| format!("Could not initialize the MCP listener: {error}"))?;
        let connection = ConnectionInfo {
            url: format!("http://{address}/mcp"),
            token,
        };
        let stop = CancellationToken::new();
        let calls = RuntimeTasks::default();
        let router = router(
            core,
            app,
            &connection,
            address.port(),
            stop.clone(),
            calls.clone(),
        );
        let server_stop = stop.clone();
        let server = tauri::async_runtime::spawn(async move {
            if let Err(error) = axum::serve(listener, router)
                .with_graceful_shutdown(server_stop.cancelled_owned())
                .await
            {
                eprintln!("MCP server stopped unexpectedly: {error}");
            }
        });
        Ok(Self {
            connection,
            stop,
            calls,
            server: Mutex::new(Some(server)),
        })
    }

    pub(crate) fn connection(&self) -> ConnectionInfo {
        self.connection.clone()
    }

    pub(crate) async fn shutdown(&self) {
        self.calls.close();
        self.stop.cancel();
        if let Some(server) = self.server.lock().await.take() {
            let _ = server.await;
        }
        self.calls.wait().await;
    }
}

fn load_or_create_token(path: &Path) -> Result<String, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    match read_token(path) {
        Ok(token) => return Ok(token),
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.to_string()),
        Err(_) => {}
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&temporary) {
        Ok(mut file) => {
            if let Err(error) = file
                .write_all(token.as_bytes())
                .and_then(|_| file.sync_all())
            {
                drop(file);
                let _ = std::fs::remove_file(&temporary);
                return Err(error.to_string());
            }
            drop(file);
            match std::fs::hard_link(&temporary, path) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&temporary);
                    Ok(token)
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_file(&temporary);
                    read_token(path).map_err(|error| error.to_string())
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&temporary);
                    Err(error.to_string())
                }
            }
        }
        Err(error) => Err(error.to_string()),
    }
}

fn read_token(path: &Path) -> std::io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "The MCP authorization token must be a regular file.",
        ));
    }
    let mut token = String::new();
    OpenOptions::new()
        .read(true)
        .open(path)?
        .take(128)
        .read_to_string(&mut token)?;
    let token = token.trim().to_owned();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "The MCP authorization token is invalid.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

#[derive(Clone)]
struct HttpPolicy {
    token: Arc<str>,
    permits: Arc<Semaphore>,
}

fn router(
    core: Arc<Core>,
    app: Option<tauri::AppHandle>,
    connection: &ConnectionInfo,
    port: u16,
    stop: CancellationToken,
    calls: RuntimeTasks,
) -> Router {
    let handler = AgentHandler {
        core,
        app,
        calls,
        work: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
    };
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_cancellation_token(stop)
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
        .with_allowed_origins([
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ])
        .with_max_request_body_bytes(MAX_REQUEST);
    let service: StreamableHttpService<AgentHandler, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(handler.clone()), Default::default(), config);
    let policy = HttpPolicy {
        token: Arc::from(connection.token.as_str()),
        permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
    };
    Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(policy, authorize_and_bound))
}

async fn authorize_and_bound(
    State(policy): State<HttpPolicy>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !authorized(request.headers(), &policy.token) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let Ok(_permit) = policy.permits.try_acquire_owned() else {
        return (StatusCode::TOO_MANY_REQUESTS, "Too many MCP requests").into_response();
    };
    match tokio::time::timeout(REQUEST_TIMEOUT, next.run(request)).await {
        Ok(response) => response,
        Err(_) => (StatusCode::REQUEST_TIMEOUT, "MCP request timed out").into_response(),
    }
}

fn authorized(headers: &HeaderMap, expected: &str) -> bool {
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return false;
    };
    value.len() == expected.len() && value.as_bytes().ct_eq(expected.as_bytes()).into()
}

#[derive(Clone)]
struct AgentHandler {
    core: Arc<Core>,
    app: Option<tauri::AppHandle>,
    calls: RuntimeTasks,
    work: Arc<Semaphore>,
}

impl AgentHandler {
    async fn execute(&self, name: String, arguments: Value) -> Result<Value, String> {
        let permit = self
            .work
            .clone()
            .try_acquire_owned()
            .map_err(|_| "Too many MCP tool calls are still running.".to_string())?;
        let (send, receive) = tokio::sync::oneshot::channel();
        let core = self.core.clone();
        let app = self.app.clone();
        let write = is_write_tool(&name);
        if !self.calls.spawn(async move {
            let _permit = permit;
            let result = tokio::task::spawn_blocking(move || {
                core.agent
                    .call(&name, &arguments, AgentPolicy { allow_writes: true })
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let notify = result.is_ok() && write;
            if let Some(app) = app.as_ref().filter(|_| notify) {
                let _ = crate::core_ports::DesktopEvents(app.clone()).emit(
                    CoreEvent::WorkspaceChanged {
                        source: WorkspaceChangeSource::Mcp,
                    },
                );
            }
            let _ = send.send(result);
            // This work remains owned after the HTTP caller disconnects.
            if let Some(app) = app.filter(|_| notify) {
                crate::cloud_sync::CloudSync::changed(&app).await;
            }
        }) {
            return Err("The MCP server is shutting down.".into());
        }
        receive
            .await
            .map_err(|_| "The MCP request stopped before completion.".to_string())?
    }
}

impl ServerHandler for AgentHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("IsctCompanion", env!("CARGO_PKG_VERSION")),
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let core = self.core.clone();
        let permit = self.work.clone().try_acquire_owned().map_err(|_| {
            ErrorData::internal_error("Too many MCP operations are still running.", None)
        })?;
        let enabled = self
            .calls
            .run(async move {
                let _permit = permit;
                tokio::task::spawn_blocking(move || core.store.agent_access_enabled()).await
            })
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        Ok(ListToolsResult::with_all_items(if enabled {
            tools()
        } else {
            Vec::new()
        }))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params(
                "Unknown IsctCompanion tool.",
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = match self.execute(request.name.into_owned(), arguments).await {
            Ok(value) => tool_result(value),
            Err(error) => CallToolResult::error(vec![rmcp::model::ContentBlock::text(error)]),
        };
        Ok(result.into())
    }
}

fn tool_result(value: Value) -> CallToolResult {
    let mut result =
        CallToolResult::success(vec![rmcp::model::ContentBlock::text(value.to_string())]);
    if value.is_object() {
        result.structured_content = Some(value);
    }
    result
}

fn tool(
    name: &'static str,
    description: &'static str,
    properties: Value,
    required: &[&str],
) -> Tool {
    let properties = properties.as_object().cloned().unwrap_or_default();
    let mut schema = Map::new();
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("properties".into(), Value::Object(properties));
    schema.insert(
        "required".into(),
        Value::Array(
            required
                .iter()
                .map(|item| Value::String((*item).into()))
                .collect(),
        ),
    );
    schema.insert("additionalProperties".into(), Value::Bool(false));
    Tool::new(name, description, schema)
}

fn tools() -> Vec<Tool> {
    vec![
        tool("today", "Today in Japan time: the calendar state (holiday, replacement day), classes actually held, personal plans, open assignments not yet due, open assignments overdue in the last 14 days, and the GPA summary.", json!({}), &[]),
        tool("schedule", "Registered or planned courses of the quarters in an inclusive ISO date range (maximum 366 days), intensive dates and personal plans with their occurrences.", json!({"from":{"type":"string","format":"date"},"to":{"type":"string","format":"date"}}), &["from","to"]),
        tool("courses", "Registered or planned courses for one academic quarter.", json!({"year":{"type":"integer"},"quarter":{"enum":["1Q","2Q","3Q","4Q"]}}), &["year","quarter"]),
        tool("course", "One course with syllabus, LMS items and assignments.", json!({"id":{"type":"string"}}), &["id"]),
        tool("search_catalog", "Search the local bilingual course catalog. Only rows with plannable: true can be passed to plan_add; the others need their syllabus opened once in IsctCompanion first.", json!({"query":{"type":"string"}}), &["query"]),
        tool("registration", "Imported read-only registration records.", json!({"year":{"type":"integer"},"term":{"enum":["1Q","2Q","3Q","4Q"]}}), &[]),
        tool("grades", "Imported official grade records.", json!({}), &[]),
        tool("gpa", "Imported GPA, GPT and credit summaries.", json!({}), &[]),
        tool("requirements", "Imported graduation requirement progress.", json!({}), &[]),
        tool("recommended", "Recommended courses for the person's admission year. departmentId defaults to the department chosen in Settings.", json!({"departmentId":{"type":"string"}}), &[]),
        tool("plan_add", "Add a catalog offering (plannable: true in search_catalog) to the person's local plan. term is YYYY-nQ and must be one of the offering's quarters.", json!({"offeringId":{"type":"string"},"term":{"type":"string"}}), &["offeringId","term"]),
        tool("plan_remove", "Remove an offering from the person's local plan.", json!({"offeringId":{"type":"string"},"term":{"type":"string"}}), &["offeringId","term"]),
        tool("plan_event_create", "Create a personal one-off or repeating plan and return its id. start is RFC 3339; duration is minutes.", json!({"title":{"type":"string"},"start":{"type":"string","format":"date-time"},"duration":{"type":"integer","minimum":1,"maximum":10080,"description":"Duration in minutes."},"rrule":{"type":"string"},"room":{"type":"string"}}), &["title","start","duration"]),
        tool("plan_event_update", "Update a personal plan. start is RFC 3339; duration is minutes.", json!({"id":{"type":"string"},"title":{"type":"string"},"start":{"type":"string","format":"date-time"},"duration":{"type":"integer","minimum":1,"maximum":10080,"description":"Duration in minutes."},"rrule":{"type":"string"},"room":{"type":"string"}}), &["id"]),
        tool("plan_event_delete", "Delete a personal plan.", json!({"id":{"type":"string"}}), &["id"]),
        tool("hide_assignment", "Hide an assignment locally without changing LMS.", json!({"id":{"type":"string"}}), &["id"]),
        tool("restore_assignment", "Restore a locally hidden assignment.", json!({"id":{"type":"string"}}), &["id"]),
    ]
}

#[cfg(test)]
mod tests;
