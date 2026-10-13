//! Desktop school-download handling, including the macOS PDF viewer save hook.

use isct_core::{
    lifecycle::SchoolSite, runtime_tasks::RuntimeTasks, school_browser::sanitize_school_url,
};
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::collections::HashMap;
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(desktop)]
use tauri::webview::DownloadEvent;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

#[cfg(target_os = "macos")]
#[path = "school_pdf_save.rs"]
mod pdf_save;

const MAIN_WINDOW: &str = "main";
const DOWNLOAD_NAME_LIMIT: usize = 200;

#[derive(Default)]
struct DownloadState {
    active: Option<SchoolSite>,
    pending: Vec<(String, PathBuf)>,
    saved: HashSet<PathBuf>,
    #[cfg(target_os = "macos")]
    delegates: HashMap<SchoolSite, pdf_save::DelegateHandle>,
}

#[derive(Default)]
struct DownloadInner {
    state: Mutex<DownloadState>,
    tasks: RuntimeTasks,
}

#[derive(Clone, Default)]
pub(crate) struct SchoolDownloads(Arc<DownloadInner>);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchoolDownloadEvent {
    site: SchoolSite,
    /// `started`, `done` or `failed`.
    state: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

impl SchoolDownloads {
    pub(crate) fn set_visible(&self, site: Option<SchoolSite>) {
        if let Ok(mut state) = self.0.state.lock() {
            state.active = site;
        }
    }

    #[cfg(desktop)]
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
                    eprintln!("[school-download] {site:?} download refused");
                    return false;
                }
                let name = file_name(destination);
                emit_download(
                    app,
                    site,
                    "started",
                    name,
                    sanitize_school_url(url.as_str()),
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
    fn save_viewer_data(&self, app: &AppHandle, site: SchoolSite, saved: pdf_save::SavedData) {
        let active = self.0.state.lock().ok().and_then(|state| state.active);
        let allowed = saved
            .origin
            .as_deref()
            .is_some_and(|origin| download_allowed(origin, site, active));
        if !allowed {
            eprintln!("[school-download] {site:?} PDF save refused");
            return;
        }

        let name = download_file_name(&saved.suggested_name, &saved.mime_type);
        let Ok(directory) = app.path().download_dir() else {
            emit_download(app, site, "failed", name, None, None);
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
                    eprintln!("[school-download] {site:?} PDF save failed: {error}");
                    None
                }
                Err(error) => {
                    eprintln!("[school-download] {site:?} PDF save task failed: {error}");
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
            emit_download(&failed_app, site, "failed", failed_name, None, None);
        }
    }

    /// Installs and retains the macOS PDF viewer save shim for this view.
    #[cfg(target_os = "macos")]
    pub(crate) fn install(
        &self,
        app: &AppHandle,
        site: SchoolSite,
        view: &crate::school_web::NativeWebview,
    ) {
        let downloads = Arc::downgrade(&self.0);
        let app = app.clone();
        let result = view.with_webview(move |platform| {
            let downloads_for_save = downloads.clone();
            let on_save = Box::new(move |saved| {
                if let Some(downloads) = downloads_for_save.upgrade() {
                    SchoolDownloads(downloads).save_viewer_data(&app, site, saved);
                }
            });
            // SAFETY: `inner()` is this view's live WKWebView, and
            // `with_webview` invokes this closure on the main thread.
            let delegate = unsafe { pdf_save::install(platform.inner(), on_save) };
            if let (Some(downloads), Some(delegate)) = (downloads.upgrade(), delegate) {
                if let Ok(mut state) = downloads.state.lock() {
                    state.delegates.insert(site, delegate);
                }
            }
        });
        if let Err(error) = result {
            eprintln!("[school-download] {site:?} PDF save hook unavailable: {error}");
        }
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
            #[cfg(target_os = "macos")]
            state.delegates.clear();
        }
    }

    fn finish(&self, app: &AppHandle, site: SchoolSite, name: String, saved: Option<PathBuf>) {
        match saved {
            Some(path) => {
                if let Ok(mut state) = self.0.state.lock() {
                    state.saved.insert(path.clone());
                }
                emit_download(app, site, "done", name, None, Some(&path));
            }
            None => emit_download(app, site, "failed", name, None, None),
        }
    }
}

fn emit_download(
    app: &AppHandle,
    site: SchoolSite,
    state: &'static str,
    name: String,
    url: Option<String>,
    path: Option<&Path>,
) {
    eprintln!("[school-download] {site:?} {state} {name}");
    let _ = app.emit_to(
        MAIN_WINDOW,
        "school-download",
        SchoolDownloadEvent {
            site,
            state,
            name,
            url,
            path: path.and_then(Path::to_str).map(str::to_string),
        },
    );
}

fn download_allowed(raw_url: &str, site: SchoolSite, active: Option<SchoolSite>) -> bool {
    active == Some(site) && sanitize_school_url(raw_url).is_some()
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
mod tests {
    use super::*;

    #[test]
    fn downloads_are_limited_to_the_visible_school_view() {
        let lms = SchoolSite::ScienceLms;
        let file = "https://lms.s.isct.ac.jp/2025/pluginfile.php/224823/mod_resource/content/3/LAIIAssignment1.pdf";
        assert!(download_allowed(file, lms, Some(lms)));
        assert!(!download_allowed(file, lms, None));
        assert!(!download_allowed(file, lms, Some(SchoolSite::Kyomu)));
        for url in [
            "https://example.com/file.pdf",
            "http://lms.s.isct.ac.jp/2025/pluginfile.php/1/file.pdf",
            "https://user:secret@lms.s.isct.ac.jp/file.pdf",
            "blob:https://lms.s.isct.ac.jp/uuid",
            "data:application/pdf;base64,AAAA",
        ] {
            assert!(!download_allowed(url, lms, Some(lms)), "{url}");
        }
    }

    #[test]
    fn download_destinations_stay_directly_in_the_download_folder() {
        let downloads = Path::new("/Users/student/Downloads");
        assert!(download_destination_ok(
            &downloads.join("Lecture 1 (2).pptx"),
            downloads
        ));
        for bad in [
            "/Users/student/Downloads/../.zshrc",
            "/Users/student/Downloads/nested/file.docx",
            "/Users/student/Desktop/file.docx",
            "/Users/student/Downloads/line\nbreak.pdf",
            "/Users/student/Downloads/back\\slash.pdf",
            "/Users/student/Downloads",
        ] {
            assert!(!download_destination_ok(Path::new(bad), downloads));
        }
    }

    #[test]
    fn viewer_file_names_are_plain_and_bounded() {
        let pdf = "application/pdf";
        assert_eq!(
            download_file_name("LAIIAssignment1.pdf", pdf),
            "LAIIAssignment1.pdf"
        );
        assert_eq!(
            download_file_name("../../etc/passwd", pdf),
            "_.._etc_passwd.pdf"
        );
        assert_eq!(download_file_name("..hidden.pdf", pdf), "hidden.pdf");
        assert_eq!(download_file_name("a\u{0}b\nc.pdf", pdf), "abc.pdf");
        assert_eq!(download_file_name("C:\\x\\講義.pdf", pdf), "C__x_講義.pdf");
        assert_eq!(download_file_name("Slides", pdf), "Slides.pdf");
        assert_eq!(download_file_name("Slides.PDF", pdf), "Slides.PDF");
        assert_eq!(download_file_name(" .. ", pdf), "download.pdf");
        assert_eq!(download_file_name("", "application/zip"), "download");
        assert_eq!(download_file_name("notes.txt", "text/plain"), "notes.txt");
        let long = download_file_name(&format!("{}.pdf", "講".repeat(400)), pdf);
        assert!(long.len() <= DOWNLOAD_NAME_LIMIT, "{}", long.len());
        assert!(long.ends_with("講.pdf"));
        assert!(safe_file_name(&long));
    }

    #[test]
    fn unique_names_number_before_the_extension() {
        assert_eq!(
            download_candidates("Report.final.pdf")
                .take(3)
                .collect::<Vec<_>>(),
            [
                "Report.final.pdf",
                "Report.final (1).pdf",
                "Report.final (2).pdf"
            ]
        );
        assert_eq!(
            download_candidates("README").nth(1).as_deref(),
            Some("README (1)")
        );
    }

    #[test]
    fn viewer_saves_never_overwrite_existing_files() {
        let directory = std::env::temp_dir().join(format!(
            "isct-companion-download-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("a.pdf"), b"old").unwrap();
        let first = write_unique(&directory, "a.pdf", b"new").unwrap();
        let second = write_unique(&directory, "a.pdf", b"newer").unwrap();
        assert_eq!(first, directory.join("a (1).pdf"));
        assert_eq!(second, directory.join("a (2).pdf"));
        assert_eq!(fs::read(directory.join("a.pdf")).unwrap(), b"old");
        assert_eq!(fs::read(&second).unwrap(), b"newer");
        fs::remove_dir_all(&directory).unwrap();
    }
}
