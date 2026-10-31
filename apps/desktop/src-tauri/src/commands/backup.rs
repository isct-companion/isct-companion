//! Backup command adapters and saving exported files.
use crate::{cloud_sync, data, require_local_main};
use crate::{coded, shell, CommandError};
use app_core::backup::BackupService;
use tauri::Manager;

pub(crate) fn service(app: &tauri::AppHandle) -> Result<BackupService, CommandError> {
    Ok(BackupService::new(
        app.state::<data::DataStore>().inner().clone(),
    ))
}

#[tauri::command]
pub(crate) async fn export_data(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<String, CommandError> {
    require_local_main(&webview)?;
    let backup = service(&app)?;
    // A slow notes volume must not hold the workspace database command gate.
    tauri::async_runtime::spawn_blocking(move || backup.export().map_err(coded))
        .await
        .map_err(shell)?
}

/// Saves a file the person exported (calendar, backup) where they choose.
/// The main webview has no `on_download` handler, so WebKit cancels
/// `<a download>` clicks there. Resolves false when the dialog is cancelled.
#[tauri::command]
pub(crate) async fn save_file(
    webview: tauri::Webview,
    name: String,
    contents: String,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    if name.trim().is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("invalid file name".into());
    }
    if contents.len() > 64 * 1024 * 1024 {
        return Err("file exceeds the 64 MiB limit".into());
    }
    let Some(path) = crate::native_dialog::save_file(&webview.window(), name).await? else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || std::fs::write(path, contents))
        .await
        .map_err(shell)?
        .map_err(shell)?;
    Ok(true)
}

pub(crate) fn write_scheduled_export_inner(app: &tauri::AppHandle) -> Result<String, CommandError> {
    service(app)?
        .write_scheduled(
            &app.path()
                .app_data_dir()
                .map_err(shell)?
                .join("scheduled-exports"),
        )
        .map(|path| path.display().to_string())
        .map_err(coded)
}

#[tauri::command]
pub(crate) async fn import_data(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    document: String,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    let parsed = data::parse_import(&document).map_err(coded)?;
    // The restore's own commit tells the window what to query again.
    app.state::<cloud_sync::CloudSync>()
        .restore_document(&app, &parsed)
        .await
        .map(|_| ())
}
