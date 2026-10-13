//! Backup command adapters and native file dialogs.
use crate::{cloud_sync, core_ports, data, lms_files, require_local_main};
use isct_core::backup::BackupService;
use tauri::Manager;

pub(crate) fn service(app: &tauri::AppHandle) -> Result<BackupService, String> {
    Ok(BackupService::new(
        app.state::<data::DataStore>().inner().clone(),
        lms_files::notes(app)?,
    ))
}

#[tauri::command]
pub(crate) async fn export_data(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let backup = service(&app)?;
    // A slow notes volume must not hold the workspace database command gate.
    tauri::async_runtime::spawn_blocking(move || backup.export().map_err(|error| error.to_string()))
        .await
        .map_err(|error| error.to_string())?
}

/// Saves a file the person exported (calendar, backup) where they choose.
/// The main webview has no `on_download` handler, so WebKit cancels
/// `<a download>` clicks there. Resolves false when the dialog is cancelled.
#[tauri::command]
pub(crate) async fn save_file(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    name: String,
    contents: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    if name.trim().is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("invalid file name".into());
    }
    if contents.len() > 64 * 1024 * 1024 {
        return Err("file exceeds the 64 MiB limit".into());
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
            let mut dialog = app.dialog().file().set_file_name(&name);
            if let Some((_, extension)) = name.rsplit_once('.') {
                dialog = dialog.add_filter(extension.to_uppercase(), &[extension]);
            }
            let Some(chosen) = dialog.blocking_save_file() else {
                return Ok(false);
            };
            let path = chosen.into_path().map_err(|e| e.to_string())?;
            std::fs::write(path, contents).map_err(|e| e.to_string())?;
            Ok(true)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(mobile)]
    {
        let _ = (app, contents);
        Err("Saving files is available on desktop.".into())
    }
}

pub(crate) fn write_scheduled_export_inner(app: &tauri::AppHandle) -> Result<String, String> {
    service(app)?
        .write_scheduled(
            &app.path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("scheduled-exports"),
        )
        .map(|path| path.display().to_string())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn write_scheduled_export(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let runtime = core_ports::runtime(&app);
    runtime
        .run(async move {
            tauri::async_runtime::spawn_blocking(move || write_scheduled_export_inner(&app))
                .await
                .map_err(|error| error.to_string())?
        })
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn import_data(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    document: String,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let parsed = data::parse_import(&document).map_err(|error| error.to_string())?;
    app.state::<cloud_sync::CloudSync>()
        .restore_document(&app, &parsed)
        .await?;
    app.state::<data::DataStore>()
        .load_workspace()
        .map_err(|error| error.to_string())
}
