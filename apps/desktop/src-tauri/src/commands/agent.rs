use crate::require_local_main;
use tauri::Manager;

/// Agent access state for Settings. Reconciles the listener with the stored
/// setting first, so enabling or disabling takes effect immediately.
#[tauri::command]
pub(crate) async fn agent_connection(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let access = app
        .try_state::<crate::agent_server::AgentAccess>()
        .ok_or("MCP server is unavailable.")?;
    serde_json::to_value(access.reconcile().await).map_err(|error| error.to_string())
}
