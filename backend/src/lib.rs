#![recursion_limit = "256"]

use serde::Serialize;
use tauri::{plugin::PermissionState, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use cloud::{CloudProvider, DesktopCloudService};
use lifecycle::SchoolSite;
use school_auth::SchoolAuthService;
use school_extract::sanitize_school_url;
use school_web::{SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds};
use secrets::OsSecretVault;

#[cfg(target_os = "macos")]
mod campus_wifi;
mod cloud;
mod cloud_sync;
mod commands;
pub use isct_core::contracts;
pub use isct_core::data;
#[cfg(any(target_os = "macos", target_os = "ios"))]
mod icloud;
use isct_core::kyomu_guide;
pub(crate) use isct_core::kyomu_ingest;
use isct_core::lifecycle;
#[cfg(desktop)]
mod lms_downloader;
mod lms_files;
pub(crate) use isct_core::lms_ingest;
mod lms_protocol;
pub use isct_core::plans;
use isct_core::pull_coordinator;
#[cfg(matrix_ocr)]
use isct_ocr as matrix_ocr;
mod core_ports;
pub(crate) use isct_core::school_auth;
mod school_extract;
pub(crate) use isct_core::school_parser;
use isct_core::school_refresh;
#[cfg(not(mobile))]
mod school_web;
#[cfg(mobile)]
#[path = "school_web_mobile.rs"]
mod school_web;
mod school_web_shared;
mod secrets;
pub(crate) use isct_core::syllabus;
#[cfg(desktop)]
mod workspaces;

/// The frontend's build-time identity: shell kind and operating system, both
/// fixed when this binary is compiled, so the UI never sniffs user agents.
fn native_bootstrap() -> String {
    #[cfg(desktop)]
    const TARGET: &str = "native-desktop";
    #[cfg(mobile)]
    const TARGET: &str = "native-mobile";
    format!(
        "Object.defineProperty(window,'__ISCT_COMPANION_NATIVE_TARGET__',{{value:'{TARGET}',writable:false,configurable:false}});\
         Object.defineProperty(window,'__ISCT_COMPANION_NATIVE_OS__',{{value:'{}',writable:false,configurable:false}});",
        std::env::consts::OS
    )
}

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
    app: tauri::AppHandle,
    data: String,
    preview: String,
    on_progress: tauri::ipc::Channel<usize>,
) -> Result<Vec<matrix_ocr::Recognition>, String> {
    require_local_main(&webview)?;
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        let service = service.with_debug_dir(
            lms_files::debug_dir(&app)
                .inspect_err(|error| eprintln!("Matrix debug directory unavailable: {error}"))
                .ok(),
        );
        #[cfg(not(debug_assertions))]
        let _ = app;
        service
            .scan(&data, &preview, |completed| {
                let _ = on_progress.send(completed);
            })
            .map_err(|error| error.to_string())
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
        school_import: true,
        school_web: true,
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
// Tauri supplies the first three parameters and the remaining fields preserve
// the established IPC command shape.
#[allow(clippy::too_many_arguments)]
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

/// The app's Forward for the school view, the counterpart of Back.
#[tauri::command]
fn school_web_forward(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.forward(&app, site)
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
    let services = core_ports::services(&app);
    core_ports::runtime(&app)
        .run(async move {
            services
                .school
                .pull(
                    services.executor,
                    site,
                    lms_course_id,
                    pull_coordinator::PullOptions::manual(),
                    false,
                )
                .await
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn pull_state(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<Vec<pull_coordinator::PullSnapshot>, String> {
    require_local_main(&webview)?;
    Ok(core_ports::runtime(&app).coordinator.snapshots().await)
}

#[tauri::command]
async fn cancel_pull(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    Ok(core_ports::runtime(&app).coordinator.cancel(&id).await)
}

#[tauri::command]
async fn refresh_reference_subscriptions(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    force: bool,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let references = core_ports::services(&app).references;
    core_ports::runtime(&app)
        .run(async move { references.refresh_all(force, true).await })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
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
            .table_rows(table)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|row| row.get("deletedAt").is_none_or(serde_json::Value::is_null))
            .collect();
        records.insert(key.into(), serde_json::Value::Array(rows));
    }
    records.insert(
        "lastRuns".into(),
        store
            .last_import_runs()
            .map_err(|error| error.to_string())?,
    );
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
fn school_refresh_event(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    event: school_refresh::RefreshEvent,
) -> Result<(), String> {
    require_local_main(&webview)?;
    core_ports::runtime(&app)
        .refresh
        .event(event)
        .map_err(|error| error.to_string())?;
    core_ports::publish_conditions(&app)
}

#[tauri::command]
async fn cloud_lifecycle(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    kind: String,
) -> Result<(), String> {
    require_local_main(&webview)?;
    if matches!(kind.as_str(), "foreground" | "background") {
        cloud_sync::CloudSync::changed(&app).await;
        app.state::<cloud_sync::CloudSync>().resume(&app).await?;
    }
    Ok(())
}

#[tauri::command]
fn cloud_sync_state(
    webview: tauri::Webview,
    sync: tauri::State<'_, cloud_sync::CloudSync>,
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    sync.state()
}

#[tauri::command]
async fn sync_cloud_now(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    direction: cloud_sync::Direction,
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .sync(&app, direction)
        .await
}

#[tauri::command]
async fn set_cloud_sync_automatic(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    automatic: bool,
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .set_automatic(&app, automatic)
        .await
}

#[tauri::command]
fn cloud_recovery_document(
    webview: tauri::Webview,
    sync: tauri::State<'_, cloud_sync::CloudSync>,
    id: String,
) -> Result<String, String> {
    require_local_main(&webview)?;
    sync.recovery_document(&id)
}

#[tauri::command]
async fn restore_cloud_version(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .restore(&app, &id)
        .await
}

#[tauri::command]
async fn set_cloud_sync_provider(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    provider: CloudProvider,
    enabled: bool,
) -> Result<cloud_sync::SyncState, String> {
    require_local_main(&webview)?;
    if enabled && provider == CloudProvider::GoogleDrive {
        let service = app.state::<DesktopCloudService<OsSecretVault>>();
        if !service.authorized_providers()?.contains(&provider) {
            let workspace = app
                .state::<data::DataStore>()
                .load_workspace()
                .map_err(|error| error.to_string())?;
            let override_id = workspace
                .pointer("/settings/cloud/clientId")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty());
            #[cfg(desktop)]
            let configured = option_env!("ISCT_GOOGLE_DESKTOP_CLIENT_ID");
            #[cfg(target_os = "ios")]
            let configured = option_env!("ISCT_GOOGLE_IOS_CLIENT_ID");
            #[cfg(target_os = "android")]
            let configured = option_env!("ISCT_GOOGLE_ANDROID_CLIENT_ID");
            let client_id = override_id.or(configured).unwrap_or("").to_string();
            if let Err(error) = connect_cloud(
                webview,
                app.clone(),
                service,
                provider,
                client_id,
                "http://127.0.0.1:42813/oauth/callback".into(),
            )
            .await
            {
                return app
                    .state::<cloud_sync::CloudSync>()
                    .provider_result(&app, provider, false, Some(error))
                    .await;
            }
        }
    }
    app.state::<cloud_sync::CloudSync>()
        .set_provider(&app, provider, enabled)
        .await?;
    cloud_sync::CloudSync::changed(&app).await;
    app.state::<cloud_sync::CloudSync>().state()
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
    if provider == CloudProvider::Icloud {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        return service.connect_icloud().await;
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        return Err("iCloud sync is only available on Apple devices.".into());
    }
    #[cfg(mobile)]
    {
        let _ = redirect_uri;
        if provider != CloudProvider::GoogleDrive {
            return Err("This cloud service is not available on this device.".into());
        }
        let token = tauri_plugin_google_drive_auth::sign_in(&app, &client_id)?;
        service.connect_with_native_token(
            provider,
            &token.client_id,
            token.access_token,
            token.refresh_token,
            token.expires_in,
        )
    }
    #[cfg(desktop)]
    {
        service
            .connect(provider, &client_id, &redirect_uri, move |url| {
                app.opener()
                    .open_url(url, None::<&str>)
                    .map_err(|_| "system browser could not be opened".to_string())
            })
            .await
    }
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
    app: tauri::AppHandle,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    #[cfg(target_os = "ios")]
    if provider == CloudProvider::GoogleDrive {
        tauri_plugin_google_drive_auth::sign_out(&app)?;
    }
    #[cfg(not(target_os = "ios"))]
    let _ = app;
    service.disconnect(provider)?;
    Ok(true)
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
async fn cloud_credential_backup_status(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    provider: CloudProvider,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    service.credential_backup_exists(provider).await
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
async fn sync_cloud_credentials(
    webview: tauri::Webview,
    service: tauri::State<'_, DesktopCloudService<OsSecretVault>>,
    auth: tauri::State<'_, std::sync::Arc<SchoolAuthService>>,
    provider: CloudProvider,
    restore_existing: bool,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    if provider == CloudProvider::Icloud {
        return Err("iCloud credentials sync through iCloud Keychain.".into());
    }
    if restore_existing {
        if let Some(credentials) = service.pull_credentials(provider).await? {
            auth.import_sync_payload(&credentials)
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
    }
    let credentials = auth
        .export_sync_payload()
        .map_err(|error| error.to_string())?;
    service.back_up_credentials(provider, &credentials).await?;
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
    run_data_command(move || store.load_workspace().map_err(|error| error.to_string())).await
}

#[tauri::command]
fn new_user_record_id(webview: tauri::Webview) -> Result<String, String> {
    require_local_main(&webview)?;
    Ok(uuid::Uuid::now_v7().to_string())
}

#[tauri::command]
async fn put_user_record(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    table: String,
    record: serde_json::Value,
    expected_rev: Option<u64>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = run_data_command(move || {
        store
            .put_user_record_checked(&table, record, "desktop", expected_rev)
            .map_err(|error| error.to_string())
    })
    .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
}

#[tauri::command]
async fn delete_user_record(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    table: String,
    id: String,
    expected_rev: Option<u64>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = run_data_command(move || {
        store
            .delete_user_record_checked(&table, &id, "desktop", expected_rev)
            .map_err(|error| error.to_string())
    })
    .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
}

#[tauri::command]
async fn run_scheduled_task(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<String, String> {
    require_local_main(&webview)?;
    core_ports::runtime(&app)
        .run_scheduled(core_ports::services(&app), id)
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn put_workspace_setting(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
    key: String,
    value: serde_json::Value,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    let result = run_data_command(move || {
        store
            .put_workspace_setting(&key, value, "desktop")
            .map_err(|error| error.to_string())?;
        store
            .load_workspace()
            .map_err(|error| error.to_string())?
            .get("currentTerm")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "current academic term is unavailable".to_string())
    })
    .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
}

#[tauri::command]
async fn reset_workspace(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    app.state::<cloud_sync::CloudSync>()
        .prepare_reset(&app)
        .await?;
    let store = store.inner().clone();
    let result =
        run_data_command(move || store.reset_workspace().map_err(|error| error.to_string()))
            .await?;
    cloud_sync::CloudSync::changed(&app).await;
    Ok(result)
}

#[tauri::command]
async fn catalog_upsert_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    offerings: Vec<serde_json::Value>,
) -> Result<usize, String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    run_data_command(move || {
        store
            .catalog_upsert_offerings(&offerings)
            .map_err(|error| error.to_string())
    })
    .await
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
            .catalog_search("")
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|row| {
                row.get("year").and_then(serde_json::Value::as_u64) == Some(u64::from(year))
                    && row.get("deletedAt").is_none_or(serde_json::Value::is_null)
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
    run_data_command(move || {
        store
            .catalog_offering(&id)
            .map_err(|error| error.to_string())
    })
    .await
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
        if let Some(stored) = store
            .catalog_offering(&offering_id)
            .map_err(|error| error.to_string())?
        {
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
                    .table_rows("offering_content")
                    .map_err(|error| error.to_string())?
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
    let result = syllabus::sync_offering(&id)
        .await
        .map_err(|error| error.to_string())?;
    let offering = serde_json::to_value(&result.offering).map_err(|error| error.to_string())?;
    let content = result
        .content
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    store
        .catalog_upsert_offerings(&[offering])
        .map_err(|error| error.to_string())?;
    store
        .upsert_offering_content(&content)
        .map_err(|error| error.to_string())?;
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
        if store
            .catalog_offering(&item.id)
            .map_err(|error| error.to_string())?
            .is_none()
        {
            rows.push(serde_json::to_value(item).map_err(|error| error.to_string())?);
        }
    }
    if !rows.is_empty() {
        store
            .catalog_upsert_search_items(&rows)
            .map_err(|error| error.to_string())?;
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
                store
                    .catalog_upsert_offerings(&[offering])
                    .map_err(|error| error.to_string())?;
                store
                    .upsert_offering_content(&content)
                    .map_err(|error| error.to_string())?;
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
    let catalog = syllabus::department_catalog(year, &scope_path, quarter.as_deref())
        .await
        .map_err(|error| error.to_string())?;
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
    let result = syllabus::search_bilingual(&query, year, 100, filters)
        .await
        .map_err(|error| error.to_string())?;
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
            store
                .catalog_offering(&id)
                .map_err(|error| error.to_string())?,
            store
                .catalog_search_item(&id)
                .map_err(|error| error.to_string())?,
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
        .map_err(|error| error.to_string())
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
        .manage(SchoolWebController::new())
        .manage(core_ports::DesktopRuntime::default())
        .manage(std::sync::Arc::new(isct_core::runtime::CoreRuntime::new(
            if cfg!(mobile) {
                pull_coordinator::PullNetwork::Cellular
            } else {
                pull_coordinator::PullNetwork::Wifi
            },
        )));
    #[cfg(target_os = "macos")]
    let builder = builder.manage(campus_wifi::CampusWifi::default());
    #[cfg(desktop)]
    let builder = builder.manage(lms_downloader::SessionView::default());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(tauri_plugin_school_web::init());
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_google_drive_auth::init());
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
            #[cfg(desktop)]
            workspaces::assignment_workspace,
            #[cfg(desktop)]
            workspaces::create_assignment_workspace,
            #[cfg(desktop)]
            workspaces::open_workspace,
            #[cfg(desktop)]
            workspaces::set_workspace_submission,
            #[cfg(desktop)]
            workspaces::prepare_workspace_submission,
            #[cfg(desktop)]
            workspaces::delete_workspace,
            #[cfg(desktop)]
            workspaces::clean_workspaces,
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
            school_web_forward,
            reveal_school_download,
            set_school_web_zoom,
            pull_school,
            pull_state,
            cancel_pull,
            refresh_reference_subscriptions,
            school_records,
            set_kyomu_guide,
            commands::school_auth::school_credential_status,
            commands::school_auth::save_school_credentials,
            commands::school_auth::delete_school_credentials,
            commands::school_auth::copy_school_matrix,
            commands::school_auth::login_school,
            commands::school_auth::test_school_connection,
            commands::school_auth::ensure_school_session,
            request_notification_permission,
            notify,
            school_refresh_event,
            cloud_lifecycle,
            cloud_sync_state,
            set_cloud_sync_provider,
            sync_cloud_now,
            set_cloud_sync_automatic,
            restore_cloud_version,
            cloud_recovery_document,
            connect_cloud,
            cloud_authorization_status,
            disconnect_cloud,
            cloud_credential_sync_status,
            cloud_credential_backup_status,
            set_cloud_credential_sync_password,
            sync_cloud_credentials,
            clear_cloud_credential_sync_password,
            load_workspace,
            new_user_record_id,
            put_user_record,
            delete_user_record,
            run_scheduled_task,
            put_workspace_setting,
            reset_workspace,
            commands::backup::export_data,
            commands::backup::save_file,
            commands::backup::write_scheduled_export,
            commands::backup::import_data,
            catalog_upsert_offerings,
            catalog_offering,
            catalog_offerings,
            catalog_sync_offering,
            catalog_sync_department,
            catalog_query,
            catalog_sync_offerings,
            preview_plan_recurrence,
            agent_command,
            #[cfg(target_os = "macos")]
            campus_wifi::campus_wifi_connect,
            #[cfg(target_os = "macos")]
            campus_wifi::campus_wifi_status
        ])
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            let (data_store, rebuilt) = data::DataStore::initialize(app_data_dir.join("school.db"))
                .map_err(std::io::Error::other)?;
            if rebuilt {
                let path = app_data_dir.join("workspace.json");
                if path.is_file() {
                    std::fs::remove_file(path)?;
                }
            }
            app.manage(data_store);
            let vault = OsSecretVault::new(app.handle().clone());
            app.manage(
                DesktopCloudService::new(vault.clone())
                    .expect("failed to initialize the cloud transport"),
            );
            app.manage(std::sync::Arc::new(SchoolAuthService::with_status_cache(
                vault,
                app.path()
                    .app_data_dir()?
                    .join("school-credential-status.json"),
            )));
            let authorized = app
                .state::<DesktopCloudService<OsSecretVault>>()
                .authorized_providers()
                .unwrap_or_default();
            app.manage(
                cloud_sync::CloudSync::new(app_data_dir.join("cloud-sync.json"), &authorized)
                    .map_err(std::io::Error::other)?,
            );
            core_ports::runtime(app.handle())
                .attach_cloud(app.state::<cloud_sync::CloudSync>().0.clone())
                .map_err(std::io::Error::other)?;
            core_ports::DesktopRuntime::start(app.handle());

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
                    .initialization_script(native_bootstrap())
                    .build()?;
            }

            #[cfg(mobile)]
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .initialization_script(native_bootstrap())
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build IsctCompanion")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                let runtime = app.state::<core_ports::DesktopRuntime>();
                if !runtime.stopped() {
                    api.prevent_exit();
                    if runtime.begin_exit() {
                        let app = app.clone();
                        let code = code.unwrap_or(0);
                        tauri::async_runtime::spawn(async move {
                            cloud_sync::CloudSync::changed(&app).await;
                            app.state::<core_ports::DesktopRuntime>()
                                .shutdown(&core_ports::runtime(&app))
                                .await;
                            app.exit(code);
                        });
                    }
                }
            }
            // OS suspension differs from losing keyboard focus. Only real
            // suspension pauses admission in every lane; focus remains a fact.
            #[cfg(mobile)]
            if let tauri::RunEvent::WindowEvent { event, .. } = &event {
                match event {
                    tauri::WindowEvent::Suspended => {
                        tauri::async_runtime::block_on(core_ports::runtime(app).suspend())
                    }
                    tauri::WindowEvent::Resumed => {
                        tauri::async_runtime::block_on(core_ports::runtime(app).resume())
                    }
                    _ => (),
                }
            }
            if matches!(&event, tauri::RunEvent::Resumed) {
                tauri::async_runtime::block_on(core_ports::runtime(app).resume());
            }

            let foreground = match &event {
                tauri::RunEvent::Resumed => Some(true),
                tauri::RunEvent::WindowEvent {
                    label,
                    event: tauri::WindowEvent::Focused(focused),
                    ..
                } if label == "main" => Some(*focused),
                #[cfg(mobile)]
                tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::Suspended,
                    ..
                } => Some(false),
                #[cfg(mobile)]
                tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::Resumed,
                    ..
                } => Some(true),
                _ => None,
            };
            if let Some(foreground) = foreground {
                let _ = core_ports::runtime(app)
                    .refresh
                    .event(school_refresh::RefreshEvent {
                        kind: if foreground {
                            "foreground"
                        } else {
                            "background"
                        }
                        .into(),
                        profile: None,
                        course_id: None,
                        network: None,
                    });
                let _ = core_ports::publish_conditions(app);
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    cloud_sync::CloudSync::changed(&app).await;
                    let _ = app.state::<cloud_sync::CloudSync>().resume(&app).await;
                });
            }
            #[cfg(target_os = "ios")]
            if let tauri::RunEvent::Opened { urls } = event {
                for url in urls {
                    if let Err(error) =
                        tauri_plugin_google_drive_auth::handle_url(app, url.as_str())
                    {
                        eprintln!("Google OAuth callback could not be handled: {error}");
                    }
                }
            }
            #[cfg(not(target_os = "ios"))]
            let _ = (app, event);
        });
}
