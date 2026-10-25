use crate::{core_ports, data, require_local_main, run_data_command};
use isct_core::workspace::{WorkspaceCommit, WorkspaceError, WorkspaceMutation, WorkspaceSnapshot};
use isct_core::workspace_changes::{WorkspaceChange, WorkspacePatch};

#[tauri::command]
pub(crate) async fn workspace_snapshot(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<WorkspaceSnapshot, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_data_command(move || core.workspace.snapshot().map_err(|error| error.to_string())).await
}

/// The records a `workspace-changed` event's `changes` feed; the window
/// patches its copy with them instead of reading the whole snapshot.
#[tauri::command]
pub(crate) async fn workspace_records(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    changes: Vec<WorkspaceChange>,
) -> Result<WorkspacePatch, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_data_command(move || {
        core.workspace_records(&changes)
            .map_err(|error| error.to_string())
    })
    .await
}

/// Applies one intent and answers what it changed; the window merges the
/// patch over the snapshot it holds, as it does for `workspace_records`.
#[tauri::command]
pub(crate) async fn mutate_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    mutation: WorkspaceMutation,
) -> Result<WorkspaceCommit, String> {
    require_local_main(&webview)?;
    let started = std::time::Instant::now();
    let core = core_ports::core(&app);
    let commit = run_data_command(move || {
        core.workspace
            .mutate_patch(mutation)
            .map_err(|error| match error {
                WorkspaceError::Conflict(_) => {
                    "Workspace changed. Review the latest data and try again.".to_owned()
                }
                error => error.to_string(),
            })
    })
    .await?;
    if std::env::var_os("ISCT_TRACE_READS").is_some() {
        log::info!(
            "[native-mutation] persist_ms={}",
            started.elapsed().as_millis()
        );
    }
    Ok(commit)
}

#[tauri::command]
pub(crate) async fn load_workspace(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || store.load_workspace().map_err(|error| error.to_string())).await
}

#[tauri::command]
pub(crate) async fn run_scheduled_task(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<String, String> {
    require_local_main(&webview)?;
    core_ports::runtime(&app)
        .run_scheduled(core_ports::services(&app), id)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}
