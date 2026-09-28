use serde::Serialize;
use tauri::{plugin::PermissionState, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use cloud::{CloudProvider, CloudRequestInput, CloudResponse, DesktopCloudService};
use lifecycle::SchoolSite;
use school_auth::{
    SchoolAuthProfile, SchoolAuthService, SchoolCredentialStatus, SchoolCredentials,
};
use school_extract::sanitize_school_url;
use school_web::{SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds};
use secrets::OsSecretVault;

mod matrix_red_crops;
mod matrix_ocr;
mod cloud;
mod lifecycle;
mod policy;
mod school_auth;
mod school_extract;
mod school_web;
mod secrets;
mod syllabus;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeCapabilities {
    school_import: bool,
    school_web: bool,
    notifications: bool,
    background_schedule: bool,
    cloud_sync: bool,
    syllabus_search: bool,
    syllabus_filters: bool,
}

fn require_local_main(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("native commands are restricted to the local main webview".into());
    }
    let url = webview
        .url()
        .map_err(|_| "native caller URL is unavailable")?;
    let packaged = (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"));
    #[cfg(debug_assertions)]
    let trusted = packaged
        || (url.scheme() == "http"
            && url.host_str() == Some("localhost")
            && url.port() == Some(4173));
    #[cfg(not(debug_assertions))]
    let trusted = packaged;
    if !trusted {
        return Err("native command caller is not the bundled application origin".into());
    }
    Ok(())
}

#[tauri::command]
async fn recognize_matrix_image(
    webview: tauri::Webview,
    service: tauri::State<'_, matrix_ocr::MatrixOcrService>,
    data: String,
    preview: String,
    on_progress: tauri::ipc::Channel<usize>,
) -> Result<Vec<matrix_ocr::Recognition>, String> {
    require_local_main(&webview)?;
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.scan(&data, &preview, |completed| { let _ = on_progress.send(completed); })
    }).await.map_err(|e| format!("Matrix recognition task failed: {e}"))?
}

#[tauri::command]
fn runtime_capabilities(webview: tauri::Webview) -> Result<RuntimeCapabilities, String> {
    require_local_main(&webview)?;
    Ok(RuntimeCapabilities {
        school_import: false,
        school_web: true,
        notifications: true,
        background_schedule: false,
        cloud_sync: true,
        syllabus_search: true,
        syllabus_filters: true,
    })
}

#[tauri::command]
fn open_external(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    url: String,
) -> Result<(), String> {
    require_local_main(&webview)?;
    let safe = sanitize_school_url(&url)
        .ok_or_else(|| "external URL is outside the allowed school origins".to_string())?;
    app.opener()
        .open_url(safe, None::<&str>)
        .map_err(|_| "external URL could not be opened".to_string())
}

#[tauri::command]
fn configure_school_web_policy(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    policy: SchoolWebPolicy,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.configure(&app, policy)
}

#[tauri::command]
async fn open_school_site(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    css: String,
    js: String,
    bounds: Option<WebBounds>,
    target_url: Option<String>,
) -> Result<SchoolWebResult, String> {
    require_local_main(&webview)?;
    if css.len() > 256 * 1024 || js.len() > 256 * 1024 {
        return Err("school customization exceeds the safe size".into());
    }
    controller.open(&app, site, css, js, bounds, target_url)
}

#[tauri::command]
fn set_school_web_bounds(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    bounds: WebBounds,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.set_bounds(&app, bounds)
}

#[tauri::command]
fn hide_school_web(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
) -> Result<bool, String> {
    if let Err(error) = require_local_main(&webview) {
        eprintln!("hide_school_web rejected for {}: {error}", webview.label());
        return Err(error);
    }
    controller.hide_all(&app)
}

#[tauri::command]
fn reload_school_web(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.reload(&app, site)
}

#[tauri::command]
fn navigate_school_web(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    url: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.navigate(&app, site, &url)
}

#[tauri::command]
fn set_school_web_zoom(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    scale: f64,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.set_zoom(site, scale)
}

#[tauri::command]
fn school_credential_status(
    webview: tauri::Webview,
    service: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
) -> Result<SchoolCredentialStatus, String> {
    require_local_main(&webview)?;
    service.status()
}

#[tauri::command]
fn save_school_credentials(
    webview: tauri::Webview,
    service: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.save(profile, credentials)
}

#[tauri::command]
fn delete_school_credentials(
    webview: tauri::Webview,
    service: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
    profile: SchoolAuthProfile,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.delete(profile)
}

#[tauri::command]
async fn login_school(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
    bounds: Option<WebBounds>,
) -> Result<SchoolWebResult, String> {
    require_local_main(&webview)?;
    let credentials = auth
        .load(profile)?
        .ok_or_else(|| "Save this school account before signing in.".to_string())?;
    let _ = bounds;
    controller.login_in_background(&app, profile, credentials)
}

#[tauri::command]
async fn ensure_school_session(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.ensure_session(&app, profile)
}

#[tauri::command]
async fn test_school_connection(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
) -> Result<SchoolWebResult, String> {
    require_local_main(&webview)?;
    let credentials = auth.resolve(profile, credentials)?;
    controller.login_in_background(&app, profile, credentials)
}

#[tauri::command]
fn notify(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    title: String,
    body: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    if title.len() > 160 || body.len() > 2_000 {
        return Err("notification content is too large".into());
    }
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|_| "notification could not be shown".to_string())?;
    Ok(true)
}

#[tauri::command]
fn request_notification_permission(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<&'static str, String> {
    require_local_main(&webview)?;
    match app
        .notification()
        .request_permission()
        .map_err(|_| "notification permission could not be requested".to_string())?
    {
        PermissionState::Granted => Ok("granted"),
        PermissionState::Denied => Ok("denied"),
        PermissionState::Prompt | PermissionState::PromptWithRationale => Ok("default"),
    }
}

#[tauri::command]
async fn connect_cloud(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
    client_id: String,
    redirect_uri: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service
        .connect(provider, &client_id, &redirect_uri, move |url| {
            app.opener()
                .open_url(url, None::<&str>)
                .map_err(|_| "system browser could not be opened".to_string())
        })
        .await
}

#[tauri::command]
async fn cloud_request(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
    url: String,
    init: CloudRequestInput,
) -> Result<CloudResponse, String> {
    require_local_main(&webview)?;
    service.request(provider, &url, init).await
}

#[tauri::command]
async fn search_syllabus(
    webview: tauri::Webview,
    query: String,
    year: u16,
    limit: usize,
    filters: Option<syllabus::SyllabusFilters>,
    language: Option<String>,
) -> Result<syllabus::SyllabusSearch, String> {
    require_local_main(&webview)?;
    syllabus::search(&query, year, limit, filters, language.as_deref()).await
}

#[tauri::command]
async fn syllabus_course(
    webview: tauri::Webview,
    url: String,
) -> Result<syllabus::SyllabusCourse, String> {
    require_local_main(&webview)?;
    syllabus::detail(&url).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_secure_storage::init())
        .manage(SchoolWebController::new())
        .manage(matrix_ocr::MatrixOcrService::default())
        .invoke_handler(tauri::generate_handler![
            runtime_capabilities,
            recognize_matrix_image,
            open_external,
            configure_school_web_policy,
            open_school_site,
            set_school_web_bounds,
            hide_school_web,
            reload_school_web,
            navigate_school_web,
            set_school_web_zoom,
            school_credential_status,
            save_school_credentials,
            delete_school_credentials,
            login_school,
            test_school_connection,
            ensure_school_session,
            request_notification_permission,
            notify,
            connect_cloud,
            cloud_request
            ,search_syllabus,
            syllabus_course
        ])
        .setup(|app| {
            let vault = OsSecretVault::new(app.handle().clone());
            app.manage(
                DesktopCloudService::new(vault.clone())
                    .expect("failed to initialize the cloud transport"),
            );
            app.manage(SchoolAuthService::new(vault));

            #[cfg(desktop)]
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Science Tokyo Workstation")
                .inner_size(1280.0, 820.0)
                .min_inner_size(860.0, 620.0)
                .initialization_script(
                    "Object.defineProperty(window,'__STWS_NATIVE_TARGET__',{value:'native-desktop',writable:false,configurable:false});",
                )
                .build()?;

            #[cfg(mobile)]
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(
                    "Object.defineProperty(window,'__STWS_NATIVE_TARGET__',{value:'native-mobile',writable:false,configurable:false});",
                )
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run Science Tokyo Workstation");
}
