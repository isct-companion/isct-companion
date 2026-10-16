//! Local course materials and notes. No file bytes or session cookies reach JS;
//! CourseMaterials in Core owns material downloads and cache policy.
use crate::data::DataStore;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

use isct_core::course_materials::CourseMaterials;
pub use isct_core::course_materials::{CourseKey, Material};
pub use isct_core::notes::CourseFolder;
use isct_core::notes::Notes;
use isct_core::storage;
pub use isct_core::storage::LocalFile;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    root: String,
    custom: bool,
    can_choose: bool,
}
pub(crate) fn component(value: &str) -> Result<&str, String> {
    storage::component(value).map_err(|error| error.to_string())
}
fn config(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("course-files-root.json"))
}
/// The chosen storage root. Only a missing configuration means "use the
/// default"; any other read failure is surfaced, because NoteStore pins the
/// first root it sees for the process lifetime.
pub(crate) fn root(app: &AppHandle) -> Result<(PathBuf, bool), String> {
    match fs::read_to_string(config(app)?) {
        Ok(value) => {
            let path: PathBuf = serde_json::from_str(&value).map_err(|e| e.to_string())?;
            Ok((path, true))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((default_root(app)?, false))
        }
        Err(error) => Err(format!("Course storage settings are unreadable: {error}")),
    }
}
fn default_root(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(desktop)]
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| e.to_string())?;
    #[cfg(mobile)]
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(base.join("IsctCompanion"))
}

/// Keep canonical/extended paths internally, but never expose Windows' `\\?\`
/// prefix or locale-dependent backslashes in user-facing text.
pub(crate) fn display_root(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        let value = value
            .strip_prefix(r"\\?\UNC\")
            .map(|rest| format!(r"\\{rest}"))
            .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
            .unwrap_or_else(|| value.into_owned());
        return value.replace('\u{5c}', "/");
    }
    #[cfg(not(windows))]
    value.into_owned()
}
#[cfg(all(matrix_ocr, debug_assertions))]
pub(crate) fn debug_dir(app: &AppHandle) -> Result<PathBuf, String> {
    storage::storage_dir(&root(app)?.0, &["debug"]).map_err(|error| error.to_string())
}
pub(crate) fn materials(app: &AppHandle) -> Result<CourseMaterials, String> {
    #[cfg(desktop)]
    let session = Some(crate::lms_downloader::session(app));
    #[cfg(mobile)]
    let session = None;
    Ok(CourseMaterials::new(
        app.state::<DataStore>().inner().clone(),
        root(app)?.0,
        crate::core_ports::runtime(app).downloads.clone(),
        session,
    ))
}
pub(crate) fn notes(app: &AppHandle) -> Result<Notes, String> {
    Ok(crate::core_ports::runtime(app).notes.at(root(app)?.0))
}

fn device_id(app: &AppHandle) -> Result<String, String> {
    app.state::<DataStore>()
        .load_workspace()
        .map_err(|error| error.to_string())?["deviceId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Device ID is unavailable".into())
}

#[tauri::command]
pub async fn course_storage_info(
    webview: tauri::Webview,
    app: AppHandle,
) -> Result<StorageInfo, String> {
    crate::require_local_main(&webview)?;
    let (root, custom) = root(&app)?;
    Ok(StorageInfo {
        root: display_root(&root),
        custom,
        can_choose: cfg!(desktop),
    })
}
#[tauri::command]
pub async fn choose_course_storage(
    webview: tauri::Webview,
    app: AppHandle,
    reset: bool,
) -> Result<StorageInfo, String> {
    crate::require_local_main(&webview)?;
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let selection = if reset {
            default_root(&handle)?
        } else {
            #[cfg(desktop)]
            {
                use tauri_plugin_dialog::DialogExt;
                let Some(chosen) = handle
                    .dialog()
                    .file()
                    .set_title("Course materials and notes")
                    .blocking_pick_folder()
                else {
                    return Ok(());
                };
                chosen
                    .into_path()
                    .map_err(|e| e.to_string())?
                    .canonicalize()
                    .map_err(|e| e.to_string())?
            }
            #[cfg(mobile)]
            {
                return Err(
                    "Folder selection is available on desktop; mobile uses app storage".into(),
                );
            }
        };
        let config = config(&handle)?;
        let bytes = serde_json::to_vec(&selection).map_err(|error| error.to_string())?;
        crate::core_ports::runtime(&handle)
            .notes
            .select_root(selection, || {
                if reset {
                    match fs::remove_file(&config) {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(error.to_string()),
                    }
                } else {
                    let parent = config
                        .parent()
                        .ok_or("Storage configuration has no directory")?;
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    storage::atomic_write(&config, &bytes).map_err(|error| error.to_string())
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    course_storage_info(webview, app).await
}
#[tauri::command]
pub async fn save_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
    text: String,
) -> Result<LocalFile, String> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    let device_id = device_id(&app)?;
    let runtime = crate::core_ports::runtime(&app);
    runtime
        .run(async move {
            let file = tauri::async_runtime::spawn_blocking(move || {
                service
                    .save(context, id, text, &device_id)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())??;
            crate::cloud_sync::CloudSync::changed(&app).await;
            Ok::<LocalFile, String>(file)
        })
        .await
        .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn read_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
) -> Result<LocalFile, String> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        service.read(context, id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn move_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
    restore: bool,
) -> Result<bool, String> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    let device_id = device_id(&app)?;
    let runtime = crate::core_ports::runtime(&app);
    runtime
        .run(async move {
            let moved = tauri::async_runtime::spawn_blocking(move || {
                service
                    .move_note(context, id, restore, &device_id)
                    .map_err(|error| error.to_string())
            })
            .await
            .map_err(|error| error.to_string())??;
            crate::cloud_sync::CloudSync::changed(&app).await;
            Ok::<bool, String>(moved)
        })
        .await
        .map_err(|error| error.to_string())?
}
#[tauri::command]
pub async fn list_course_notes(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    sections: Vec<String>,
) -> Result<Vec<LocalFile>, String> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        service
            .list(context, sections)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
/// A course file from disk, downloading it first when asked. Files over
/// `limit` bytes are reported instead of downloaded.
#[tauri::command]
pub async fn open_course_material(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseKey,
    url: String,
    download: bool,
    read_text: bool,
    limit: Option<u64>,
) -> Result<Material, String> {
    crate::require_local_main(&webview)?;
    let materials = materials(&app)?;
    let result = crate::core_ports::runtime(&app)
        .run(async move {
            materials
                .open(context, url, download, read_text, limit)
                .await
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    if let Material::Ready { file } = &result {
        app.asset_protocol_scope()
            .allow_file(&file.path)
            .map_err(|error| error.to_string())?;
    }
    Ok(result)
}

/// A path the app saved under the storage root; anything else is refused.
pub(crate) fn stored_file(app: &AppHandle, path: &str) -> Result<PathBuf, String> {
    storage::stored_file(&root(app)?.0, path).map_err(|error| error.to_string())
}
#[tauri::command]
pub async fn open_course_file(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    reveal: bool,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    crate::require_local_main(&webview)?;
    let path = stored_file(&app, &path)?;
    if reveal {
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| e.to_string())
    } else {
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())
    }
}
/// Remove downloaded course files not opened for `days` days. Notes are kept.
#[tauri::command]
pub async fn clean_course_materials(
    webview: tauri::Webview,
    app: AppHandle,
    days: u64,
) -> Result<usize, String> {
    crate::require_local_main(&webview)?;
    let materials = materials(&app)?;
    crate::core_ports::runtime(&app)
        .run(async move { materials.clean(days).await })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[cfg(all(test, windows))]
#[path = "tests/lms_files.rs"]
mod tests;
