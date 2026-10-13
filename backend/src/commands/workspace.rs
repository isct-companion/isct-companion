use crate::{cloud_sync, core_ports, data, require_local_main, run_data_command};
use tauri::Manager;

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
pub(crate) fn new_user_record_id(webview: tauri::Webview) -> Result<String, String> {
    require_local_main(&webview)?;
    Ok(uuid::Uuid::now_v7().to_string())
}

#[tauri::command]
pub(crate) async fn put_user_record(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    table: String,
    record: serde_json::Value,
    expected_rev: Option<u64>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = run_data_command(move || {
        store
            .put_user_record_checked(&table, record, "desktop", expected_rev)
            .map_err(|error| error.to_string())
    })
    .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
}

#[tauri::command]
pub(crate) async fn delete_user_record(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    table: String,
    id: String,
    expected_rev: Option<u64>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = run_data_command(move || {
        store
            .delete_user_record_checked(&table, &id, "desktop", expected_rev)
            .map_err(|error| error.to_string())
    })
    .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
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
pub(crate) async fn put_workspace_setting(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    key: String,
    value: serde_json::Value,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = run_data_command(move || {
        store
            .put_workspace_setting(&key, value, "desktop")
            .map_err(|error| error.to_string())?;
        store
            .load_workspace()
            .map_err(|error| error.to_string())?
            .get("currentTerm")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "current academic term is unavailable".to_string())
    })
    .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
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
