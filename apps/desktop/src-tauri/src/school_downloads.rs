//! Desktop school-download handling: what school pages hand over, saved to
//! Downloads and announced to the app.

use app_core::{
    events::{CoreEvent, SchoolDownloadEvent, SchoolDownloadState},
    lifecycle::SchoolSite,
    ports::EventSink,
    runtime_tasks::RuntimeTasks,
    school_browser::{download_permitted, restore_record, ViewRole},
};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

const DOWNLOAD_NAME_LIMIT: usize = 200;

#[derive(Default)]
struct DownloadState {
    active: Option<SchoolSite>,
    pending: Vec<(String, PathBuf)>,
    saved: HashSet<PathBuf>,
}

#[derive(Default)]
struct DownloadInner {
    state: Mutex<DownloadState>,
    tasks: RuntimeTasks,
}

#[derive(Clone, Default)]
pub(crate) struct SchoolDownloads(Arc<DownloadInner>);

impl SchoolDownloads {
    pub(crate) fn set_visible(&self, site: Option<SchoolSite>) {
        if let Ok(mut state) = self.0.state.lock() {
            state.active = site;
        }
    }

    pub(crate) fn download(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        event: DownloadEvent<'_>,
    ) -> bool {
        match event {
            DownloadEvent::Requested { url, destination } => {
                let download_dir = app.path().download_dir().ok();
                let accepted = download_dir
                    .is_some_and(|directory| download_destination_ok(destination, &directory))
                    && self.0.state.lock().ok().is_some_and(|mut state| {
                        if !download_allowed(url.as_str(), site, state.active) {
                            return false;
                        }
                        state.pending.push((url.to_string(), destination.clone()));
                        true
                    });
                if !accepted {
                    log::warn!("[school-download] {site:?} download refused");
                    return false;
                }
                let name = file_name(destination);
                emit_download(
                    app,
                    site,
                    SchoolDownloadState::Started,
                    name,
                    restore_record(url.as_str()),
                    None,
                );
                true
            }
            DownloadEvent::Finished { url, success, .. } => {
                let destination = self.0.state.lock().ok().and_then(|mut state| {
                    let index = state
                        .pending
                        .iter()
                        .position(|(pending, _)| pending == url.as_str())?;
                    Some(state.pending.remove(index).1)
                });
                // Refused downloads also finish unsuccessfully, but were never announced.
                if let Some(destination) = destination {
                    let name = file_name(&destination);
                    self.finish(app, site, name, success.then_some(destination));
                }
                true
            }
            _ => true,
        }
    }

    /// Writes bytes supplied by WebKit's built-in PDF viewer without blocking
    /// the main thread. The owned task is drained during shutdown.
    #[cfg(target_os = "macos")]
    pub(crate) fn save_viewer_data(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        saved: crate::school_webkit::SavedData,
    ) {
        let active = self.0.state.lock().ok().and_then(|state| state.active);
        let allowed = saved
            .origin
            .as_deref()
            .is_some_and(|origin| download_allowed(origin, site, active));
        if !allowed {
            log::warn!("[school-download] {site:?} PDF save refused");
            return;
        }

        let name = download_file_name(&saved.suggested_name, &saved.mime_type);
        let Ok(directory) = app.path().download_dir() else {
            emit_download(app, site, SchoolDownloadState::Failed, name, None, None);
            return;
        };
        let app = app.clone();
        let failed_app = app.clone();
        let downloads = self.clone();
        let fallback_name = name.clone();
        let failed_name = fallback_name.clone();
        let runtime = tauri::async_runtime::handle();
        let _runtime_guard = runtime.inner().enter();
        let spawned = self.0.tasks.spawn(async move {
            let written = match tokio::task::spawn_blocking(move || {
                write_unique(&directory, &name, &saved.bytes)
            })
            .await
            {
                Ok(Ok(path)) => Some(path),
                Ok(Err(error)) => {
                    log::error!("[school-download] {site:?} PDF save failed: {error}");
                    None
                }
                Err(error) => {
                    log::error!("[school-download] {site:?} PDF save task failed: {error}");
                    None
                }
            };
            let name = written
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|file| file.to_string_lossy().into_owned())
                .unwrap_or(fallback_name);
            downloads.finish(&app, site, name, written);
        });
        if !spawned {
            emit_download(
                &failed_app,
                site,
                SchoolDownloadState::Failed,
                failed_name,
                None,
                None,
            );
        }
    }

    /// Where a file a popup downloads is written: a free name in Downloads,
    /// announced as started. `None` refuses it.
    #[cfg(target_os = "macos")]
    pub(crate) fn popup_destination(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        url: Option<&url::Url>,
        suggested: &str,
    ) -> Option<PathBuf> {
        let active = self.0.state.lock().ok().and_then(|state| state.active);
        if !url.is_some_and(|url| download_allowed(url.as_str(), site, active)) {
            log::warn!("[school-download] {site:?} popup download refused");
            return None;
        }
        let directory = app.path().download_dir().ok()?;
        let name = download_file_name(suggested, "");
        let path = download_candidates(&name)
            .map(|candidate| directory.join(candidate))
            .find(|path| !path.exists())?;
        emit_download(
            app,
            site,
            SchoolDownloadState::Started,
            file_name(&path),
            url.and_then(|url| restore_record(url.as_str())),
            None,
        );
        Some(path)
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn popup_finished(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        path: PathBuf,
        saved: bool,
    ) {
        let name = file_name(&path);
        self.finish(app, site, name, saved.then_some(path));
    }

    /// Shows a file saved from a school page this session in Finder. Unknown
    /// paths are refused.
    pub(crate) fn reveal(&self, app: &AppHandle, path: &str) -> Result<bool, String> {
        let path = PathBuf::from(path);
        let known = self
            .0
            .state
            .lock()
            .map_err(|_| "School download state is unavailable.")?
            .saved
            .contains(&path);
        if !known {
            return Ok(false);
        }
        app.opener()
            .reveal_item_in_dir(&path)
            .map(|_| true)
            .map_err(|_| "The file could not be shown. It may have been moved.".into())
    }

    pub(crate) async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
        if let Ok(mut state) = self.0.state.lock() {
            state.active = None;
            state.pending.clear();
        }
    }

    fn finish(&self, app: &AppHandle, site: SchoolSite, name: String, saved: Option<PathBuf>) {
        match saved {
            Some(path) => {
                if let Ok(mut state) = self.0.state.lock() {
                    state.saved.insert(path.clone());
                }
                emit_download(
                    app,
                    site,
                    SchoolDownloadState::Done,
                    name,
                    None,
                    Some(&path),
                );
            }
            None => emit_download(app, site, SchoolDownloadState::Failed, name, None, None),
        }
    }
}

fn emit_download(
    app: &AppHandle,
    site: SchoolSite,
    state: SchoolDownloadState,
    name: String,
    url: Option<String>,
    path: Option<&Path>,
) {
    match state {
        SchoolDownloadState::Failed => log::warn!("[school-download] {site:?} {state:?} {name}"),
        SchoolDownloadState::Started | SchoolDownloadState::Done => {
            log::info!("[school-download] {site:?} {state:?} {name}")
        }
    }
    let _ = crate::core_ports::DesktopEvents(app.clone()).emit(CoreEvent::SchoolDownload(
        SchoolDownloadEvent {
            site,
            state,
            name,
            url,
            path: path.and_then(Path::to_str).map(str::to_string),
        },
    ));
}

/// Core's rule: what the person is looking at may hand over any file, a
/// hidden view only files from adapter sites.
fn download_allowed(raw_url: &str, site: SchoolSite, active: Option<SchoolSite>) -> bool {
    let role = if active == Some(site) {
        ViewRole::User
    } else {
        ViewRole::Automation
    };
    download_permitted(role, raw_url)
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name
            .chars()
            .any(|character| matches!(character, '/' | '\\') || character.is_control())
}

fn download_destination_ok(destination: &Path, download_dir: &Path) -> bool {
    destination.parent() == Some(download_dir)
        && destination
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(safe_file_name)
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(index) if index > 0 && name.len() - index <= 16 => name.split_at(index),
        _ => (name, ""),
    }
}

fn download_file_name(suggested: &str, mime_type: &str) -> String {
    let cleaned = suggested
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| {
            if matches!(character, '/' | '\\' | ':') {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim().trim_start_matches('.').trim_start();
    let pdf = mime_type.eq_ignore_ascii_case("application/pdf");
    let mut name = match cleaned {
        "" if pdf => "download.pdf".to_string(),
        "" => "download".to_string(),
        cleaned => cleaned.to_string(),
    };
    if pdf && !name.to_ascii_lowercase().ends_with(".pdf") {
        name.push_str(".pdf");
    }
    if name.len() <= DOWNLOAD_NAME_LIMIT {
        return name;
    }
    let (stem, extension) = split_extension(&name);
    let mut keep = DOWNLOAD_NAME_LIMIT.saturating_sub(extension.len());
    while !stem.is_char_boundary(keep) {
        keep -= 1;
    }
    format!("{}{extension}", stem[..keep].trim_end())
}

fn download_candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split_extension(name);
    std::iter::once(name.to_string())
        .chain((1..1000).map(move |number| format!("{stem} ({number}){extension}")))
}

fn write_unique(directory: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    for candidate in download_candidates(name) {
        let path = directory.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
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

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "tests/school_downloads.rs"]
mod tests;
