use crate::{core_ports, data, require_local_main, run_data_command};

/// Every stored offering of one academic year, with schedule, credits, language and
/// format, so the course finder can filter and suggest locally without a round trip.
/// The catalog itself is downloaded by Core's sync planner (`catalog.official`).
#[tauri::command]
pub(crate) async fn catalog_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    year: u16,
) -> Result<Vec<serde_json::Value>, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        isct_core::catalog::finder_offerings(&store, year).map_err(|e| e.to_string())
    })
    .await
}

/// Reads one syllabus page the person pasted into the course editor.
#[tauri::command]
pub(crate) async fn syllabus_course(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
    language: String,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    let runtime = core.runtime.clone();
    runtime
        .run(async move { isct_core::catalog::syllabus_course(&core.store, id, language).await })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
