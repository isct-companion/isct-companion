//! The system's own Copy and Share for a downloaded course file. A copied
//! file pastes as the file itself in Finder, Explorer, Mail or a chat app;
//! Share opens the system share UI at the button. Each platform's half lives
//! in its own file and only macOS and Windows have one; the frontend shows
//! neither action elsewhere (`nativeOs()`). Both commands accept only a file
//! under the course-files root (`lms_files::stored_file`).
use crate::{lms_files, CommandError};
use serde::Deserialize;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::webview::PlatformWebview;
use tauri::{AppHandle, Webview};

#[cfg(target_os = "macos")]
use crate::file_share_macos as platform;
#[cfg(target_os = "windows")]
use crate::file_share_windows as platform;

/// The share button's box in CSS pixels from the viewport's top left. Only
/// macOS anchors its picker; Windows places its share UI itself.
#[derive(Clone, Copy, Debug, Deserialize)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct Anchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[tauri::command]
pub async fn copy_course_file(
    webview: Webview,
    app: AppHandle,
    path: String,
) -> Result<(), CommandError> {
    crate::require_local_main(&webview)?;
    let path = lms_files::stored_file(&app, &path)?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let done = platform::copy(&webview, path).await;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let done: Result<(), String> = Err(format!("no file clipboard for {path:?}"));
    done.map_err(|detail| {
        log::warn!("[file-share] copy failed: {detail}");
        "share.copyFailed".into()
    })
}

#[tauri::command]
pub async fn share_course_file(
    webview: Webview,
    app: AppHandle,
    path: String,
    anchor: Anchor,
) -> Result<(), CommandError> {
    crate::require_local_main(&webview)?;
    let path = lms_files::stored_file(&app, &path)?;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let done = platform::share(&webview, path, anchor).await;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let done: Result<(), String> = Err(format!("no share UI for {path:?} at {anchor:?}"));
    done.map_err(|detail| {
        log::warn!("[file-share] share failed: {detail}");
        "share.openFailed".into()
    })
}

/// Runs `work` on the main thread, where AppKit and the Windows clipboard
/// and share UI must be driven, and waits for its result.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) async fn on_main<T: Send + 'static>(
    webview: &Webview,
    work: impl FnOnce(PlatformWebview) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    webview
        .with_webview(move |view| {
            let _ = send.send(work(view));
        })
        .map_err(|error| error.to_string())?;
    receive
        .await
        .map_err(|_| "the main thread dropped the request".to_string())?
}
