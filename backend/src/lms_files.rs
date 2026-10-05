//! Local course materials and notes. No file bytes or session cookies reach JS.
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

pub struct FileStore(
    pub Mutex<()>,
    pub std::sync::Arc<tauri::async_runtime::Mutex<()>>,
    pub PathBuf,
);
impl Drop for FileStore { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.2); } }
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseFolder {
    pub term: String,
    pub course: String,
    pub section: String,
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
    let (root, _) = root(app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut path = root.clone();
    for name in [&context.term, &context.course, &context.section, kind] {
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
fn material_path(app: &AppHandle, context: &CourseFolder, url: &str, persistent: bool) -> Result<PathBuf, String> {
    let url = material_url(url)?;
    let original = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .unwrap_or("file");
    // Keep the extension when bounding long encoded Japanese filenames so
    // the asset responder still supplies the correct PDF/image media type.
    let extension = original
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| ext.len() <= 12 && ext.chars().all(|c| c.is_ascii_alphanumeric()));
    let stem = if extension.is_some() {
        original.rsplit_once('.').unwrap().0
    } else {
        original
    };
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let name = extension.map(|ext| format!("{name}.{ext}")).unwrap_or(name);
    let digest = format!("{:x}", Sha256::digest(url.as_str()));
    let dir = if persistent { folder(app, context, "materials")? } else {
        let state = app.state::<FileStore>();
        let mut dir = state.2.clone();
        for part in [&context.term, &context.course, &context.section] { dir.push(component(part)?); }
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        dir
    };
    let path = dir.join(format!("{}-{name}", &digest[..16]));
    if path.is_symlink() {
        return Err("Cannot read symbolic link".into());
    }
    Ok(path)
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
fn expose(app: &AppHandle, file: LocalFile) -> Result<LocalFile, String> {
    app.asset_protocol_scope()
        .allow_file(&file.path)
        .map_err(|e| e.to_string())?;
    Ok(file)
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
    if text.len() > 1_000_000 {
        return Err("Notes are limited to 1 MB".into());
    }
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
#[tauri::command]
pub async fn open_course_material(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseFolder,
    url: String,
    download: bool,
    read_text: bool,
    auto_save: Option<bool>,
) -> Result<Option<LocalFile>, String> {
    crate::require_local_main(&webview)?;
    // Await asynchronously before occupying a blocking worker. At most one
    // attachment client is alive; queued downloads do not allocate OS threads.
    let permit = if download {
        Some(app.state::<FileStore>().1.clone().lock_owned().await)
    } else {
        None
    };
    if download && !app.state::<crate::school_web::SchoolWebController>().attachment_view_ready() {
        for _ in 0..150 {
            app.state::<crate::school_web::SchoolWebController>().prepare_attachment_view(&app)?;
            let ready = app.state::<crate::school_web::SchoolWebController>().attachment_view_ready();
            if ready { break; }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        if !app.state::<crate::school_web::SchoolWebController>().attachment_view_ready() {
            return Err("LMS sign-in required; open LMS and reconnect before downloading".into());
        }
    }
    let handle = app.clone();
    let file =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<LocalFile>, String> {
            let _permit = permit;
            let path = material_path(&handle, &context, &url, auto_save.unwrap_or(false))?;
            if !auto_save.unwrap_or(false) {
                let (base, _) = root(&handle)?;
                let mut saved = base.clone();
                for part in [&context.term, &context.course, &context.section, "materials"] { saved.push(component(part)?); }
                saved.push(path.file_name().ok_or("File name is unavailable")?);
                if saved.is_file() {
                    if saved.is_symlink() || !saved.canonicalize().map_err(|e| e.to_string())?.starts_with(base.canonicalize().map_err(|e| e.to_string())?) {
                        return Err("Cached file points outside storage root".into());
                    }
                    return describe(saved, url, context.section, true, read_text).map(Some);
                }
            }
            if path.is_file() {
                return describe(path, url, context.section, true, read_text).map(Some);
            }
            if !download {
                return Ok(None);
            }
            // Serialize downloads separately from note writes: bounded TLS/client
            // memory without delaying autosave behind a large attachment.
            if path.is_file() {
                return describe(path, url, context.section, true, read_text).map(Some);
            }
            let lms = handle
                .get_webview("school-web-science-lms")
                .ok_or("Open LMS and sign in before downloading")?;
            let target = material_url(&url)?;
            let cookies = lms
                .cookies_for_url(target.clone())
                .map_err(|e| e.to_string())?;
            if cookies.is_empty() {
                return Err("LMS sign-in required".into());
            }
            let cookie = cookies
                .iter()
                .map(|c| format!("{}={}", c.name(), c.value()))
                .collect::<Vec<_>>()
                .join("; ");
            let client = reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() < 3 && material_url(attempt.url().as_str()).is_ok() { attempt.follow() } else { attempt.stop() }
                }))
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(|e| e.to_string())?;
            let response = client
                .get(target)
                .header(reqwest::header::COOKIE, cookie)
                .send()
                .map_err(|e| { use std::error::Error; format!("File download failed: {}", e.source().map(|cause| cause.to_string()).unwrap_or_else(|| e.to_string())) })?;
            if !response.status().is_success()
                || response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|h| h.to_str().ok())
                    .unwrap_or("")
                    .contains("text/html")
            {
                return Err(format!("LMS did not return a file (HTTP {}); open LMS and reconnect before downloading", response.status().as_u16()));
            }
            const LIMIT: u64 = 128 * 1024 * 1024;
            if response.content_length().is_some_and(|n| n > LIMIT) {
                return Err("Material exceeds the 128 MB download limit".into());
            }
            let temp = path.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
            let result = (|| {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)
                    .map_err(|e| e.to_string())?;
                let bytes = std::io::copy(&mut response.take(LIMIT + 1), &mut output)
                    .map_err(|e| e.to_string())?;
                if bytes > LIMIT {
                    return Err("Material exceeds the 128 MB download limit".into());
                }
                output.sync_all().map_err(|e| e.to_string())?;
                fs::rename(&temp, &path).map_err(|e| e.to_string())?;
                describe(path, url, context.section, false, read_text).map(Some)
            })();
            let _ = fs::remove_file(temp);
            result
        })
        .await
        .map_err(|e| e.to_string())??;
    file.map(|file| expose(&app, file)).transpose()
}
#[tauri::command]
pub async fn save_course_material(webview: tauri::Webview, app: AppHandle, context: CourseFolder, url: String) -> Result<Option<String>, String> {
    crate::require_local_main(&webview)?;
    let cached = open_course_material(webview.clone(), app.clone(), context.clone(), url.clone(), false, false, Some(false)).await?;
    let file = match cached { Some(file) => file, None => open_course_material(webview, app.clone(), context.clone(), url, true, false, Some(false)).await?.ok_or("File was not downloaded")? };
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(desktop)] {
            use tauri_plugin_dialog::DialogExt;
            let name = file.name.get(17..).unwrap_or(&file.name);
            let target = app.dialog().file().set_title("Save course file").set_directory(app.path().download_dir().map_err(|e| e.to_string())?).set_file_name(name).blocking_save_file();
            let Some(target) = target else { return Ok(None); };
            let target = target.into_path().map_err(|e| e.to_string())?;
            if target != PathBuf::from(&file.path) { fs::copy(&file.path, &target).map_err(|e| e.to_string())?; }
            Ok(Some(target.to_string_lossy().into_owned()))
        }
        #[cfg(mobile)] {
            let path = folder(&app, &context, "materials")?.join(file.name);
            fs::copy(file.path, &path).map_err(|e| e.to_string())?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
    }).await.map_err(|e| e.to_string())?
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
}
