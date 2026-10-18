use crate::{cloud_sync, core_ports, data, require_local_main, run_data_command};
use isct_core::workspace::{WorkspaceError, WorkspaceMutation, WorkspaceSnapshot};
use tauri::Manager;

#[tauri::command]
pub(crate) async fn workspace_snapshot(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<WorkspaceSnapshot, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_data_command(move || core.workspace.snapshot().map_err(|error| error.to_string())).await
}

#[tauri::command]
pub(crate) async fn mutate_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    mutation: WorkspaceMutation,
) -> Result<WorkspaceSnapshot, String> {
    require_local_main(&webview)?;
    let started = std::time::Instant::now();
    let core = core_ports::core(&app);
    let snapshot = run_data_command(move || {
        core.workspace
            .mutate(mutation)
            .map_err(|error| match error {
                WorkspaceError::Conflict(_) => {
                    "Workspace changed. Review the latest data and try again.".to_owned()
                }
                error => error.to_string(),
            })
    })
    .await?;
    let persisted = std::time::Instant::now();
    cloud_sync::CloudSync::changed(&app).await;
    if std::env::var_os("ISCT_TRACE_READS").is_some() {
        log::info!(
            "[native-mutation] persist_ms={} cloud_capture_ms={} total_ms={}",
            persisted.duration_since(started).as_millis(),
            persisted.elapsed().as_millis(),
            started.elapsed().as_millis()
        );
    }
    Ok(snapshot)
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

#[tauri::command]
pub(crate) async fn reset_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .prepare_reset(&app)
        .await?;
    let store = store.inner().clone();
    let result =
        run_data_command(move || store.reset_workspace().map_err(|error| error.to_string()))
            .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
}
