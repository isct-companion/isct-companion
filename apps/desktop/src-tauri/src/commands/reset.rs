//! Settings → reset. Core decides what a scope removes; this file lends it
//! the desktop course folder and the school WebView cookie store.
use crate::{cloud_sync, core_ports, require_local_main};
use isct_core::{
    net::BoxFuture,
    reset::ResetDevice,
    sync_engine::{ResetReport, ResetScope},
};
use std::{path::PathBuf, sync::Arc};
use tauri::{
    webview::WebviewBuilder, AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview,
    WebviewUrl,
};

/// A hidden view in the school data store, made only when no school view
/// exists yet in this process: cookies are reached through a view.
const SIGN_OUT_LABEL: &str = "school-sign-out";
/// The resident school views and the course-file view all share the school
/// data store (`school_web::school_profile`).
const SCHOOL_VIEW_PREFIX: &str = "school-web-";
const FILES_VIEW: &str = "lms-files";

struct DesktopReset(AppHandle);

impl ResetDevice for DesktopReset {
    fn course_files_root(&self) -> Option<PathBuf> {
        crate::lms_files::root(&self.0).ok().map(|(root, _)| root)
    }

    fn can_sign_out_school_web(&self) -> bool {
        true
    }

    fn sign_out_school_web(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move { sign_out(&self.0) })
    }
}

fn device(app: &AppHandle) -> Option<Arc<dyn ResetDevice>> {
    Some(Arc::new(DesktopReset(app.clone())))
}

/// Deletes the school store's cookies. Called from an async command's
/// worker, never the main thread, which WebView2 cookie calls require.
fn sign_out(app: &AppHandle) -> Result<(), String> {
    let existing = app
        .webviews()
        .into_iter()
        .find(|(label, _)| label == FILES_VIEW || label.starts_with(SCHOOL_VIEW_PREFIX))
        .map(|(_, view)| view);
    let (view, temporary) = match existing {
        Some(view) => (view, false),
        None => (store_view(app)?, true),
    };
    let result = view
        .cookies()
        .map_err(|error| format!("School cookies are unreadable: {error}"))
        .and_then(|cookies| {
            for cookie in cookies
                .into_iter()
                .filter(|cookie| removable(cookie.domain()))
            {
                view.delete_cookie(cookie)
                    .map_err(|error| format!("A school cookie could not be removed: {error}"))?;
            }
            Ok(())
        });
    if temporary {
        let _ = view.close();
    }
    result
}

fn store_view(app: &AppHandle) -> Result<Webview, String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let blank = Url::parse("about:blank").map_err(|error| error.to_string())?;
    let builder = crate::school_web::school_profile(
        app,
        WebviewBuilder::new(SIGN_OUT_LABEL, WebviewUrl::External(blank)),
    )?;
    let view = main
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| format!("School data store is unavailable: {error}"))?;
    view.hide().map_err(|error| error.to_string())?;
    Ok(view)
}

/// macOS and Windows give the school views their own data store, so every
/// cookie in it is a school session.
#[cfg(not(target_os = "linux"))]
fn removable(_domain: Option<&str>) -> bool {
    true
}

/// WebKitGTK views share the app's default store; only school sessions go.
#[cfg(target_os = "linux")]
fn removable(domain: Option<&str>) -> bool {
    const SCHOOL: [&str; 3] = ["isct.ac.jp", "titech.ac.jp", "ex-tic.com"];
    domain
        .map(|domain| domain.trim_start_matches('.'))
        .is_some_and(|domain| {
            SCHOOL
                .iter()
                .any(|school| domain == *school || domain.ends_with(&format!(".{school}")))
        })
}

/// Clears what `scope` names and returns what went. A full reset first
/// stops cloud sync and keeps a local recovery point; no cloud copy changes.
#[tauri::command]
pub(crate) async fn reset_caches(
    webview: tauri::Webview,
    app: AppHandle,
    scope: ResetScope,
) -> Result<ResetReport, String> {
    require_local_main(&webview)?;
    if scope == ResetScope::Everything {
        app.state::<cloud_sync::CloudSync>()
            .prepare_reset(&app)
            .await?;
    }
    core_ports::core(&app)
        .reset_with(scope, device(&app))
        .await
        .map_err(|error| error.to_string())
}

/// What `reset_caches(scope)` would remove now, for the confirmation panel.
#[tauri::command]
pub(crate) async fn reset_caches_preview(
    webview: tauri::Webview,
    app: AppHandle,
    scope: ResetScope,
) -> Result<ResetReport, String> {
    require_local_main(&webview)?;
    core_ports::core(&app)
        .reset_preview(scope, device(&app))
        .await
        .map_err(|error| error.to_string())
}
