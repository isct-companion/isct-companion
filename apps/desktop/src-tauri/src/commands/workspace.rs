use crate::{coded, CommandError};
use crate::{core_ports, require_local_main, run_data_command, run_read_command};
use app_core::workspace::{WorkspaceCommit, WorkspaceMutation};

/// The revision the window's next intent is made against.
#[tauri::command]
pub(crate) async fn workspace_revision(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<String, CommandError> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_read_command(move || core.workspace.revision().map_err(coded)).await
}

/// Applies one intent and answers what it changed; the window queries again
/// what reads those changes.
#[tauri::command]
pub(crate) async fn mutate_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    mutation: WorkspaceMutation,
) -> Result<WorkspaceCommit, CommandError> {
    require_local_main(&webview)?;
    let started = std::time::Instant::now();
    let core = core_ports::core(&app);
    let commit =
        run_data_command(move || core.workspace.mutate_commit(mutation).map_err(coded)).await?;
    if std::env::var_os("ISCT_TRACE_READS").is_some() {
        log::info!(
            "[native-mutation] persist_ms={}",
            started.elapsed().as_millis()
        );
    }
    Ok(commit)
}

#[tauri::command]
pub(crate) async fn run_scheduled_task(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<String, CommandError> {
    require_local_main(&webview)?;
    core_ports::runtime(&app)
        .run_scheduled(core_ports::services(&app), id)
        .await
        .map_err(coded)?
        .map_err(coded)
}
