//! Settings → reset. Core decides what a scope removes; this file lends it
//! the desktop course folder. Core clears the school views' cookies through
//! its web cookie port (`school_cookies`).
use crate::{cloud_sync, core_ports, require_local_main};
use crate::{coded, CommandError};
use app_core::{
    reset::ResetDevice,
    plan::{ResetReport, ResetScope},
};
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Manager};

struct DesktopReset(AppHandle);

impl ResetDevice for DesktopReset {
    fn course_files_root(&self) -> Option<PathBuf> {
        crate::lms_files::root(&self.0).ok().map(|(root, _)| root)
    }
}

fn device(app: &AppHandle) -> Option<Arc<dyn ResetDevice>> {
    Some(Arc::new(DesktopReset(app.clone())))
}

/// Clears what `scope` names and returns what went. A full reset first
/// stops cloud sync; no cloud copy changes.
#[tauri::command]
pub(crate) async fn reset_caches(
    webview: tauri::Webview,
    app: AppHandle,
    scope: ResetScope,
) -> Result<ResetReport, CommandError> {
    require_local_main(&webview)?;
    if scope == ResetScope::Everything {
        app.state::<cloud_sync::CloudSync>()
            .prepare_reset(&app)
            .await?;
    }
    core_ports::core(&app)
        .reset_with(scope, device(&app))
        .await
        .map_err(coded)
}

/// What `reset_caches(scope)` would remove now, for the confirmation panel.
#[tauri::command]
pub(crate) async fn reset_caches_preview(
    webview: tauri::Webview,
    app: AppHandle,
    scope: ResetScope,
) -> Result<ResetReport, CommandError> {
    require_local_main(&webview)?;
    core_ports::core(&app)
        .reset_preview(scope, device(&app))
        .await
        .map_err(coded)
}
