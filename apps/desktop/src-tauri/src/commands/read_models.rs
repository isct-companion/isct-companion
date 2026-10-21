use crate::{core_ports, require_local_main, run_data_command};

#[tauri::command]
pub(crate) async fn schedule_query(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    queries: Vec<isct_core::schedule::WorkspaceQuery>,
) -> Result<isct_core::facade::ScheduleQuerySnapshot, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_data_command(move || {
        core.schedule_query(queries)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub(crate) async fn finder_query(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    request: isct_core::course_query::FinderRequest,
) -> Result<isct_core::course_query::FinderPage, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    run_data_command(move || {
        core.finder_query(request)
            .map_err(|error| error.to_string())
    })
    .await
}
