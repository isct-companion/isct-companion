//! Desktop file saves and system handoff. Callers validate their own sources.
use crate::{shell, CommandError};
use serde::Serialize;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

// Leave room for the extension and collision number on both desktop systems.
const DOWNLOAD_NAME_LIMIT: usize = 200;

#[derive(Serialize)]
pub(crate) struct SavedDownload {
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Default)]
pub(crate) struct Downloads(Mutex<HashSet<PathBuf>>);

impl Downloads {
    pub(crate) fn finished(&self, path: PathBuf) -> Result<SavedDownload, CommandError> {
        let saved = SavedDownload {
            name: file_name(&path),
            path: path.to_string_lossy().into_owned(),
        };
        self.0.lock().map_err(shell)?.insert(path.clone());
        #[cfg(target_os = "macos")]
        bounce_downloads_stack(&path);
        log::info!("[download] saved {}", saved.name);
        Ok(saved)
    }
}

/// Copy an already validated source. Core's owned runtime drains the save at exit.
pub(crate) async fn save_copy(
    app: AppHandle,
    source: PathBuf,
    name: String,
) -> Result<SavedDownload, CommandError> {
    let directory = app.path().download_dir().map_err(shell)?;
    crate::core_ports::runtime(&app)
        .run(async move {
            let path = tauri::async_runtime::spawn_blocking(move || {
                let source = fs::File::open(source)?;
                write_unique(&directory, &download_file_name(&name, ""), source)
            })
            .await
            .map_err(shell)?
            .map_err(shell)?;
            app.state::<Downloads>().finished(path)
        })
        .await
        .map_err(crate::coded)?
}

/// The caller has checked which local file this operation may hand to the OS.
pub(crate) fn show_file(app: &AppHandle, path: &Path, reveal: bool) -> Result<(), CommandError> {
    if reveal {
        app.opener().reveal_item_in_dir(path).map_err(shell)
    } else {
        app.opener()
            .open_path(path.to_string_lossy(), None::<&str>)
            .map_err(shell)
    }
}

/// Only successfully saved files from this session can be located here.
#[tauri::command]
pub(crate) fn reveal_download(webview: tauri::Webview, path: String) -> Result<bool, CommandError> {
    crate::require_local_main(&webview)?;
    let app = webview.app_handle();
    let path = PathBuf::from(path);
    if !app
        .state::<Downloads>()
        .0
        .lock()
        .map_err(shell)?
        .contains(&path)
        || !path.is_file()
    {
        return Ok(false);
    }
    show_file(app, &path, true)?;
    Ok(true)
}

pub(crate) fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 && name.len() - index <= 16 => name.split_at(index),
        _ => (name, ""),
    }
}

pub(crate) fn download_file_name(suggested: &str, mime_type: &str) -> String {
    let cleaned: String = suggested
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| match character {
            '/' | '\\' | ':' => '_',
            #[cfg(target_os = "windows")]
            '<' | '>' | '"' | '|' | '?' | '*' => '_',
            character => character,
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let mut name = if cleaned.is_empty() {
        "download"
    } else {
        cleaned
    }
    .to_owned();
    if mime_type.eq_ignore_ascii_case("application/pdf")
        && !name.to_ascii_lowercase().ends_with(".pdf")
    {
        name.push_str(".pdf");
    }
    #[cfg(target_os = "windows")]
    if reserved_windows_name(&name) {
        name.insert(0, '_');
    }
    if name.len() <= DOWNLOAD_NAME_LIMIT {
        return name;
    }
    let (stem, extension) = split_extension(&name);
    let mut keep = DOWNLOAD_NAME_LIMIT - extension.len();
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{extension}", stem[..keep].trim_end())
}

#[cfg(target_os = "windows")]
fn reserved_windows_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|number| {
                matches!(
                    number,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
}

pub(crate) fn download_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split_extension(name);
    std::iter::once(name.to_string())
        .chain((1..1000).map(move |number| format!("{stem} ({number}){extension}")))
}

/// Atomic name reservation; failed writes leave no partial download behind.
pub(crate) fn write_unique(
    directory: &Path,
    name: &str,
    mut source: impl Read,
) -> io::Result<PathBuf> {
    for candidate in download_candidates(name) {
        let path = directory.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = io::copy(&mut source, &mut file).and_then(|_| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free file name",
    ))
}

#[cfg(target_os = "macos")]
fn bounce_downloads_stack(path: &Path) {
    use objc2_foundation::{NSDistributedNotificationCenter, NSString};
    let name = NSString::from_str("com.apple.DownloadFileFinished");
    let file = NSString::from_str(&path.to_string_lossy());
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .postNotificationName_object(&name, Some(&file));
    }
}

#[cfg(test)]
#[path = "tests/downloads.rs"]
mod tests;
