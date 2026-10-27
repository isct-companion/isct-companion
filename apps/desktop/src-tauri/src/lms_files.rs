//! Local course materials and notes. No file bytes or session cookies reach JS;
//! CourseMaterials in Core owns material downloads and cache policy.
use crate::data::DataStore;
use crate::{coded, shell, CommandError};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

use app_core::course_materials::CourseMaterials;
pub use app_core::course_materials::{CourseKey, Material};
pub use app_core::notes::CourseFolder;
use app_core::notes::Notes;
use app_core::storage;
pub use app_core::storage::LocalFile;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    root: String,
    custom: bool,
    can_choose: bool,
}
pub(crate) fn component(value: &str) -> Result<&str, CommandError> {
    storage::component(value).map_err(coded)
}
fn config(app: &AppHandle) -> Result<PathBuf, CommandError> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(shell)?
        .join("course-files-root.json"))
}
/// The chosen storage root. Only a missing configuration means "use the
/// default"; any other read failure is surfaced, because NoteStore pins the
/// first root it sees for the process lifetime.
pub(crate) fn root(app: &AppHandle) -> Result<(PathBuf, bool), CommandError> {
    match fs::read_to_string(config(app)?) {
        Ok(value) => {
            let path: PathBuf = serde_json::from_str(&value).map_err(shell)?;
            Ok((path, true))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((default_root(app)?, false))
        }
        Err(error) => Err(format!("Course storage settings are unreadable: {error}").into()),
    }
}
fn default_root(app: &AppHandle) -> Result<PathBuf, CommandError> {
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(shell)?;
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
pub(crate) fn materials(app: &AppHandle) -> Result<CourseMaterials, CommandError> {
    let session = Some(crate::lms_downloader::session(app));
    Ok(CourseMaterials::new(
        app.state::<DataStore>().inner().clone(),
        root(app)?.0,
        crate::core_ports::runtime(app).downloads.clone(),
        session,
    ))
}
pub(crate) fn notes(app: &AppHandle) -> Result<Notes, CommandError> {
    Ok(crate::core_ports::runtime(app).notes.at(root(app)?.0))
}

fn device_id(app: &AppHandle) -> Result<String, CommandError> {
    app.state::<DataStore>().load_fields(&[]).map_err(coded)?["deviceId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "Device ID is unavailable".into())
}

#[tauri::command]
pub async fn course_storage_info(
    webview: tauri::Webview,
    app: AppHandle,
) -> Result<StorageInfo, CommandError> {
    crate::require_local_main(&webview)?;
    let (root, custom) = root(&app)?;
    Ok(StorageInfo {
        root: display_root(&root),
        custom,
        can_choose: true,
    })
}
#[tauri::command]
pub async fn choose_course_storage(
    webview: tauri::Webview,
    app: AppHandle,
    reset: bool,
) -> Result<StorageInfo, CommandError> {
    crate::require_local_main(&webview)?;
    let chosen = if reset {
        None
    } else {
        let Some(chosen) =
            crate::native_dialog::pick_folder(&webview.window(), "Course materials and notes")
                .await?
        else {
            return course_storage_info(webview, app).await;
        };
        Some(chosen)
    };
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), CommandError> {
        let selection = match chosen {
            None => default_root(&handle)?,
            Some(chosen) => chosen.canonicalize().map_err(shell)?,
        };
        let config = config(&handle)?;
        let bytes = serde_json::to_vec(&selection).map_err(shell)?;
        crate::core_ports::runtime(&handle)
            .notes
            .select_root(selection, || {
                if reset {
                    match fs::remove_file(&config) {
                        Ok(()) => Ok(()),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(error) => Err(shell(error)),
                    }
                } else {
                    let parent = config
                        .parent()
                        .ok_or("Storage configuration has no directory")?;
                    fs::create_dir_all(parent).map_err(shell)?;
                    storage::atomic_write(&config, &bytes).map_err(coded)
                }
            })
            .map_err(coded)?;
        Ok(())
    })
    .await
    .map_err(shell)??;
    course_storage_info(webview, app).await
}
#[tauri::command]
pub async fn save_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
    text: String,
) -> Result<LocalFile, CommandError> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    let device_id = device_id(&app)?;
    let runtime = crate::core_ports::runtime(&app);
    runtime
        .run(async move {
            let file = tauri::async_runtime::spawn_blocking(move || {
                service.save(context, id, text, &device_id).map_err(coded)
            })
            .await
            .map_err(shell)??;
            Ok::<LocalFile, CommandError>(file)
        })
        .await
        .map_err(coded)?
}
#[tauri::command]
pub async fn read_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
) -> Result<LocalFile, CommandError> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    tauri::async_runtime::spawn_blocking(move || service.read(context, id).map_err(coded))
        .await
        .map_err(shell)?
}
#[tauri::command]
pub async fn move_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
    restore: bool,
) -> Result<bool, CommandError> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    let device_id = device_id(&app)?;
    let runtime = crate::core_ports::runtime(&app);
    runtime
        .run(async move {
            let moved = tauri::async_runtime::spawn_blocking(move || {
                service
                    .move_note(context, id, restore, &device_id)
                    .map_err(coded)
            })
            .await
            .map_err(shell)??;
            Ok::<bool, CommandError>(moved)
        })
        .await
        .map_err(coded)?
}
#[tauri::command]
pub async fn list_course_notes(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    sections: Vec<String>,
) -> Result<Vec<LocalFile>, CommandError> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    tauri::async_runtime::spawn_blocking(move || service.list(context, sections).map_err(coded))
        .await
        .map_err(shell)?
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
) -> Result<Material, CommandError> {
    crate::require_local_main(&webview)?;
    let materials = materials(&app)?;
    let result = crate::core_ports::runtime(&app)
        .run(async move {
            materials
                .open(context, url, download, read_text, limit)
                .await
        })
        .await
        .map_err(coded)?
        .map_err(coded)?;
    if let Material::Ready { file } = &result {
        app.asset_protocol_scope()
            .allow_file(&file.path)
            .map_err(shell)?;
    }
    Ok(result)
}

/// A path the app saved under the storage root; anything else is refused.
pub(crate) fn stored_file(app: &AppHandle, path: &str) -> Result<PathBuf, CommandError> {
    storage::stored_file(&root(app)?.0, path).map_err(coded)
}
#[tauri::command]
pub async fn open_course_file(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    reveal: bool,
) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    crate::require_local_main(&webview)?;
    let path = stored_file(&app, &path)?;
    if reveal {
        app.opener().reveal_item_in_dir(&path).map_err(shell)
    } else {
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(shell)
    }
}
/// Remove downloaded course files not opened for `days` days. Notes are kept.
#[tauri::command]
pub async fn clean_course_materials(
    webview: tauri::Webview,
    app: AppHandle,
    days: u64,
) -> Result<usize, CommandError> {
    crate::require_local_main(&webview)?;
    let materials = materials(&app)?;
    crate::core_ports::runtime(&app)
        .run(async move { materials.clean(days).await })
        .await
        .map_err(coded)?
        .map_err(coded)
}

#[cfg(all(test, windows))]
#[path = "tests/lms_files.rs"]
mod tests;
