use crate::{core_ports, data, require_local_main, run_data_command, syllabus};

#[tauri::command]
pub(crate) async fn catalog_upsert_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    offerings: Vec<serde_json::Value>,
) -> Result<usize, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        store
            .catalog_upsert_offerings(&offerings)
            .map_err(|error| error.to_string())
    })
    .await
}

/// Every stored offering of one academic year, with schedule, credits, language and
/// format, so the course finder can filter and suggest locally without a round trip.
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

#[tauri::command]
pub(crate) async fn catalog_offering(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    id: String,
) -> Result<Option<serde_json::Value>, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        store
            .catalog_offering(&id)
            .map_err(|error| error.to_string())
    })
    .await
}

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

#[tauri::command]
pub(crate) async fn catalog_sync_department(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    year: u16,
    scope_path: String,
    quarter: Option<String>,
) -> Result<syllabus::DepartmentSyncResult, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    let runtime = core.runtime.clone();
    runtime
        .run(async move {
            isct_core::catalog::catalog_sync_department(&core.store, year, scope_path, quarter)
                .await
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn catalog_query(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    query: String,
    year: u16,
    filters: Option<syllabus::SyllabusFilters>,
) -> Result<syllabus::CatalogSearchResult, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    let runtime = core.runtime.clone();
    runtime.run(async move { isct_core::catalog::catalog_query(&core.store, query, year, filters).await })
        .await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn catalog_sync_offerings(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    ids: Vec<String>,
) -> Result<Vec<String>, String> {
    require_local_main(&webview)?;
    let core = core_ports::core(&app);
    let runtime = core.runtime.clone();
    runtime
        .run(async move { isct_core::catalog::catalog_sync_offerings(&core.store, ids).await })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
