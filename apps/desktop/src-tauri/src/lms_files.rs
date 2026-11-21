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
pub use app_core::course_materials::Material;
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
    // A development build (`isolated_dev`) keeps its own course folder.
    Ok(base.join(if cfg!(isolated_dev) {
        "IsctCompanion Dev"
    } else {
        "IsctCompanion"
    }))
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
        value.replace('\u{5c}', "/")
    }
    #[cfg(not(windows))]
    value.into_owned()
}
pub(crate) fn materials(app: &AppHandle) -> Result<CourseMaterials, CommandError> {
    Ok(CourseMaterials::new(
        app.state::<DataStore>().inner().clone(),
        root(app)?.0,
        crate::core_ports::runtime(app).downloads.clone(),
    ))
}
pub(crate) fn notes(app: &AppHandle) -> Result<Notes, CommandError> {
    Ok(crate::core_ports::runtime(app).notes.at(root(app)?.0))
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
                    storage::durable_replace(&config, &bytes).map_err(coded)
                }
            })
            .map_err(coded)?;
        Ok(())
    })
    .await
    .map_err(shell)??;
    course_storage_info(webview, app).await
}
async fn run_note_mutation<T: Send + 'static>(
    app: &AppHandle,
    mutate: impl FnOnce(Notes, DataStore) -> Result<T, app_core::notes::NoteError> + Send + 'static,
) -> Result<T, CommandError> {
    let service = notes(app)?;
    let data = crate::core_ports::core(app).store.clone();
    crate::core_ports::runtime(app)
        .run(async move {
            tauri::async_runtime::spawn_blocking(move || mutate(service, data).map_err(coded))
                .await
                .map_err(shell)?
        })
        .await
        .map_err(coded)?
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
    run_note_mutation(&app, move |service, data| {
        service.save(&data, context, id, text)
    })
    .await
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
    let data = crate::core_ports::core(&app).store.clone();
    tauri::async_runtime::spawn_blocking(move || service.read(&data, context, id).map_err(coded))
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
    run_note_mutation(&app, move |service, data| {
        service.move_note(&data, context, id, restore)
    })
    .await
}
#[tauri::command]
pub async fn list_course_notes(
    webview: tauri::Webview,
    app: AppHandle,
    course_id: String,
    sections: Vec<String>,
) -> Result<Vec<LocalFile>, CommandError> {
    crate::require_local_main(&webview)?;
    let service = notes(&app)?;
    let data = crate::core_ports::core(&app).store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.list(&data, &course_id, sections).map_err(coded)
    })
    .await
    .map_err(shell)?
}
/// What `url` opens as for the course `course_id` (Core's `Material`): a
/// course file from disk, downloaded first when asked; files over `limit`
/// bytes are reported instead of downloaded.
#[tauri::command]
pub async fn open_course_material(
    webview: tauri::Webview,
    app: AppHandle,
    course_id: String,
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
                .open(course_id, url, download, read_text, limit)
                .await
        })
        .await
        .map_err(coded)?
        .map_err(coded)?;
    if let Material::Ready { file, .. } = &result {
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
    crate::require_local_main(&webview)?;
    let path = stored_file(&app, &path)?;
    crate::downloads::show_file(&app, &path, reveal)
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
