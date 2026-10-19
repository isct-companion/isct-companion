//! Native assignment-workspace commands. Core owns storage and archive rules;
//! this adapter supplies the selected root and desktop openers.

use crate::lms_files::root;
use isct_core::{
    course_materials::CourseKey,
    school_browser::{SubmissionError, SubmissionMessages},
    workspaces::{Attachment, HandInView, WorkspaceError, WorkspaceInfo, Workspaces},
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

fn service(webview: &tauri::Webview, app: &AppHandle) -> Result<Workspaces, WorkspaceError> {
    crate::require_local_main(webview).map_err(WorkspaceError::Input)?;
    Ok(Workspaces::new(
        root(app).map_err(WorkspaceError::Storage)?.0,
    ))
}

async fn run_workspace<T>(
    app: &AppHandle,
    operation: impl FnOnce() -> Result<T, WorkspaceError> + Send + 'static,
) -> Result<T, WorkspaceError>
where
    T: Send + 'static,
{
    crate::core_ports::runtime(app)
        .run(async move {
            tauri::async_runtime::spawn_blocking(operation)
                .await
                .map_err(|error| WorkspaceError::Storage(error.to_string()))?
        })
        .await
        .map_err(|error| WorkspaceError::Storage(error.to_string()))?
}

/// `closed`: whether `TaskView.closedIds` lists the assignment.
#[tauri::command]
pub async fn assignment_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseKey,
    assignment: String,
    closed: bool,
) -> Result<HandInView, WorkspaceError> {
    let service = service(&webview, &app)?;
    run_workspace(&app, move || service.hand_in(&context, &assignment, closed)).await
}

#[tauri::command]
pub async fn create_assignment_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseKey,
    assignment: String,
    title: String,
    brief: String,
    attachments: Vec<Attachment>,
) -> Result<WorkspaceInfo, WorkspaceError> {
    let service = service(&webview, &app)?;
    run_workspace(&app, move || {
        service.create(&context, assignment, title, brief, attachments)
    })
    .await
}

/// `finder`: the workspace in the file manager; `vscode`: the whole folder.
#[tauri::command]
pub async fn open_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    target: String,
) -> Result<(), WorkspaceError> {
    let service = service(&webview, &app)?;
    let dir = run_workspace(&app, move || service.workspace_path(&path)).await?;
    match target.as_str() {
        "finder" => app.opener().open_path(dir.to_string_lossy(), None::<&str>),
        "vscode" => {
            let url = url::Url::from_directory_path(&dir)
                .map_err(|_| WorkspaceError::Input("Invalid workspace path".into()))?;
            let url = format!("vscode://file{}", url.path().trim_end_matches('/'));
            app.opener().open_url(url, None::<&str>)
        }
        _ => return Err(WorkspaceError::Input("Unknown target".into())),
    }
    .map_err(|error| WorkspaceError::Storage(error.to_string()))
}

#[tauri::command]
pub async fn set_workspace_submission(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    zip: bool,
    zip_name: String,
) -> Result<WorkspaceInfo, WorkspaceError> {
    let service = service(&webview, &app)?;
    run_workspace(&app, move || service.set_submission(&path, zip, &zip_name)).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionReady {
    workspace: WorkspaceInfo,
    url: String,
}

#[tauri::command]
#[deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub async fn prepare_workspace_submission(
    webview: tauri::Webview,
    app: AppHandle,
    controller: tauri::State<'_, crate::school_web::SchoolWebController>,
    path: String,
    assignment: String,
    url: String,
    messages: SubmissionMessages,
) -> Result<SubmissionReady, SubmissionError> {
    let service = service(&webview, &app)?;
    let prepared = crate::core_ports::runtime(&app)
        .run(async move {
            tauri::async_runtime::spawn_blocking(move || service.prepare_upload(&path, &assignment))
                .await
                .map_err(|error| SubmissionError::Storage(error.to_string()))?
                .map_err(SubmissionError::from)
        })
        .await
        .map_err(|error| SubmissionError::Browser(error.to_string()))??;
    let url = controller
        .engine
        .prepare_submission(&url, prepared.files, messages)?;
    Ok(SubmissionReady {
        workspace: prepared.info,
        url,
    })
}

#[tauri::command]
pub async fn delete_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
) -> Result<(), WorkspaceError> {
    let service = service(&webview, &app)?;
    run_workspace(&app, move || service.delete(&path)).await
}

#[tauri::command]
pub async fn clean_workspaces(
    webview: tauri::Webview,
    app: AppHandle,
) -> Result<usize, WorkspaceError> {
    let service = service(&webview, &app)?;
    run_workspace(&app, move || service.clean()).await
}
