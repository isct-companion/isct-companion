//! Local course materials and notes. No file bytes or session cookies reach JS;
//! CourseMaterials in Core owns material downloads and cache policy.
use crate::data::DataStore;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager};

const MAX_NOTE_BYTES: u64 = 1_000_000;
const MAX_BACKUP_NOTES: usize = 1_000;
const MAX_BACKUP_NOTE_BYTES: u64 = 32 * 1024 * 1024;

/// Serializes note writes.
pub struct FileStore(pub Mutex<()>);
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseFolder {
    pub term: String,
    pub course: String,
    pub section: String,
}
use isct_core::course_materials::{self, CourseMaterials};
pub use isct_core::course_materials::{CourseKey, LocalFile, Material};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    root: String,
    custom: bool,
    can_choose: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalNoteBackup {
    pub id: String,
    pub term: String,
    pub course: String,
    pub section: String,
    pub text: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub rev: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}
pub(crate) fn component(value: &str) -> Result<&str, String> {
    course_materials::component(value).map_err(|error| error.to_string())
}
fn config(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("course-files-root.json"))
}
pub(crate) fn root(app: &AppHandle) -> Result<(PathBuf, bool), String> {
    if let Ok(value) = fs::read_to_string(config(app)?) {
        let path: PathBuf = serde_json::from_str(&value).map_err(|e| e.to_string())?;
        return Ok((path, true));
    }
    #[cfg(desktop)]
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| e.to_string())?;
    #[cfg(mobile)]
    let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok((base.join("IsctCompanion"), false))
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
fn folder(app: &AppHandle, context: &CourseFolder, kind: &str) -> Result<PathBuf, String> {
    storage_dir(
        app,
        &[&context.term, &context.course, &context.section, kind],
    )
}
#[cfg(debug_assertions)]
pub(crate) fn debug_dir(app: &AppHandle) -> Result<PathBuf, String> {
    storage_dir(app, &["debug"])
}
pub(crate) fn storage_dir(app: &AppHandle, parts: &[&str]) -> Result<PathBuf, String> {
    course_materials::storage_dir(&root(app)?.0, parts).map_err(|error| error.to_string())
}
fn describe(
    path: PathBuf,
    id: String,
    section: String,
    cached: bool,
    read_text: bool,
) -> Result<LocalFile, String> {
    course_materials::describe(path, id, section, cached, read_text)
        .map_err(|error| error.to_string())
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
fn note_path(app: &AppHandle, context: &CourseFolder, id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid note ID")?;
    let path = folder(app, context, "notes")?.join(format!("{id}.md"));
    if path.is_symlink() {
        return Err("Cannot edit symbolic link".into());
    }
    Ok(path)
}

/// Notes are user data, so local backup traverses only `notes` and
/// `deleted-notes`; downloaded `materials` are deliberately excluded.
pub fn export_notes(app: &AppHandle) -> Result<Vec<LocalNoteBackup>, String> {
    let (base, _) = root(app)?;
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let canonical = base.canonicalize().map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    let mut total = 0u64;
    for term in fs::read_dir(&base).map_err(|e| e.to_string())?.take(100) {
        let term = term.map_err(|e| e.to_string())?;
        if term.path().is_symlink() || !term.path().is_dir() {
            continue;
        }
        let term_name = term.file_name().to_string_lossy().into_owned();
        component(&term_name)?;
        for course in fs::read_dir(term.path())
            .map_err(|e| e.to_string())?
            .take(500)
        {
            let course = course.map_err(|e| e.to_string())?;
            if course.path().is_symlink() || !course.path().is_dir() {
                continue;
            }
            let course_name = course.file_name().to_string_lossy().into_owned();
            component(&course_name)?;
            for section in fs::read_dir(course.path())
                .map_err(|e| e.to_string())?
                .take(500)
            {
                let section = section.map_err(|e| e.to_string())?;
                if section.path().is_symlink() || !section.path().is_dir() {
                    continue;
                }
                let section_name = section.file_name().to_string_lossy().into_owned();
                // A course-level materials directory is not a section.
                if section_name == "materials" {
                    continue;
                }
                component(&section_name)?;
                for (kind, deleted) in [("notes", false), ("deleted-notes", true)] {
                    let dir = section.path().join(kind);
                    if !dir.is_dir() || dir.is_symlink() {
                        continue;
                    }
                    if !dir
                        .canonicalize()
                        .map_err(|e| e.to_string())?
                        .starts_with(&canonical)
                    {
                        return Err("Notes folder points outside storage root".into());
                    }
                    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.take(500) {
                        let path = entry.map_err(|e| e.to_string())?.path();
                        if path.is_symlink()
                            || path.extension().and_then(|s| s.to_str()) != Some("md")
                        {
                            continue;
                        }
                        let id = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        uuid::Uuid::parse_str(&id).map_err(|_| "Invalid note ID in storage")?;
                        let bytes = path.metadata().map_err(|e| e.to_string())?.len();
                        if bytes > MAX_NOTE_BYTES {
                            return Err("A note exceeds the 1 MB backup limit".into());
                        }
                        total = total
                            .checked_add(bytes)
                            .ok_or("Note backup size overflow")?;
                        if total > MAX_BACKUP_NOTE_BYTES || result.len() >= MAX_BACKUP_NOTES {
                            return Err("Notes exceed the local backup safety limit".into());
                        }
                        let updated_at = chrono::DateTime::<chrono::Utc>::from(
                            path.metadata()
                                .and_then(|m| m.modified())
                                .map_err(|e| e.to_string())?,
                        )
                        .to_rfc3339();
                        let meta: serde_json::Value = fs::read(path.with_extension("sync.json"))
                            .ok()
                            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                            .unwrap_or_default();
                        let text =
                            fs::read_to_string(&path).map_err(|_| "A note is not UTF-8 text")?;
                        result.push(LocalNoteBackup {
                            id,
                            term: term_name.clone(),
                            course: course_name.clone(),
                            section: section_name.clone(),
                            text,
                            deleted,
                            deleted_at: deleted.then(|| updated_at.clone()),
                            updated_at,
                            updated_by: meta["updatedBy"].as_str().unwrap_or("legacy").to_string(),
                            rev: meta["rev"].as_u64().unwrap_or(1),
                        });
                    }
                }
            }
        }
    }
    Ok(result)
}

pub fn restore_notes(app: &AppHandle, notes: &[LocalNoteBackup]) -> Result<(), String> {
    write_notes(app, notes, false)
}
pub fn merge_notes(app: &AppHandle, notes: &[LocalNoteBackup]) -> Result<(), String> {
    write_notes(app, notes, true)
}
pub(crate) fn validate_notes(notes: &[LocalNoteBackup]) -> Result<(), String> {
    if notes.len() > MAX_BACKUP_NOTES {
        return Err("Too many notes in backup".into());
    }
    let mut total = 0usize;
    for note in notes {
        uuid::Uuid::parse_str(&note.id).map_err(|_| "Invalid note ID in backup")?;
        component(&note.term)?;
        component(&note.course)?;
        component(&note.section)?;
        if note.text.len() as u64 > MAX_NOTE_BYTES {
            return Err("A note exceeds the 1 MB limit".into());
        }
        total = total
            .checked_add(note.text.len())
            .ok_or("Note backup size overflow")?;
        if total as u64 > MAX_BACKUP_NOTE_BYTES {
            return Err("Notes exceed the local backup safety limit".into());
        }
    }
    Ok(())
}
fn write_notes(app: &AppHandle, notes: &[LocalNoteBackup], merge: bool) -> Result<(), String> {
    validate_notes(notes)?;
    let state = app.state::<FileStore>();
    let _guard = state.0.lock().map_err(|_| "Storage lock unavailable")?;
    let current = export_notes(app)?;
    if !merge {
        for old in current
            .iter()
            .filter(|old| !notes.iter().any(|n| n.id == old.id))
        {
            let context = CourseFolder {
                term: old.term.clone(),
                course: old.course.clone(),
                section: old.section.clone(),
            };
            let path = folder(
                app,
                &context,
                if old.deleted {
                    "deleted-notes"
                } else {
                    "notes"
                },
            )?
            .join(format!("{}.md", old.id));
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            let _ = fs::remove_file(path.with_extension("sync.json"));
        }
    }
    for note in notes {
        if let Some(local) = current.iter().find(|local| local.id == note.id) {
            if merge
                && crate::cloud_merge::compare(
                    &serde_json::to_value(note).map_err(|e| e.to_string())?,
                    &serde_json::to_value(local).map_err(|e| e.to_string())?,
                )
                .is_lt()
            {
                continue;
            }
            if local.text == note.text
                && local.deleted == note.deleted
                && local.updated_at == note.updated_at
                && local.updated_by == note.updated_by
            {
                continue;
            }
        }
        let context = CourseFolder {
            term: note.term.clone(),
            course: note.course.clone(),
            section: note.section.clone(),
        };
        let path = if note.deleted {
            folder(app, &context, "deleted-notes")?.join(format!("{}.md", note.id))
        } else {
            note_path(app, &context, &note.id)?
        };
        if path.is_symlink() {
            return Err("Cannot restore a note through a symbolic link".into());
        }
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&temp, note.text.as_bytes()).map_err(|e| e.to_string())?;
        fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&note.updated_at) {
            fs::File::options()
                .write(true)
                .open(&path)
                .and_then(|f| f.set_times(fs::FileTimes::new().set_modified(at.into())))
                .map_err(|e| e.to_string())?;
        }
        fs::write(
            path.with_extension("sync.json"),
            serde_json::to_vec(&serde_json::json!({"updatedBy":note.updated_by,"rev":note.rev}))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let opposite = folder(
            app,
            &context,
            if note.deleted {
                "notes"
            } else {
                "deleted-notes"
            },
        )?
        .join(format!("{}.md", note.id));
        if opposite.exists() {
            fs::remove_file(&opposite).map_err(|e| e.to_string())?;
        }
        let _ = fs::remove_file(opposite.with_extension("sync.json"));
    }
    Ok(())
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
        if reset {
            if config(&handle)?.exists() {
                fs::remove_file(config(&handle)?).map_err(|e| e.to_string())?;
            }
        } else {
            #[cfg(desktop)]
            {
                use tauri_plugin_dialog::DialogExt;
                if let Some(chosen) = handle
                    .dialog()
                    .file()
                    .set_title("Course materials and notes")
                    .blocking_pick_folder()
                {
                    let path = chosen
                        .into_path()
                        .map_err(|e| e.to_string())?
                        .canonicalize()
                        .map_err(|e| e.to_string())?;
                    let config = config(&handle)?;
                    fs::create_dir_all(config.parent().unwrap()).map_err(|e| e.to_string())?;
                    fs::write(
                        config,
                        serde_json::to_vec(&path).map_err(|e| e.to_string())?,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            #[cfg(mobile)]
            {
                return Err(
                    "Folder selection is available on desktop; mobile uses app storage".into(),
                );
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    course_storage_info(webview, app).await
}
fn stamp_note(app: &AppHandle, path: &Path) -> Result<(), String> {
    let old: serde_json::Value = fs::read(path.with_extension("sync.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let device = app
        .state::<crate::data::DataStore>()
        .load_workspace()
        .map_err(|error| error.to_string())?["deviceId"]
        .clone();
    fs::write(
        path.with_extension("sync.json"),
        serde_json::to_vec(
            &serde_json::json!({"updatedBy":device,"rev":old["rev"].as_u64().unwrap_or(0)+1}),
        )
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
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
    if text.len() as u64 > MAX_NOTE_BYTES {
        return Err("Notes are limited to 1 MB".into());
    }
    let id = if id.is_empty() {
        uuid::Uuid::now_v7().to_string()
    } else {
        id
    };
    let handle = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<FileStore>();
        let _guard = state.0.lock().map_err(|_| "Storage lock unavailable")?;
        let path = note_path(&handle, &context, &id)?;
        // Create a complete replacement beside the note before replacing it.
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .map_err(|e| e.to_string())?;
            file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            fs::rename(&temp, &path).map_err(|e| e.to_string())?;
            stamp_note(&handle, &path)?;
            describe(path, id, context.section, false, false)
        })();
        let _ = fs::remove_file(temp);
        result
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::cloud_sync::CloudSync::changed(&app);
    Ok(file)
}
#[tauri::command]
pub async fn read_course_note(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    id: String,
) -> Result<LocalFile, String> {
    crate::require_local_main(&webview)?;
    let handle = app.clone();
    let file = tauri::async_runtime::spawn_blocking(move || {
        describe(
            note_path(&handle, &context, &id)?,
            id,
            context.section,
            false,
            true,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(file)
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
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
        let app = handle;
        let state = app.state::<FileStore>();
        let _guard = state.0.lock().map_err(|_| "Storage lock unavailable")?;
        let note = note_path(&app, &context, &id)?;
        let trash = folder(&app, &context, "deleted-notes")?.join(format!("{id}.md"));
        if trash.is_symlink() {
            return Err("Cannot move symbolic link".into());
        }
        let (source, target) = if restore {
            (trash, note)
        } else {
            (note, trash)
        };
        if target.exists() {
            return Err("A note already exists at the destination".into());
        }
        fs::rename(&source, &target).map_err(|e| e.to_string())?;
        let _ = fs::rename(
            source.with_extension("sync.json"),
            target.with_extension("sync.json"),
        );
        fs::File::options()
            .write(true)
            .open(&target)
            .and_then(|f| {
                f.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
            })
            .map_err(|e| e.to_string())?;
        stamp_note(&app, &target)?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::cloud_sync::CloudSync::changed(&app);
    Ok(result)
}
#[tauri::command]
pub async fn list_course_notes(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    sections: Vec<String>,
) -> Result<Vec<LocalFile>, String> {
    crate::require_local_main(&webview)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut notes = Vec::new();
        for section in sections.into_iter().take(300) {
            let mut context = context.clone();
            context.section = section.clone();
            let (base, _) = root(&app)?;
            let dir = base
                .join(component(&context.term)?)
                .join(component(&context.course)?)
                .join(component(&context.section)?)
                .join("notes");
            if !dir.is_dir() {
                continue;
            }
            if !dir
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(base.canonicalize().map_err(|e| e.to_string())?)
            {
                return Err("Notes folder points outside storage root".into());
            }
            for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.take(500) {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.is_symlink() || path.extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let id = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                if uuid::Uuid::parse_str(&id).is_err() {
                    continue;
                }
                notes.push(describe(path, id, section.clone(), false, false)?);
                if notes.len() >= 1000 {
                    return Ok(notes);
                }
            }
        }
        Ok(notes)
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
    course_materials::stored_file(&root(app)?.0, path).map_err(|error| error.to_string())
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_utf8_notes_and_bounds_text_preview() {
        let dir = std::env::temp_dir().join(format!("course-note-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let path = dir.join(format!("{id}.md"));
        let text = "# 読書ノート\n\n$E=mc^2$\n";
        fs::write(&path, text).unwrap();
        let file = describe(path.clone(), id.clone(), "section-91".into(), false, true).unwrap();
        assert_eq!(file.name, "読書ノート");
        assert_eq!(file.text.as_deref(), Some(text));
        assert_eq!(file.bytes, text.len() as u64);
        fs::write(&path, "x".repeat(1_000_001)).unwrap();
        assert!(describe(path, id, "section-91".into(), true, true)
            .unwrap()
            .text
            .is_none());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn binary_materials_do_not_fail_optional_text_preview() {
        let dir = std::env::temp_dir().join(format!("course-file-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("image.png");
        fs::write(&path, [0x89, b'P', b'N', b'G', 0xff]).unwrap();
        let file = describe(
            path,
            "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/image.png".into(),
            String::new(),
            true,
            true,
        )
        .unwrap();
        assert_eq!(file.name, "image.png");
        assert_eq!(file.text, None);
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn hides_extended_windows_prefix_and_uses_unambiguous_separators() {
        assert_eq!(
            display_root(Path::new(r"\\?\E:\IsctCompanion")),
            "E:/IsctCompanion"
        );
        assert_eq!(
            display_root(Path::new(r"\\?\UNC\server\courses")),
            "//server/courses"
        );
    }
}
