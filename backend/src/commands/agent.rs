use crate::require_local_main;
#[cfg(desktop)]
use tauri::Manager;

#[cfg(desktop)]
pub(crate) struct AgentServerError(pub String);

#[tauri::command]
pub(crate) fn agent_connection(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    #[cfg(desktop)]
    {
        if let Some(server) = app.try_state::<crate::agent_server::AgentServer>() {
            return serde_json::to_value(server.connection()).map_err(|error| error.to_string());
        }
        Err(app
            .try_state::<AgentServerError>()
            .map(|error| error.0.clone())
            .unwrap_or_else(|| "MCP server is unavailable.".into()))
    }
    #[cfg(mobile)]
    {
        let _ = app;
        Err("Agent access requires the desktop app.".into())
    }
}
