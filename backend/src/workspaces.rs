//! Native assignment-workspace commands. Core owns storage and archive rules;
//! this adapter supplies the selected root and desktop openers.

use crate::lms_files::root;
use isct_core::{
    course_materials::CourseKey,
    workspaces::{Attachment, SubmissionTarget, WorkspaceInfo, Workspaces},
};
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

fn service(app: &AppHandle) -> Result<Workspaces, String> {
    Ok(Workspaces::new(root(app)?.0))
}

async fn run_workspace<T>(
    app: &AppHandle,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    crate::core_ports::runtime(app)
        .run(async move {
            tauri::async_runtime::spawn_blocking(operation)
                .await
                .map_err(|error| error.to_string())?
        })
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn assignment_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    context: CourseKey,
    assignment: String,
) -> Result<Option<WorkspaceInfo>, String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    run_workspace(&app, move || {
        service
            .find(&context, &assignment)
            .map_err(|error| error.to_string())
    })
    .await
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
) -> Result<WorkspaceInfo, String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    run_workspace(&app, move || {
        service
            .create(&context, assignment, title, brief, attachments)
            .map_err(|error| error.to_string())
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
) -> Result<(), String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    let dir = run_workspace(&app, move || {
        service
            .workspace_path(&path)
            .map_err(|error| error.to_string())
    })
    .await?;
    match target.as_str() {
        "finder" => app.opener().open_path(dir.to_string_lossy(), None::<&str>),
        "vscode" => {
            let url = url::Url::from_directory_path(&dir).map_err(|_| "Invalid workspace path")?;
            let url = format!("vscode://file{}", url.path().trim_end_matches('/'));
            app.opener().open_url(url, None::<&str>)
        }
        _ => return Err("Unknown target".into()),
    }
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn set_workspace_submission(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
    zip: bool,
    zip_name: String,
) -> Result<WorkspaceInfo, String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    run_workspace(&app, move || {
        service
            .set_submission(&path, zip, &zip_name)
            .map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn prepare_workspace_submission(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
) -> Result<WorkspaceInfo, String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    let prepared = run_workspace(&app, move || {
        service.prepare(&path).map_err(|error| error.to_string())
    })
    .await?;
    match prepared.target {
        SubmissionTarget::Folder(path) => {
            app.opener().open_path(path.to_string_lossy(), None::<&str>)
        }
        SubmissionTarget::Archive(path) => app.opener().reveal_item_in_dir(path),
    }
    .map_err(|error| error.to_string())?;
    Ok(prepared.info)
}

#[tauri::command]
pub async fn delete_workspace(
    webview: tauri::Webview,
    app: AppHandle,
    path: String,
) -> Result<(), String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    run_workspace(&app, move || {
        service.delete(&path).map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn clean_workspaces(webview: tauri::Webview, app: AppHandle) -> Result<usize, String> {
    crate::require_local_main(&webview)?;
    let service = service(&app)?;
    run_workspace(&app, move || {
        service.clean().map_err(|error| error.to_string())
    })
    .await
}
