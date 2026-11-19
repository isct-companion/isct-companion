//! System actions for Core's past-exam files, on top of the reader's
//! asset-protocol grant.
use crate::{coded, downloads, CommandError};
use app_core::{exam_share::ExamError, storage};
use std::{collections::HashSet, path::PathBuf, sync::Mutex};
use tauri::{Manager, Webview};

/// Native file grants: the asset protocol only lets the reader load a file,
/// these let the shell open, retain or export it.
#[derive(Default)]
pub(crate) struct ExamFiles(Mutex<HashSet<PathBuf>>);

impl ExamFiles {
    pub(crate) fn allow(&self, path: &str) -> Result<(), CommandError> {
        self.0
            .lock()
            .map_err(|_| coded(ExamError::Storage))?
            .insert(PathBuf::from(path));
        Ok(())
    }
}

fn source(webview: &Webview, path: &str) -> Result<PathBuf, CommandError> {
    crate::require_local_main(webview)?;
    let files = webview.app_handle().state::<ExamFiles>();
    if !files
        .0
        .lock()
        .map_err(|_| coded(ExamError::Storage))?
        .contains(&PathBuf::from(path))
    {
        return Err(coded(ExamError::InvalidInput));
    }
    let core = crate::core_ports::core(webview.app_handle());
    storage::stored_file(&core.store.paths().root().join("exam-share"), path).map_err(coded)
}

#[tauri::command]
pub(crate) fn exam_open_file(webview: Webview, path: String) -> Result<(), CommandError> {
    let path = source(&webview, &path)?;
    downloads::show_file(webview.app_handle(), &path, false)
}

#[tauri::command]
pub(crate) async fn exam_retain_file(
    webview: Webview,
    id: String,
    path: String,
) -> Result<bool, CommandError> {
    source(&webview, &path)?;
    let core = crate::core_ports::core(webview.app_handle());
    core.retain_open_exam(&id, std::path::Path::new(&path))
        .await
        .map_err(coded)
}

/// Core names the copy from the document's validated type, not the webview.
#[tauri::command]
pub(crate) async fn exam_save_copy(
    webview: Webview,
    id: String,
    path: String,
) -> Result<downloads::SavedDownload, CommandError> {
    let source = source(&webview, &path)?;
    let core = crate::core_ports::core(webview.app_handle());
    let name = core
        .open_exam_export_name(&id, std::path::Path::new(&path))
        .await
        .map_err(coded)?;
    downloads::save_copy(webview.app_handle().clone(), source, name).await
}
