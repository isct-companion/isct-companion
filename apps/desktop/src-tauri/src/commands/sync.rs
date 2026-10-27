//! The shell's whole share of school sync: forward hints, read the snapshot.
//! Core's planner decides what is due; `sync-snapshot` events carry changes.
use crate::CommandError;
use crate::{core_ports, require_local_main};
use app_core::sync_engine::{Hint, SyncSnapshot};

/// A lifecycle fact, an opened page or a refresh button. Never waits for a
/// read, so it stays synchronous on the IPC thread.
#[tauri::command]
pub(crate) fn sync_hint(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    hint: Hint,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    core_ports::core(&app).sync_hint(hint);
    Ok(())
}

/// The current snapshot, for start and for a missed `sync-snapshot` event.
#[tauri::command]
pub(crate) async fn sync_snapshot(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<SyncSnapshot, CommandError> {
    require_local_main(&webview)?;
    Ok(core_ports::core(&app).sync_snapshot().await)
}
