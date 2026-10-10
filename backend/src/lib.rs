#![recursion_limit = "256"]

use serde::Serialize;
use tauri::{plugin::PermissionState, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use cloud::{CloudProvider, CloudSyncResult, DesktopCloudService};
use lifecycle::SchoolSite;
use school_auth::{
    SchoolAuthProfile, SchoolAuthService, SchoolCredentialStatus, SchoolCredentials,
};
use school_extract::sanitize_school_url;
use school_web::{SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds};
use secrets::OsSecretVault;

mod cloud;
pub mod contracts;
pub mod data;
mod kyomu_guide;
mod kyomu_ingest;
mod lifecycle;
#[cfg(desktop)]
mod lms_downloader;
mod lms_files;
mod lms_ingest;
mod lms_protocol;
#[cfg(matrix_ocr)]
mod matrix_ocr;
#[cfg(matrix_ocr)]
mod matrix_red_crops;
pub mod plans;
mod policy;
mod scheduler;
mod school_auth;
mod school_extract;
mod school_parser;
#[cfg(not(mobile))]
mod school_web;
#[cfg(mobile)]
#[path = "school_web_mobile.rs"]
mod school_web;
mod secrets;
mod subscriptions;
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
    matrix_ocr: bool,
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

/// SQLite-backed commands run on a blocking thread so a slow query never stalls
/// WebView frames on the main thread. They stay serialized with each other, as
/// they were when Tauri ran them on the main thread.
async fn run_data_command<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    static DATA_COMMANDS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    tauri::async_runtime::spawn_blocking(move || {
        let _serialized = DATA_COMMANDS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        work()
    })
    .await
    .map_err(|_| "The native data command stopped unexpectedly.".to_string())?
}

#[cfg(matrix_ocr)]
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
        service.scan(&data, &preview, |completed| {
            let _ = on_progress.send(completed);
        })
    })
    .await
    .map_err(|e| format!("Matrix recognition task failed: {e}"))?
}

/// Intel macOS has no prebuilt ONNX Runtime; manual matrix entry still works.
#[cfg(not(matrix_ocr))]
#[tauri::command]
fn recognize_matrix_image(webview: tauri::Webview) -> Result<(), String> {
    require_local_main(&webview)?;
    Err(
        "Scan image is not available on Intel Macs yet. Type or paste the 70 letters instead."
            .into(),
    )
}

#[cfg(test)]
mod external_url_tests {
    use super::external_url;

    #[test]
    fn opens_school_and_plain_web_links_only() {
        assert_eq!(
            external_url("https://syllabus.s.isct.ac.jp/courses?jwc=202602431").as_deref(),
            Some("https://syllabus.s.isct.ac.jp/courses?jwc=202602431")
        );
        assert!(external_url("https://example.org/paper?id=1").is_some());
        assert!(external_url("file:///etc/passwd").is_none());
        assert!(external_url("javascript:alert(1)").is_none());
        assert!(external_url("https://user:pw@example.org/").is_none());
    }
}

#[tauri::command]
fn runtime_capabilities(webview: tauri::Webview) -> Result<RuntimeCapabilities, String> {
    require_local_main(&webview)?;
    Ok(RuntimeCapabilities {
        school_import: cfg!(not(mobile)),
        school_web: cfg!(not(mobile)),
        notifications: true,
        background_schedule: cfg!(not(mobile)),
        cloud_sync: true,
        syllabus_search: true,
        matrix_ocr: cfg!(matrix_ocr),
    })
}

/// School pages keep their sanitized form; any other link must be plain
/// http(s) without embedded credentials.
fn external_url(raw: &str) -> Option<String> {
    if let Some(safe) = sanitize_school_url(raw) {
        return Some(safe);
    }
    let url = url::Url::parse(raw).ok()?;
    let plain = matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none();
    plain.then(|| url.to_string())
}

#[tauri::command]
fn resolve_school_url(webview: tauri::Webview, url: String) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let safe = sanitize_school_url(&url)
        .ok_or_else(|| "Use an official Science Tokyo school URL.".to_string())?;
    let site = SchoolSite::for_url(&safe)
        .ok_or_else(|| "Use an official Science Tokyo school URL.".to_string())?;
    Ok(serde_json::json!({"site": site, "url": safe}))
}

#[tauri::command]
fn open_external(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    url: String,
) -> Result<(), String> {
    require_local_main(&webview)?;
    let safe = external_url(&url).ok_or_else(|| "This link cannot be opened.".to_string())?;
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

/// The app's own Back for the school view: reloads the previous page as a
/// GET instead of `history.back()` (which would resubmit postbacks).
#[tauri::command]
fn school_web_back(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.back(&app, site)
}

/// Shows a file a school page saved this session in Finder; other paths are
/// refused.
#[tauri::command]
fn reveal_school_download(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    path: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.reveal_download(&app, &path)
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

/// Runs the LMS or 教務Web extractor in the hidden school view and waits for
/// its result (one per 教務Web page).
#[tauri::command]
async fn pull_school(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    site: SchoolSite,
    lms_course_id: Option<u64>,
) -> Result<Vec<school_web::IngestOutcome>, String> {
    require_local_main(&webview)?;
    let controller = app.state::<SchoolWebController>().inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        controller.request_pull(&handle, site, lms_course_id)
    })
    .await
    .map_err(|_| "The school pull stopped unexpectedly.".to_string())?
}

/// School-owned records the workspace does not carry (LMS courses,
/// registrations, grades, GPA by term, requirements).
#[tauri::command]
async fn school_records(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || school_records_inner(&store)).await
}

fn school_records_inner(store: &data::DataStore) -> Result<serde_json::Value, String> {
    let mut records = serde_json::Map::new();
    for (key, table) in [
        ("lmsCourses", "lms_courses"),
        ("registrations", "registrations"),
        ("registrationStates", "registration_states"),
        ("grades", "grades"),
        ("gpaByTerm", "gpa_by_term"),
        ("requirements", "requirement_progress"),
    ] {
        let rows = store
            .table_rows(table)?
            .into_iter()
            .filter(|row| {
                row.get("deletedAt")
                    .map_or(true, serde_json::Value::is_null)
            })
            .collect();
        records.insert(key.into(), serde_json::Value::Array(rows));
    }
    records.insert("lastRuns".into(), store.last_import_runs()?);
    Ok(serde_json::Value::Object(records))
}

#[tauri::command]
fn set_kyomu_guide(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    items: Vec<kyomu_guide::GuideItem>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.set_guide(items)
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

/// Copies the saved Matrix directly from the vault to the system clipboard,
/// so the plaintext never passes through the WebView.
#[tauri::command]
fn copy_school_matrix(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let text = auth.matrix_text()?;
    app.clipboard()
        .write_text(text)
        .map_err(|_| "Matrix could not be copied to the clipboard.".to_string())?;
    Ok(true)
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
    let controller = controller.inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        controller.request_login(&handle, profile, credentials)
    })
    .await
    .map_err(|_| "The queued school sign-in stopped unexpectedly.".to_string())?
}

#[tauri::command]
async fn ensure_school_session(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    profile: SchoolAuthProfile,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.request_session(&app, profile)
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
    let controller = controller.inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        controller.request_login(&handle, profile, credentials)
    })
    .await
    .map_err(|_| "The queued school sign-in stopped unexpectedly.".to_string())?
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
fn cloud_authorization_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<Vec<CloudProvider>, String> {
    require_local_main(&webview)?;
    service.authorized_providers()
}

#[tauri::command]
fn disconnect_cloud(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.disconnect(provider)?;
    Ok(true)
}

#[tauri::command]
async fn cloud_backup_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    auth: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
    store: tauri::State<'_, data::DataStore>,
    provider: CloudProvider,
    overwrite: bool,
) -> Result<CloudSyncResult, String> {
    require_local_main(&webview)?;
    let notes =
        serde_json::to_value(lms_files::export_notes(&app)?).map_err(|error| error.to_string())?;
    let document = store.export_data(notes)?;
    let workspace = store.load_workspace()?;
    let device_id = workspace
        .get("deviceId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Local device identifier is unavailable.".to_string())?
        .to_string();
    let credentials = if service.credential_sync_configured()? {
        Some(auth.export_sync_payload()?)
    } else {
        None
    };
    let result = service
        .back_up_workspace(provider, &document, &device_id, overwrite)
        .await?;
    if result.state != cloud::CloudSyncState::Conflict {
        if let Some(credentials) = credentials {
            service.back_up_credentials(provider, &credentials).await?;
        }
    }
    Ok(result)
}

#[tauri::command]
async fn cloud_restore_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    auth: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
    store: tauri::State<'_, data::DataStore>,
    provider: CloudProvider,
) -> Result<CloudSyncResult, String> {
    require_local_main(&webview)?;
    let (document, result) = service.pull_workspace(provider).await?;
    let credentials = service.pull_credentials(provider).await?;
    let parsed: serde_json::Value =
        serde_json::from_str(&document).map_err(|_| "cloud workspace is invalid JSON")?;
    let notes: Vec<lms_files::LocalNoteBackup> = serde_json::from_value(
        parsed
            .get("localNotes")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|_| "cloud workspace localNotes is invalid")?;
    lms_files::restore_notes(&app, &notes)?;
    store.import_data(&parsed)?;
    if let Some(credentials) = credentials {
        auth.import_sync_payload(&credentials)?;
    }
    Ok(result)
}

#[tauri::command]
fn cloud_credential_sync_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.credential_sync_configured()
}

#[tauri::command]
fn set_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    password: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.set_credential_sync_password(&password)?;
    Ok(true)
}

#[tauri::command]
fn clear_cloud_credential_sync_password(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.clear_credential_sync_password()?;
    Ok(true)
}

#[tauri::command]
async fn load_workspace(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || store.load_workspace()).await
}

#[tauri::command]
fn new_user_record_id(webview: tauri::Webview) -> Result<String, String> {
    require_local_main(&webview)?;
    Ok(uuid::Uuid::now_v7().to_string())
}

#[tauri::command]
async fn put_user_record(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    table: String,
    record: serde_json::Value,
    expected_rev: Option<u64>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        store.put_user_record_checked(&table, record, "desktop", expected_rev)
    })
    .await
}

#[tauri::command]
async fn delete_user_record(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    table: String,
    id: String,
    expected_rev: Option<u64>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        store.delete_user_record_checked(&table, &id, "desktop", expected_rev)
    })
    .await
}

#[tauri::command]
async fn run_scheduled_task(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    scheduler: tauri::State<'_, scheduler::Scheduler>,
    id: String,
) -> Result<String, String> {
    require_local_main(&webview)?;
    scheduler.run_now(&app, &id).await
}

#[tauri::command]
async fn put_workspace_setting(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    key: String,
    value: serde_json::Value,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        store.put_workspace_setting(&key, value, "desktop")?;
        store
            .load_workspace()?
            .get("currentTerm")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "current academic term is unavailable".to_string())
    })
    .await
}

#[tauri::command]
async fn reset_workspace(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || store.reset_workspace()).await
}

#[tauri::command]
async fn export_data(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        let notes = serde_json::to_value(lms_files::export_notes(&app)?)
            .map_err(|error| error.to_string())?;
        store.export_data(notes)
    })
    .await
}

/// Saves a file the person exported (calendar, backup) where they choose.
/// The main webview has no `on_download` handler, so WebKit cancels
/// `<a download>` clicks there. Resolves false when the dialog is cancelled.
#[tauri::command]
async fn save_file(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    name: String,
    contents: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    if name.trim().is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err("invalid file name".into());
    }
    if contents.len() > 64 * 1024 * 1024 {
        return Err("file exceeds the 64 MiB limit".into());
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_dialog::DialogExt;
        tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
            let mut dialog = app.dialog().file().set_file_name(&name);
            if let Some((_, extension)) = name.rsplit_once('.') {
                dialog = dialog.add_filter(extension.to_uppercase(), &[extension]);
            }
            let Some(chosen) = dialog.blocking_save_file() else {
                return Ok(false);
            };
            let path = chosen.into_path().map_err(|e| e.to_string())?;
            std::fs::write(path, contents).map_err(|e| e.to_string())?;
            Ok(true)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(mobile)]
    {
        let _ = (app, contents);
        Err("Saving files is available on desktop.".into())
    }
}

/// Writes the shared export document into app-owned storage. Scheduled tasks
/// must not trigger browser downloads or reuse an interactive cloud token.
#[tauri::command]
async fn write_scheduled_export(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || write_scheduled_export_inner(&app, &store)).await
}

fn write_scheduled_export_inner(
    app: &tauri::AppHandle,
    store: &data::DataStore,
) -> Result<String, String> {
    use std::io::Write;

    let notes =
        serde_json::to_value(lms_files::export_notes(app)?).map_err(|error| error.to_string())?;
    let document = store.export_data(notes)?;
    if document.len() > 32 * 1024 * 1024 {
        return Err("scheduled export exceeds the 32 MiB safety limit".into());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&document).map_err(|_| "scheduled export is invalid JSON")?;
    if parsed
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(6)
    {
        return Err("scheduled export schema is unsupported".into());
    }

    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("scheduled-exports");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let destination = directory.join("workspace-latest.json");
    let temporary = directory.join(format!(".workspace-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(document.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        match std::fs::rename(&temporary, &destination) {
            Ok(()) => Ok(()),
            Err(error)
                if destination.is_file()
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
            {
                std::fs::remove_file(&destination).map_err(|error| error.to_string())?;
                std::fs::rename(&temporary, &destination).map_err(|error| error.to_string())
            }
            Err(error) => Err(error.to_string()),
        }
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(destination.display().to_string())
}

#[tauri::command]
async fn import_data(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    document: String,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || import_data_inner(&app, &store, &document)).await
}

fn import_data_inner(
    app: &tauri::AppHandle,
    store: &data::DataStore,
    document: &str,
) -> Result<serde_json::Value, String> {
    let parsed = data::parse_import(document)?;
    let notes: Vec<lms_files::LocalNoteBackup> =
        serde_json::from_value(parsed["localNotes"].clone())
            .map_err(|error| format!("the backup's local notes are invalid: {error}"))?;
    // Validate and restore local user notes under the file-store lock. School
    // materials are never present in this document.
    lms_files::restore_notes(app, &notes)?;
    store.import_data(&parsed)
}

#[tauri::command]
async fn catalog_upsert_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    offerings: Vec<serde_json::Value>,
) -> Result<usize, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || store.catalog_upsert_offerings(&offerings)).await
}

/// Every stored offering of one academic year, with schedule, credits, language and
/// format, so the course finder can filter and suggest locally without a round trip.
#[tauri::command]
async fn catalog_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    year: u16,
) -> Result<Vec<serde_json::Value>, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        Ok(store
            .catalog_search("")?
            .into_iter()
            .filter(|row| {
                row.get("year").and_then(serde_json::Value::as_u64) == Some(u64::from(year))
                    && row
                        .get("deletedAt")
                        .map_or(true, serde_json::Value::is_null)
            })
            .collect())
    })
    .await
}

#[tauri::command]
async fn catalog_offering(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    id: String,
) -> Result<Option<serde_json::Value>, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || store.catalog_offering(&id)).await
}

#[tauri::command]
async fn catalog_sync_offering(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    id: String,
) -> Result<syllabus::SyncOfferingResult, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let offering_id = if id.len() == 9 && id.chars().all(|value| value.is_ascii_digit()) {
        Some(id.clone())
    } else {
        url::Url::parse(&id)
            .ok()
            .and_then(|url| url.path_segments()?.next_back().map(str::to_string))
            .filter(|value| value.len() == 9 && value.chars().all(|c| c.is_ascii_digit()))
    };
    if let Some(offering_id) = offering_id {
        if let Some(stored) = store.catalog_offering(&offering_id)? {
            let cutoff = chrono::Utc::now()
                - chrono::Duration::seconds(contracts::SYLLABUS_STALE_SECONDS as i64);
            let fresh = ["ja", "en"].iter().all(|language| {
                stored
                    .pointer(&format!("/fetchedAt/{language}"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|value| value.with_timezone(&chrono::Utc) >= cutoff)
            });
            if fresh {
                let offering = serde_json::from_value(stored).map_err(|error| error.to_string())?;
                let content = store
                    .table_rows("offering_content")?
                    .into_iter()
                    .filter(|row| {
                        row.get("offeringId").and_then(serde_json::Value::as_str)
                            == Some(offering_id.as_str())
                    })
                    .map(serde_json::from_value)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                if content.len() == 2 {
                    return Ok(syllabus::SyncOfferingResult {
                        offering,
                        content,
                        warnings: Vec::new(),
                    });
                }
            }
        }
    }
    let result = syllabus::sync_offering(&id).await?;
    let offering = serde_json::to_value(&result.offering).map_err(|error| error.to_string())?;
    let content = result
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    store.catalog_upsert_offerings(&[offering])?;
    store.upsert_offering_content(&content)?;
    Ok(result)
}

/// Search rows carry no schedule; they only fill gaps and never replace an
/// offering whose detail page was already stored.
fn store_search_items(
    store: &data::DataStore,
    items: &[syllabus::CatalogSearchItem],
) -> Result<(), String> {
    let mut rows = Vec::new();
    for item in items {
        if store.catalog_offering(&item.id)?.is_none() {
            rows.push(serde_json::to_value(item).map_err(|error| error.to_string())?);
        }
    }
    if !rows.is_empty() {
        store.catalog_upsert_search_items(&rows)?;
    }
    Ok(())
}

/// Fetches detail pages for offerings that have none or a stale one (older
/// than a week), storing each bounded batch as it arrives.
async fn sync_stale_details(
    store: &data::DataStore,
    targets: Vec<(String, String)>,
    summary_only: bool,
) -> Result<(Vec<syllabus::SyncOfferingResult>, Vec<String>), String> {
    let cutoff =
        chrono::Utc::now() - chrono::Duration::seconds(contracts::SYLLABUS_STALE_SECONDS as i64);
    let stale = targets
        .into_iter()
        .filter(|(id, _)| {
            let Ok(Some(stored)) = store.catalog_offering(id) else {
                return true;
            };
            stored.get("schedule").is_none()
                || (if summary_only {
                    &["en"][..]
                } else {
                    &["ja", "en"][..]
                })
                .iter()
                .any(|language| {
                    stored
                        .pointer(&format!("/fetchedAt/{language}"))
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .is_none_or(|value| value.with_timezone(&chrono::Utc) < cutoff)
                })
        })
        .collect::<Vec<_>>();
    let mut offerings = Vec::with_capacity(stale.len());
    let mut warnings = Vec::new();
    // All at once: the syllabus transport's limiter paces the requests.
    let handles = stale
        .iter()
        .map(|(_, url)| {
            let url = url.clone();
            tauri::async_runtime::spawn(async move {
                if summary_only {
                    syllabus::sync_offering_summary(&url).await
                } else {
                    syllabus::sync_offering(&url).await
                }
            })
        })
        .collect::<Vec<_>>();
    // Each result is stored as soon as it (and those before it) arrive.
    for ((id, _), handle) in stale.iter().zip(handles) {
        match handle.await {
            Ok(Ok(synced)) => {
                let offering =
                    serde_json::to_value(&synced.offering).map_err(|error| error.to_string())?;
                let content = synced
                    .content
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                store.catalog_upsert_offerings(&[offering])?;
                store.upsert_offering_content(&content)?;
                offerings.push(synced);
            }
            Ok(Err(error)) => warnings.push(format!("{id}: {error}")),
            Err(_) => warnings.push(format!("{id}: syllabus worker stopped unexpectedly")),
        }
    }
    Ok((offerings, warnings))
}

#[tauri::command]
async fn catalog_sync_department(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    year: u16,
    scope_path: String,
    quarter: Option<String>,
) -> Result<syllabus::DepartmentSyncResult, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let catalog = syllabus::department_catalog(year, &scope_path, quarter.as_deref()).await?;
    let targets = catalog
        .items
        .iter()
        .map(|item| (item.id.clone(), item.syllabus_url.clone()))
        .collect();
    let (offerings, warnings) = sync_stale_details(&store, targets, false).await?;
    Ok(syllabus::DepartmentSyncResult {
        listed: catalog.items.len(),
        synced: offerings.len(),
        truncated: catalog.truncated,
        offerings,
        warnings,
    })
}

/// One syllabus search, exactly as the official site answers it (at most 100
/// rows, no paging). Rows are stored without replacing fetched details.
#[tauri::command]
async fn catalog_query(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    query: String,
    year: u16,
    filters: Option<syllabus::SyllabusFilters>,
) -> Result<syllabus::CatalogSearchResult, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = syllabus::search_bilingual(&query, year, 100, filters).await?;
    store_search_items(&store, &result.items)?;
    Ok(result)
}

/// Detail pages for the given stored offerings (schedule, credits, format).
#[tauri::command]
async fn catalog_sync_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    ids: Vec<String>,
) -> Result<Vec<String>, String> {
    require_local_main(&webview)?;
    if ids.len() > 20 {
        return Err("Too many offerings in one request.".into());
    }
    let store = store.inner().clone();
    let mut targets = Vec::new();
    let mut missing = Vec::new();
    for id in ids {
        // A course found by a search is stored as a search row until its
        // details have been read once.
        let url = [
            store.catalog_offering(&id)?,
            store.catalog_search_item(&id)?,
        ]
        .into_iter()
        .flatten()
        .find_map(|row| {
            row.get("syllabusUrl")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
        match url {
            Some(url) => targets.push((id, url)),
            None => missing.push(format!("{id}: not in the catalog")),
        }
    }
    let (_, mut warnings) = sync_stale_details(&store, targets, true).await?;
    warnings.extend(missing);
    Ok(warnings)
}

#[tauri::command]
fn preview_plan_recurrence(
    webview: tauri::Webview,
    start: String,
    rrule: String,
    exdates: Vec<String>,
    limit: Option<u16>,
) -> Result<plans::RecurrencePreview, String> {
    require_local_main(&webview)?;
    plans::preview_rrule(&start, &rrule, &exdates, limit.unwrap_or(5))
}

#[tauri::command]
fn agent_command(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<String, String> {
    require_local_main(&webview)?;
    Ok(format!(
        "isct-companion-mcp --db {}",
        store.path().display()
    ))
}

#[tauri::command]
async fn apply_bundled_subscriptions(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    academic_year: i32,
    admission_year: i32,
) -> Result<usize, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        let (calendar, departments) =
            subscriptions::bundled_selection(academic_year, admission_year)?;
        let mut connection =
            rusqlite::Connection::open(store.path()).map_err(|error| error.to_string())?;
        subscriptions::apply_subscription(&mut connection, &calendar, &departments)
    })
    .await
}

#[tauri::command]
async fn probe_reference_source(
    webview: tauri::Webview,
    index_url: String,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let hash = probe_reference_source_inner(&index_url).await?;
    Ok(serde_json::json!({"indexHash":hash}))
}

async fn probe_reference_source_inner(index_url: &str) -> Result<String, String> {
    subscriptions::SubscriptionClient::new()?
        .probe_index_source(index_url)
        .await
}

#[tauri::command]
async fn refresh_reference_source(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    kind: String,
    index_url: String,
    year: i32,
    expected_index_hash: Option<String>,
    known_hash: Option<String>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    refresh_reference_source_inner(
        store.inner(),
        &kind,
        &index_url,
        year,
        expected_index_hash,
        known_hash,
    )
    .await
}

async fn refresh_reference_source_inner(
    store: &data::DataStore,
    kind: &str,
    index_url: &str,
    year: i32,
    expected_index_hash: Option<String>,
    known_hash: Option<String>,
) -> Result<serde_json::Value, String> {
    let client = subscriptions::SubscriptionClient::new()?;
    let mut connection = rusqlite::Connection::open(store.path()).map_err(|e| e.to_string())?;
    match kind {
        "calendar" => {
            let fetched = client
                .fetch_calendar(
                    &index_url,
                    year,
                    expected_index_hash.as_deref(),
                    known_hash.as_deref(),
                )
                .await?;
            let Some(value) = fetched.value else {
                return Ok(
                    serde_json::json!({"changed":0,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":true}),
                );
            };
            let changed = subscriptions::apply_reference(&mut connection, Some(&value), None)?;
            Ok(
                serde_json::json!({"changed":changed,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":false}),
            )
        }
        "departments" => {
            let fetched = client
                .fetch_departments(
                    &index_url,
                    year,
                    expected_index_hash.as_deref(),
                    known_hash.as_deref(),
                )
                .await?;
            let Some(value) = fetched.value else {
                return Ok(
                    serde_json::json!({"changed":0,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":true}),
                );
            };
            let changed = subscriptions::apply_reference(&mut connection, None, Some(&value))?;
            Ok(
                serde_json::json!({"changed":changed,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":false}),
            )
        }
        "important-dates" => {
            let fetched = client
                .fetch_important_dates(
                    &index_url,
                    year,
                    expected_index_hash.as_deref(),
                    known_hash.as_deref(),
                )
                .await?;
            let Some(value) = fetched.value else {
                return Ok(
                    serde_json::json!({"changed":0,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":true}),
                );
            };
            let changed = subscriptions::apply_important_dates(&mut connection, &value)?;
            Ok(
                serde_json::json!({"changed":changed,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":false}),
            )
        }
        "exams" => {
            let fetched = client
                .fetch_exams(
                    &index_url,
                    year,
                    expected_index_hash.as_deref(),
                    known_hash.as_deref(),
                )
                .await?;
            let Some(feeds) = fetched.value else {
                return Ok(
                    serde_json::json!({"changed":0,"dataUrl":fetched.data_url,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":true}),
                );
            };
            let mut changed = 0;
            let mut files = Vec::new();
            for feed in feeds {
                changed += subscriptions::apply_exam_feed(&mut connection, &feed)?;
                files.push(serde_json::json!({"level":feed.level,"quarter":feed.quarter.to_ascii_uppercase(),"dataUrl":feed.url}));
            }
            Ok(
                serde_json::json!({"changed":changed,"dataUrl":fetched.data_url,"files":files,"remoteHash":fetched.remote_hash,"sourceTs":fetched.source_ts,"notModified":false}),
            )
        }
        _ => Err("unsupported subscription type".into()),
    }
}

/// Keeps a plugin's Rust API but drops its JavaScript side. The notification
/// plugin's init script replaces `window.Notification` in every webview and
/// calls `is_permission_granted`, which no capability grants; the app
/// delivers notifications through its own `notify` command instead.
struct RustOnlyPlugin<R: tauri::Runtime>(tauri::plugin::TauriPlugin<R>);

impl<R: tauri::Runtime> tauri::plugin::Plugin<R> for RustOnlyPlugin<R> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    fn initialize(
        &mut self,
        app: &tauri::AppHandle<R>,
        config: serde_json::Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }

    fn on_event(&mut self, app: &tauri::AppHandle<R>, event: &tauri::RunEvent) {
        self.0.on_event(app, event)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("lmsfile", |context, request, responder| {
            eprintln!(
                "[lms-file] protocol {} from {}",
                request.method(),
                context.webview_label()
            );
            if context.webview_label() != "main" {
                responder.respond(
                    tauri::http::Response::builder()
                        .status(tauri::http::StatusCode::BAD_REQUEST)
                        .header(tauri::http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                        .header("X-Isct-Reason", "invalid")
                        .body(br#"{"reason":"invalid"}"#.to_vec())
                        .unwrap_or_else(|_| tauri::http::Response::new(Vec::new())),
                );
                return;
            }
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(lms_protocol::handle(app, request).await);
            });
        })
        .plugin(RustOnlyPlugin(tauri_plugin_notification::init()))
        // The plugin's default link script runs in every webview and turns a
        // `target="_blank"` click into an `open_url` IPC call, which no
        // capability grants: links in school pages silently did nothing.
        // School views route new-window links natively (see school_web.rs);
        // the shell's links use the `open_external` command.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_secure_storage::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(lms_files::FileStore(std::sync::Mutex::new(())))
        .manage(SchoolWebController::new())
        .manage(scheduler::Scheduler::default());
    #[cfg(desktop)]
    let builder = builder.manage(lms_downloader::Downloader::default());
    #[cfg(matrix_ocr)]
    let builder = builder.manage(matrix_ocr::MatrixOcrService::default());
    builder
        .invoke_handler(tauri::generate_handler![
            runtime_capabilities,
            lms_files::course_storage_info,
            lms_files::choose_course_storage,
            lms_files::save_course_note,
            lms_files::read_course_note,
            lms_files::move_course_note,
            lms_files::list_course_notes,
            lms_files::open_course_material,
            lms_files::open_course_file,
            lms_files::clean_course_materials,
            recognize_matrix_image,
            open_external,
            resolve_school_url,
            configure_school_web_policy,
            open_school_site,
            set_school_web_bounds,
            hide_school_web,
            reload_school_web,
            navigate_school_web,
            school_web_back,
            reveal_school_download,
            set_school_web_zoom,
            pull_school,
            school_records,
            set_kyomu_guide,
            school_credential_status,
            save_school_credentials,
            delete_school_credentials,
            copy_school_matrix,
            login_school,
            test_school_connection,
            ensure_school_session,
            request_notification_permission,
            notify,
            connect_cloud,
            cloud_authorization_status,
            disconnect_cloud,
            cloud_backup_workspace,
            cloud_restore_workspace,
            cloud_credential_sync_status,
            set_cloud_credential_sync_password,
            clear_cloud_credential_sync_password,
            load_workspace,
            new_user_record_id,
            put_user_record,
            delete_user_record,
            run_scheduled_task,
            put_workspace_setting,
            reset_workspace,
            export_data,
            save_file,
            write_scheduled_export,
            import_data,
            catalog_upsert_offerings,
            catalog_offering,
            catalog_offerings,
            catalog_sync_offering,
            catalog_sync_department,
            catalog_query,
            catalog_sync_offerings,
            preview_plan_recurrence,
            agent_command,
            apply_bundled_subscriptions,
            probe_reference_source,
            refresh_reference_source
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let (data_store, rebuilt) =
                data::DataStore::initialize(app_data_dir.join("school.db"))
                    .map_err(std::io::Error::other)?;
            if rebuilt {
                for legacy in ["workspace.json"] {
                    let path = app_data_dir.join(legacy);
                    if path.is_file() {
                        std::fs::remove_file(path)?;
                    }
                }
            }
            app.manage(data_store);
            let vault = OsSecretVault::new(app.handle().clone());
            app.manage(
                DesktopCloudService::new(vault.clone())
                    .expect("failed to initialize the cloud transport"),
            );
            app.manage(SchoolAuthService::with_status_cache(vault, app.path().app_data_dir()?.join("school-credential-status.json")));
            scheduler::Scheduler::start(app.handle().clone());

            #[cfg(desktop)]
            {
                #[cfg(dev)]
                let main_url = WebviewUrl::External(
                    tauri::Url::parse("http://localhost:4173")
                        .expect("the local Vite URL is valid"),
                );
                #[cfg(not(dev))]
                let main_url = WebviewUrl::App("index.html".into());
                WebviewWindowBuilder::new(app, "main", main_url)
                .title("IsctCompanion")
                .inner_size(1280.0, 820.0)
                .min_inner_size(860.0, 620.0)
                .initialization_script(
                    "Object.defineProperty(window,'__ISCT_COMPANION_NATIVE_TARGET__',{value:'native-desktop',writable:false,configurable:false});",
                )
                .build()?;
            }

            #[cfg(mobile)]
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(
                    "Object.defineProperty(window,'__ISCT_COMPANION_NATIVE_TARGET__',{value:'native-mobile',writable:false,configurable:false});",
                )
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build IsctCompanion")
        .run(|_, _| {});
}
