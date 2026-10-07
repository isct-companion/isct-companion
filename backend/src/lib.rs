use serde::Serialize;
use tauri::{plugin::PermissionState, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;
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

mod cloud;
pub mod data;
mod kyomu_guide;
mod kyomu_ingest;
mod lifecycle;
mod lms_ingest;
mod lms_files;
#[cfg(matrix_ocr)]
mod matrix_ocr;
#[cfg(matrix_ocr)]
mod matrix_red_crops;
pub mod plans;
mod policy;
mod school_auth;
mod school_extract;
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
    Err("Scan image is not available on Intel Macs yet. Type or paste the 70 letters instead.".into())
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
        school_import: false,
        school_web: true,
        notifications: true,
        background_schedule: false,
        cloud_sync: true,
        syllabus_search: true,
        syllabus_filters: true,
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
) -> Result<Vec<school_web::IngestOutcome>, String> {
    require_local_main(&webview)?;
    let controller = app.state::<SchoolWebController>().inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || controller.pull(&handle, site))
        .await
        .map_err(|_| "The school pull stopped unexpectedly.".to_string())?
}

/// School-owned records the workspace does not carry (LMS courses,
/// registrations, grades, GPA by term, requirements).
#[tauri::command]
fn school_records(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
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
            .filter(|row| row.get("deletedAt").map_or(true, serde_json::Value::is_null))
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

/// Writes the saved Matrix from the vault straight to a file the person picks,
/// so the plaintext never passes through the WebView.
#[tauri::command]
async fn export_school_matrix(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    auth: tauri::State<'_, SchoolAuthService<OsSecretVault>>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    use std::io::Write;

    let document = auth.matrix_file()?;
    let dialog = app
        .dialog()
        .file()
        .set_title("Export Matrix")
        .set_file_name("matrix.json")
        .add_filter("JSON", &["json"]);
    #[cfg(desktop)]
    let dialog = dialog.set_parent(&webview.window());
    let Some(path) = dialog.blocking_save_file() else {
        return Ok(false);
    };
    let path = path
        .into_path()
        .map_err(|_| "Choose a file on this device.".to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(&path)
        .and_then(|mut file| file.write_all(document.as_bytes()))
        .map_err(|_| "Matrix file could not be written.".to_string())?;
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
fn load_workspace(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    store.load_workspace()
}

#[tauri::command]
async fn save_workspace(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    workspace: serde_json::Value,
) -> Result<(), String> {
    require_local_main(&webview)?;
    let store = store.inner().clone();
    tauri::async_runtime::spawn_blocking(move || store.save_workspace(&workspace).map(|_| ()))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
fn reset_workspace(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    store.reset_workspace()
}

#[tauri::command]
fn export_data(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
) -> Result<String, String> {
    require_local_main(&webview)?;
    store.export_data()
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
fn write_scheduled_export(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    store: tauri::State<'_, data::DataStore>,
) -> Result<String, String> {
    require_local_main(&webview)?;
    use std::io::Write;

    let document = store.export_data()?;
    if document.len() > 32 * 1024 * 1024 {
        return Err("scheduled export exceeds the 32 MiB safety limit".into());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&document).map_err(|_| "scheduled export is invalid JSON")?;
    if parsed
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(3)
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
fn import_data(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    document: String,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    store.import_data(&document)
}

#[tauri::command]
fn catalog_upsert_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    offerings: Vec<serde_json::Value>,
) -> Result<usize, String> {
    require_local_main(&webview)?;
    store.catalog_upsert_offerings(&offerings)
}

fn stored_catalog_search(
    store: &data::DataStore,
    query: &str,
    year: u16,
    limit: usize,
) -> Result<syllabus::CatalogSearchResult, String> {
    let mut items = store
        .catalog_search(query)?
        .into_iter()
        .filter_map(|value| serde_json::from_value::<syllabus::CatalogSearchItem>(value).ok())
        .filter(|item| item.year == year)
        .take(limit.clamp(1, 100))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.code.cmp(&right.code).then(left.id.cmp(&right.id)));
    Ok(syllabus::CatalogSearchResult {
        returned: items.len(),
        total: items.len(),
        items,
        truncated: false,
    })
}

#[tauri::command]
async fn catalog_search(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    query: String,
    year: u16,
    limit: Option<usize>,
    filters: Option<syllabus::SyllabusFilters>,
    refresh: Option<bool>,
) -> Result<syllabus::CatalogSearchResult, String> {
    require_local_main(&webview)?;
    let limit = limit.unwrap_or(100).clamp(1, 100);
    let store = store.inner().clone();
    let local = stored_catalog_search(&store, &query, year, limit)?;
    if refresh != Some(true) {
        return Ok(local);
    }
    match syllabus::search_bilingual(&query, year, limit, filters).await {
        Ok(remote) => {
            store_search_items(&store, &remote.items)?;
            Ok(remote)
        }
        Err(_) if !local.items.is_empty() => Ok(local),
        Err(error) => Err(error),
    }
}

/// Every stored offering of one academic year, with schedule, credits, language and
/// format, so the course finder can filter and suggest locally without a round trip.
#[tauri::command]
fn catalog_offerings(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    year: u16,
) -> Result<Vec<serde_json::Value>, String> {
    require_local_main(&webview)?;
    Ok(store
        .catalog_search("")?
        .into_iter()
        .filter(|row| {
            row.get("year").and_then(serde_json::Value::as_u64) == Some(u64::from(year))
                && row.get("deletedAt").map_or(true, serde_json::Value::is_null)
        })
        .collect())
}

#[tauri::command]
fn catalog_offering(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    id: String,
) -> Result<Option<serde_json::Value>, String> {
    require_local_main(&webview)?;
    store.catalog_offering(&id)
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
            let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
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
        store.catalog_upsert_offerings(&rows)?;
    }
    Ok(())
}

/// Fetches detail pages for offerings that have none or a stale one (older
/// than a week), two at a time, storing each as it arrives.
async fn sync_stale_details(
    store: &data::DataStore,
    targets: Vec<(String, String)>,
) -> Result<(Vec<syllabus::SyncOfferingResult>, Vec<String>), String> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(7);
    let stale = targets
        .into_iter()
        .filter(|(id, _)| {
            let Ok(Some(stored)) = store.catalog_offering(id) else {
                return true;
            };
            stored.get("schedule").is_none()
                || ["ja", "en"].iter().any(|language| {
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
    for (index, pair) in stale.chunks(2).enumerate() {
        if index > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        let handles = pair
            .iter()
            .map(|(_, url)| {
                let url = url.clone();
                tauri::async_runtime::spawn(async move { syllabus::sync_offering(&url).await })
            })
            .collect::<Vec<_>>();
        for ((id, _), handle) in pair.iter().zip(handles) {
            match handle.await {
                Ok(Ok(synced)) => {
                    let offering = serde_json::to_value(&synced.offering)
                        .map_err(|error| error.to_string())?;
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
    let (offerings, warnings) = sync_stale_details(&store, targets).await?;
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
    for id in ids {
        let url = store
            .catalog_offering(&id)?
            .and_then(|row| row.get("syllabusUrl").and_then(serde_json::Value::as_str).map(str::to_string))
            .ok_or_else(|| format!("{id}: not in the catalog"))?;
        targets.push((id, url));
    }
    let (_, warnings) = sync_stale_details(&store, targets).await?;
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
    Ok(format!("campus-mcp --db {}", store.path().display()))
}

#[tauri::command]
fn apply_bundled_subscriptions(
    webview: tauri::Webview,
    store: tauri::State<'_, data::DataStore>,
    academic_year: i32,
    admission_year: i32,
) -> Result<usize, String> {
    require_local_main(&webview)?;
    let (calendar, departments) = subscriptions::bundled_selection(academic_year, admission_year)?;
    let mut connection =
        rusqlite::Connection::open(store.path()).map_err(|error| error.to_string())?;
    subscriptions::apply_subscription(&mut connection, &calendar, &departments)
}

#[tauri::command]
async fn refresh_reference_source(webview: tauri::Webview, store: tauri::State<'_, data::DataStore>, kind: String, index_url: String, year: i32) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let client = subscriptions::SubscriptionClient::new()?;
    let (calendar, departments, url) = match kind.as_str() {
        "calendar" => { let (value, url) = client.fetch_calendar(&index_url, year).await?; (Some(value), None, url) },
        "departments" => { let (value, url) = client.fetch_departments(&index_url, year).await?; (None, Some(value), url) },
        _ => return Err("unsupported subscription type".into()),
    };
    let mut connection = rusqlite::Connection::open(store.path()).map_err(|e| e.to_string())?;
    let changed = subscriptions::apply_reference(&mut connection, calendar.as_ref(), departments.as_ref())?;
    Ok(serde_json::json!({"changed":changed,"dataUrl":url}))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
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
        .manage(lms_files::FileStore(std::sync::Mutex::new(()), std::sync::Arc::new(tauri::async_runtime::Mutex::new(())), std::env::temp_dir().join(format!("isct-course-preview-{}", uuid::Uuid::new_v4()))))
        .plugin(tauri_plugin_secure_storage::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(SchoolWebController::new());
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
            lms_files::save_course_material,
            recognize_matrix_image,
            open_external,
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
            export_school_matrix,
            login_school,
            test_school_connection,
            ensure_school_session,
            request_notification_permission,
            notify,
            connect_cloud,
            cloud_request,
            load_workspace,
            save_workspace,
            reset_workspace,
            export_data,
            save_file,
            write_scheduled_export,
            import_data,
            catalog_upsert_offerings,
            catalog_search,
            catalog_offering,
            catalog_offerings,
            catalog_sync_offering,
            catalog_sync_department,
            catalog_query,
            catalog_sync_offerings,
            preview_plan_recurrence,
            agent_command,
            apply_bundled_subscriptions,
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
        .build(tauri::generate_context!())
        .expect("failed to build Science Tokyo Workstation")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let _ = std::fs::remove_dir_all(&app.state::<lms_files::FileStore>().2);
            }
        });
}
