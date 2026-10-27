use crate::{core_ports, require_local_main, run_read_command};

#[tauri::command]
pub(crate) async fn schedule_query(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    queries: Vec<app_core::schedule::WorkspaceQuery>,
) -> Result<app_core::facade::ScheduleQuerySnapshot, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_read_command(move || {
        core.schedule_query(queries)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub(crate) async fn finder_query(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    request: app_core::course_query::FinderRequest,
) -> Result<app_core::course_query::FinderPage, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_read_command(move || {
        core.finder_query(request)
            .map_err(|error| error.to_string())
    })
    .await
}
