//! Local course materials and notes. No file bytes or session cookies reach JS;
//! materials are downloaded by `lms_downloader` straight into the course folder.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
};
use tauri::{AppHandle, Manager};
use url::Url;

const MAX_AUTO_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_RETENTION_DAYS: u64 = 3650;
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
/// Materials are stored per course; a full `CourseFolder` also deserializes here.
#[derive(Clone, Deserialize)]
pub struct CourseKey {
    pub term: String,
    pub course: String,
}
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Material {
    Ready { file: LocalFile },
    Missing,
    TooLarge { bytes: u64 },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageInfo {
    root: String,
    custom: bool,
    can_choose: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFile {
    pub id: String,
    pub name: String,
    pub section: String,
    pub path: String,
    pub bytes: u64,
    pub cached: bool,
    pub text: Option<String>,
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
}
fn component(value: &str) -> Result<&str, String> {
    if value.is_empty()
        || value.len() > 120
        || value == "."
        || value == ".."
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        return Err("Invalid course folder component".into());
    }
    Ok(value)
}
fn config(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("course-files-root.json"))
}
fn root(app: &AppHandle) -> Result<(PathBuf, bool), String> {
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
    Ok((base.join("Campus Life"), false))
}
fn folder(app: &AppHandle, context: &CourseFolder, kind: &str) -> Result<PathBuf, String> {
    storage_dir(app, &[&context.term, &context.course, &context.section, kind])
}
/// One folder per course for downloaded files, whichever section links them.
fn materials_dir(app: &AppHandle, course: &CourseKey) -> Result<PathBuf, String> {
    storage_dir(app, &[&course.term, &course.course, "materials"])
}
fn storage_dir(app: &AppHandle, parts: &[&str]) -> Result<PathBuf, String> {
    let (root, _) = root(app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut path = root.clone();
    for name in parts {
        path.push(component(name)?);
        if path.exists()
            && !path
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&root)
        {
            return Err("Course folder points outside storage root".into());
        }
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    Ok(path)
}
fn describe(
    path: PathBuf,
    id: String,
    section: String,
    cached: bool,
    read_text: bool,
) -> Result<LocalFile, String> {
    let bytes = path.metadata().map_err(|e| e.to_string())?.len();
    let text = if read_text && bytes <= 1_000_000 {
        Some(fs::read_to_string(&path).map_err(|_| "File is not UTF-8 text")?)
    } else {
        None
    };
    let name = if uuid::Uuid::parse_str(&id).is_ok() {
        let mut prefix = [0u8; 512];
        let size = fs::File::open(&path)
            .and_then(|mut file| file.read(&mut prefix))
            .unwrap_or(0);
        String::from_utf8_lossy(&prefix[..size])
            .lines()
            .next()
            .unwrap_or("Note")
            .trim_start_matches('#')
            .trim()
            .chars()
            .take(100)
            .collect()
    } else {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    Ok(LocalFile {
        id,
        name,
        section,
        path: path.to_string_lossy().into(),
        bytes,
        cached,
        text,
    })
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
        if term.path().is_symlink() || !term.path().is_dir() { continue; }
        let term_name = term.file_name().to_string_lossy().into_owned();
        component(&term_name)?;
        for course in fs::read_dir(term.path()).map_err(|e| e.to_string())?.take(500) {
            let course = course.map_err(|e| e.to_string())?;
            if course.path().is_symlink() || !course.path().is_dir() { continue; }
            let course_name = course.file_name().to_string_lossy().into_owned();
            component(&course_name)?;
            for section in fs::read_dir(course.path()).map_err(|e| e.to_string())?.take(500) {
                let section = section.map_err(|e| e.to_string())?;
                if section.path().is_symlink() || !section.path().is_dir() { continue; }
                let section_name = section.file_name().to_string_lossy().into_owned();
                // A course-level materials directory is not a section.
                if section_name == "materials" { continue; }
                component(&section_name)?;
                for (kind, deleted) in [("notes", false), ("deleted-notes", true)] {
                    let dir = section.path().join(kind);
                    if !dir.is_dir() || dir.is_symlink() { continue; }
                    if !dir.canonicalize().map_err(|e| e.to_string())?.starts_with(&canonical) {
                        return Err("Notes folder points outside storage root".into());
                    }
                    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.take(500) {
                        let path = entry.map_err(|e| e.to_string())?.path();
                        if path.is_symlink() || path.extension().and_then(|s| s.to_str()) != Some("md") { continue; }
                        let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                        uuid::Uuid::parse_str(&id).map_err(|_| "Invalid note ID in storage")?;
                        let bytes = path.metadata().map_err(|e| e.to_string())?.len();
                        if bytes > MAX_NOTE_BYTES { return Err("A note exceeds the 1 MB backup limit".into()); }
                        total = total.checked_add(bytes).ok_or("Note backup size overflow")?;
                        if total > MAX_BACKUP_NOTE_BYTES || result.len() >= MAX_BACKUP_NOTES {
                            return Err("Notes exceed the local backup safety limit".into());
                        }
                        let text = fs::read_to_string(path).map_err(|_| "A note is not UTF-8 text")?;
                        result.push(LocalNoteBackup { id, term: term_name.clone(), course: course_name.clone(), section: section_name.clone(), text, deleted });
                    }
                }
            }
        }
    }
    Ok(result)
}

pub fn restore_notes(app: &AppHandle, notes: &[LocalNoteBackup]) -> Result<(), String> {
    if notes.len() > MAX_BACKUP_NOTES { return Err("Too many notes in backup".into()); }
    let mut total = 0usize;
    for note in notes {
        uuid::Uuid::parse_str(&note.id).map_err(|_| "Invalid note ID in backup")?;
        component(&note.term)?; component(&note.course)?; component(&note.section)?;
        if note.text.len() as u64 > MAX_NOTE_BYTES { return Err("A note exceeds the 1 MB limit".into()); }
        total = total.checked_add(note.text.len()).ok_or("Note backup size overflow")?;
        if total as u64 > MAX_BACKUP_NOTE_BYTES { return Err("Notes exceed the local backup safety limit".into()); }
    }
    let state = app.state::<FileStore>();
    let _guard = state.0.lock().map_err(|_| "Storage lock unavailable")?;
    for note in notes {
        let context = CourseFolder { term: note.term.clone(), course: note.course.clone(), section: note.section.clone() };
        let path = if note.deleted {
            folder(app, &context, "deleted-notes")?.join(format!("{}.md", note.id))
        } else {
            note_path(app, &context, &note.id)?
        };
        if path.is_symlink() { return Err("Cannot restore a note through a symbolic link".into()); }
        let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        fs::write(&temp, note.text.as_bytes()).map_err(|e| e.to_string())?;
        fs::rename(&temp, &path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn material_path(app: &AppHandle, course: &CourseKey, url: &Url) -> Result<PathBuf, String> {
    let path = materials_dir(app, course)?.join(material_name(url));
    if path.is_symlink() {
        return Err("Cannot read symbolic link".into());
    }
    Ok(path)
}
/// `<file name>-<digest>.<ext>`: readable in Finder, unique per URL, and the
/// extension lets the asset responder send the right media type.
fn material_name(url: &Url) -> String {
    let segment = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("file");
    let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
    let (stem, extension) = match decoded.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 12 && ext.chars().all(|c| c.is_ascii_alphanumeric()) => (stem, Some(ext)),
        _ => (decoded.as_ref(), None),
    };
    let stem: String = stem
        .chars()
        .map(|c| if c.is_control() || "/\\:".contains(c) { '_' } else { c })
        .take(80)
        .collect();
    let digest = format!("{:x}", Sha256::digest(url.as_str()));
    match extension {
        Some(ext) => format!("{}-{}.{}", stem.trim(), &digest[..8], ext.to_ascii_lowercase()),
        None => format!("{}-{}", stem.trim(), &digest[..8]),
    }
}
fn material_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "Invalid material URL")?;
    if url.scheme() != "https"
        || url.host_str() != Some("lms.s.isct.ac.jp")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url.path().contains("/pluginfile.php/")
    {
        return Err("Only LMS attachment URLs can be cached".into());
    }
    Ok(url)
}
#[tauri::command]
pub async fn course_storage_info(
    webview: tauri::Webview,
    app: AppHandle,
) -> Result<StorageInfo, String> {
    crate::require_local_main(&webview)?;
    let (root, custom) = root(&app)?;
    Ok(StorageInfo {
        root: root.to_string_lossy().into(),
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
    let id = if id.is_empty() { uuid::Uuid::now_v7().to_string() } else { id };
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
            describe(path, id, context.section, false, false)
        })();
        let _ = fs::remove_file(temp);
        result
    })
    .await
    .map_err(|e| e.to_string())??;
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
pub async fn move_course_note(webview: tauri::Webview, app: AppHandle, context: CourseFolder, id: String, restore: bool) -> Result<bool, String> {
    crate::require_local_main(&webview)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<FileStore>();
        let _guard = state.0.lock().map_err(|_| "Storage lock unavailable")?;
        let note = note_path(&app, &context, &id)?;
        let trash = folder(&app, &context, "deleted-notes")?.join(format!("{id}.md"));
        if trash.is_symlink() { return Err("Cannot move symbolic link".into()); }
        let (source, target) = if restore { (trash, note) } else { (note, trash) };
        if target.exists() { return Err("A note already exists at the destination".into()); }
        fs::rename(source, target).map_err(|e| e.to_string())?;
        Ok(true)
    }).await.map_err(|e| e.to_string())?
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
    let url = material_url(&url)?;
    let path = material_path(&app, &context, &url)?;
    if !path.is_file() {
        if !download {
            return Ok(Material::Missing);
        }
        #[cfg(desktop)]
        {
            let downloader = app.state::<crate::lms_downloader::Downloader>();
            let limit = limit.map(|bytes| bytes.clamp(1, MAX_AUTO_DOWNLOAD_BYTES));
            if let crate::lms_downloader::Fetched::TooLarge(bytes) = downloader
                .fetch(&app, &url, path.clone(), limit)
                .await
                .inspect_err(|error| eprintln!("[lms-file] {url}: {error}"))?
            {
                return Ok(Material::TooLarge { bytes });
            }
        }
        #[cfg(mobile)]
        {
            let _ = limit;
            return Err("Course files are downloaded in the desktop app".into());
        }
    }
    // Opening counts as use: auto-clean removes files unused for a while.
    let _ = fs::File::options().write(true).open(&path).and_then(|file| file.set_modified(std::time::SystemTime::now()));
    let file = tauri::async_runtime::spawn_blocking(move || describe(path, url.to_string(), String::new(), true, read_text))
        .await
        .map_err(|e| e.to_string())??;
    app.asset_protocol_scope().allow_file(&file.path).map_err(|e| e.to_string())?;
    Ok(Material::Ready { file })
}
/// A path the app saved under the storage root; anything else is refused.
fn stored_file(app: &AppHandle, path: &str) -> Result<PathBuf, String> {
    let (root, _) = root(app)?;
    let path = PathBuf::from(path).canonicalize().map_err(|e| e.to_string())?;
    if !path.is_file() || !path.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("File is outside the course folder".into());
    }
    Ok(path)
}
#[tauri::command]
pub async fn open_course_file(webview: tauri::Webview, app: AppHandle, path: String, reveal: bool) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    crate::require_local_main(&webview)?;
    let path = stored_file(&app, &path)?;
    if reveal {
        app.opener().reveal_item_in_dir(&path).map_err(|e| e.to_string())
    } else {
        app.opener().open_path(path.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
    }
}
/// Remove downloaded course files not opened for `days` days. Notes are kept.
#[tauri::command]
pub async fn clean_course_materials(webview: tauri::Webview, app: AppHandle, days: u64) -> Result<usize, String> {
    crate::require_local_main(&webview)?;
    let (root, _) = root(&app)?;
    let days = days.clamp(1, MAX_RETENTION_DAYS);
    let age = std::time::Duration::from_secs(days * 24 * 60 * 60);
    tauri::async_runtime::spawn_blocking(move || {
        let mut removed = 0;
        // <term>/<course>/materials, and <term>/<course>/<section>/materials from earlier versions.
        let mut pending = vec![(root, 0)];
        while let Some((dir, depth)) = pending.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_symlink() { continue; }
                if path.is_dir() {
                    if depth < 3 { pending.push((path, depth + 1)); }
                    continue;
                }
                let in_materials = dir.file_name().is_some_and(|name| name == "materials");
                let stale = entry.metadata().and_then(|m| m.modified()).ok()
                    .and_then(|time| time.elapsed().ok()).is_some_and(|elapsed| elapsed > age);
                if in_materials && stale && fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    })
    .await
    .map_err(|e| e.to_string())
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
    fn rejects_traversal_and_external_downloads() {
        for value in ["..", ".", "x/y", "x\\y", "", "../../notes"] {
            assert!(component(value).is_err());
        }
        assert!(component("2026-3Q").is_ok());
        assert!(component("CSC.T251").is_ok());
        for url in [
            "https://evil.test/pluginfile.php/1/a.pdf",
            "https://lms.s.isct.ac.jp/login/index.php",
            "https://user:pass@lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf",
            "http://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf",
        ] {
            assert!(material_url(url).is_err());
        }
        assert!(material_url("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf").is_ok());
    }
    #[test]
    fn names_materials_readably_and_uniquely() {
        let name = |raw: &str| material_name(&Url::parse(raw).unwrap());
        let japanese = name("https://lms.s.isct.ac.jp/2025/pluginfile.php/224957/mod_resource/content/1/01%E5%9B%9E%E7%9B%AE%20SW-0%20v6.pdf");
        assert!(japanese.starts_with("01回目 SW-0 v6-") && japanese.ends_with(".pdf"), "{japanese}");
        assert!(name("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/Photo.PNG?forcedownload=1").ends_with(".png"));
        assert!(!name("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/README").contains('.'));
        // Same file name from two activities: two files.
        assert_ne!(name("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf"), name("https://lms.s.isct.ac.jp/2025/pluginfile.php/2/a.pdf"));
        // An encoded slash cannot escape the folder.
        assert!(!name("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/..%2F..%2Fx.pdf").contains('/'));
    }
}
