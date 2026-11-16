use crate::{
    coded, commands::cloud::SystemBrowser, require_local_main, secrets::OsSecretVault, CommandError,
};
use app_core::{
    exam_share::{ExamAction, ExamAnswer, ExamError, ExamShare},
    oauth::{code_exchange::OAuthClient, NativeBrowserAuth},
};
use serde::Serialize;
use std::{collections::HashSet, path::PathBuf, sync::Arc, sync::Mutex};
use tauri::{Manager, Webview};
use tokio_util::sync::CancellationToken;

pub(crate) fn service(
    app: tauri::AppHandle,
    vault: OsSecretVault,
    folder: PathBuf,
) -> Option<ExamShare> {
    let version = app.package_info().version.to_string();
    let browser = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID").map(|id| {
        let client = OAuthClient {
            id: id.to_owned(),
            secret: option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_SECRET").map(str::to_owned),
        };
        let browser: Arc<dyn NativeBrowserAuth> = Arc::new(SystemBrowser(app));
        (client, browser)
    });
    ExamShare::new(Arc::new(vault), folder, version, browser).ok()
}

/// Files the person picked for an upload. Core reads only these, so the
/// page cannot name any other file on the disk.
#[derive(Default)]
pub(crate) struct PickedUploads(Mutex<HashSet<PathBuf>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickedFile {
    path: String,
    name: String,
    size: u64,
}

/// The types ScienceTokyoExam stores.
const UPLOAD_TYPES: &[&str] = &["pdf", "jpg", "jpeg", "png", "webp", "txt"];

#[tauri::command]
pub(crate) async fn exam_pick_upload(
    webview: Webview,
    picked: tauri::State<'_, PickedUploads>,
) -> Result<Vec<PickedFile>, CommandError> {
    require_local_main(&webview)?;
    let paths = crate::native_dialog::pick_files(
        &webview.window(),
        "Past exam to share",
        "PDF, photos or text",
        UPLOAD_TYPES,
    )
    .await?;
    let mut allowed = picked.0.lock().map_err(crate::shell)?;
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let size = std::fs::metadata(&path).ok()?.len();
            let name = path.file_name()?.to_str()?.to_owned();
            let shown = path.to_str()?.to_owned();
            allowed.insert(path);
            Some(PickedFile {
                path: shown,
                name,
                size,
            })
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn exam_share(
    webview: Webview,
    service: tauri::State<'_, Option<ExamShare>>,
    picked: tauri::State<'_, PickedUploads>,
    action: ExamAction,
) -> Result<ExamAnswer, CommandError> {
    require_local_main(&webview)?;
    if let ExamAction::Upload { draft } = &action {
        let allowed = picked.0.lock().map_err(crate::shell)?;
        if !draft
            .files
            .iter()
            .all(|file| allowed.contains(&PathBuf::from(file)))
        {
            return Err(coded(ExamError::InvalidInput));
        }
    }
    let service = service
        .inner()
        .as_ref()
        .ok_or_else(|| coded(ExamError::Unavailable))?;
    let cancel = CancellationToken::new();
    let _guard = cancel.clone().drop_guard();
    let answer = service.act(action, cancel).await.map_err(coded)?;
    if let ExamAnswer::Document(file) = &answer {
        webview
            .app_handle()
            .asset_protocol_scope()
            .allow_file(&file.path)
            .map_err(|_| coded(ExamError::Storage))?;
    }
    Ok(answer)
}
