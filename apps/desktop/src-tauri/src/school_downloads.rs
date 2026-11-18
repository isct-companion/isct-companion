//! Desktop school-download handling: what school pages hand over, saved to
//! Downloads and announced to the app.

use crate::downloads::file_name;
#[cfg(target_os = "macos")]
use crate::downloads::write_unique;
#[cfg(target_os = "macos")]
use app_core::download_names::{name_candidates, safe_name};
use app_core::{
    events::{CoreEvent, SchoolDownloadEvent, SchoolDownloadState},
    lifecycle::SchoolSite,
    ports::EventSink,
    runtime_tasks::RuntimeTasks,
    school_browser::download_permitted,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Manager};

#[derive(Default)]
struct DownloadState {
    pending: Vec<(String, PathBuf)>,
}

#[derive(Default)]
struct DownloadInner {
    state: Mutex<DownloadState>,
    tasks: RuntimeTasks,
}

#[derive(Clone, Default)]
pub(crate) struct SchoolDownloads(Arc<DownloadInner>);

impl SchoolDownloads {
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
                        if !download_permitted(url.as_str()) {
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
                emit_download(app, site, SchoolDownloadState::Started, name, None);
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
        let allowed = saved.origin.as_deref().is_some_and(download_permitted);
        if !allowed {
            log::warn!("[school-download] {site:?} PDF save refused");
            return;
        }

        let name = safe_name(&saved.suggested_name, &saved.mime_type);
        let Ok(directory) = app.path().download_dir() else {
            emit_download(app, site, SchoolDownloadState::Failed, name, None);
            return;
        };
        // The viewer hands the bytes over at once; the write is still a save
        // the person started, announced like any other.
        emit_download(app, site, SchoolDownloadState::Started, name.clone(), None);
        let app = app.clone();
        let failed_app = app.clone();
        let downloads = self.clone();
        let fallback_name = name.clone();
        let failed_name = fallback_name.clone();
        let runtime = tauri::async_runtime::handle();
        let _runtime_guard = runtime.inner().enter();
        let spawned = self.0.tasks.spawn(async move {
            let written = match tokio::task::spawn_blocking(move || {
                write_unique(&directory, &name, saved.bytes.as_slice())
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
        if !url.is_some_and(|url| download_permitted(url.as_str())) {
            log::warn!("[school-download] {site:?} popup download refused");
            return None;
        }
        let directory = app.path().download_dir().ok()?;
        let name = safe_name(suggested, "");
        let path = name_candidates(&name)
            .map(|candidate| directory.join(candidate))
            .find(|path| !path.exists())?;
        emit_download(
            app,
            site,
            SchoolDownloadState::Started,
            file_name(&path),
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

    pub(crate) async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
        if let Ok(mut state) = self.0.state.lock() {
            state.pending.clear();
        }
    }

    fn finish(&self, app: &AppHandle, site: SchoolSite, name: String, saved: Option<PathBuf>) {
        match saved {
            Some(path) => {
                if let Err(error) = app
                    .state::<crate::downloads::Downloads>()
                    .finished(path.clone())
                {
                    log::error!("[school-download] could not register saved file: {error:?}");
                    emit_download(app, site, SchoolDownloadState::Failed, name, None);
                    return;
                }
                emit_download(app, site, SchoolDownloadState::Done, name, Some(&path));
            }
            None => emit_download(app, site, SchoolDownloadState::Failed, name, None),
        }
    }
}

fn emit_download(
    app: &AppHandle,
    site: SchoolSite,
    state: SchoolDownloadState,
    name: String,
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
            path: path.and_then(Path::to_str).map(str::to_string),
        },
    ));
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

#[cfg(test)]
#[path = "tests/school_downloads.rs"]
mod tests;
