use crate::{
    coded, commands::cloud::SystemBrowser, require_local_main, secrets::OsSecretVault, CommandError,
};
use app_core::{
    exam_share::{ExamAction, ExamAnswer, ExamError, ExamShare},
    oauth::{code_exchange::OAuthClient, NativeBrowserAuth},
};
use std::{path::PathBuf, sync::Arc};
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

#[tauri::command]
pub(crate) async fn exam_share(
    webview: Webview,
    service: tauri::State<'_, Option<ExamShare>>,
    action: ExamAction,
) -> Result<ExamAnswer, CommandError> {
    require_local_main(&webview)?;
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
