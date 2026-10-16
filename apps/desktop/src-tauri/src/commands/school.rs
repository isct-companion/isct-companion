use crate::{
    core_ports, kyomu_guide,
    lifecycle::SchoolSite,
    pull_coordinator, require_local_main, school_refresh,
    school_web::{self, SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds},
};
use isct_core::school_browser::sanitize_school_url;
use tauri::Manager;
use tauri_plugin_opener::OpenerExt;

/// School pages keep their sanitized form; any other link must be plain
/// http(s) without embedded credentials.
pub(crate) fn external_url(raw: &str) -> Option<String> {
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
pub(crate) fn resolve_school_url(
    webview: tauri::Webview,
    url: String,
) -> Result<serde_json::Value, String> {
    require_local_main(&webview)?;
    let safe = sanitize_school_url(&url)
        .ok_or_else(|| "Use an official Science Tokyo school URL.".to_string())?;
    let site = SchoolSite::for_url(&safe)
        .ok_or_else(|| "Use an official Science Tokyo school URL.".to_string())?;
    Ok(serde_json::json!({"site": site, "url": safe}))
}

#[tauri::command]
pub(crate) fn open_external(
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
pub(crate) fn configure_school_web_policy(
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
pub(crate) async fn open_school_site(
    webview: tauri::Webview,
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
    controller.open(site, css, js, bounds, target_url).await
}

#[tauri::command]
pub(crate) fn set_school_web_bounds(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    bounds: WebBounds,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.set_bounds(bounds)
}

#[tauri::command]
pub(crate) fn hide_school_web(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
) -> Result<bool, String> {
    if let Err(error) = require_local_main(&webview) {
        eprintln!("hide_school_web rejected for {}: {error}", webview.label());
        return Err(error);
    }
    controller.hide_all()
}

#[tauri::command]
pub(crate) fn reload_school_web(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.engine.reload(site).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn navigate_school_web(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    url: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller
        .engine
        .navigate(site, &url)
        .map_err(|e| e.to_string())
}

/// The app's own Back for the school view: reloads the previous page as a
/// GET instead of `history.back()` (which would resubmit postbacks).
#[tauri::command]
pub(crate) fn school_web_back(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.engine.back(site).map_err(|e| e.to_string())
}

/// The app's Forward for the school view, the counterpart of Back.
#[tauri::command]
pub(crate) fn school_web_forward(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.engine.forward(site).map_err(|e| e.to_string())
}

/// Shows a file a school page saved this session in Finder; other paths are
/// refused.
#[tauri::command]
pub(crate) fn reveal_school_download(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    path: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller.reveal_download(&app, &path)
}

#[tauri::command]
pub(crate) fn set_school_web_zoom(
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
pub(crate) async fn pull_school(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    site: SchoolSite,
    lms_course_id: Option<u64>,
    current_only: Option<bool>,
) -> Result<Vec<school_web::IngestOutcome>, String> {
    require_local_main(&webview)?;
    let services = core_ports::services(&app);
    core_ports::runtime(&app)
        .run(async move {
            if site == SchoolSite::ScienceLms && current_only == Some(true) {
                return services
                    .school
                    .pull_current_term(&services.store, services.executor)
                    .await;
            }
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
pub(crate) async fn pull_state(
    webview: tauri::Webview,
    app: tauri::AppHandle,
) -> Result<Vec<pull_coordinator::PullSnapshot>, String> {
    require_local_main(&webview)?;
    Ok(core_ports::runtime(&app).pull_snapshots().await)
}

#[tauri::command]
pub(crate) async fn cancel_pull(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    id: String,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    Ok(core_ports::runtime(&app).cancel_pull(&id).await)
}

#[tauri::command]
pub(crate) async fn refresh_reference_subscriptions(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    force: bool,
) -> Result<String, String> {
    require_local_main(&webview)?;
    let references = core_ports::services(&app).references;
    core_ports::runtime(&app)
        .run(async move { references.refresh_all(force).await })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn set_kyomu_guide(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    items: Vec<kyomu_guide::GuideItem>,
) -> Result<bool, String> {
    require_local_main(&webview)?;
    controller
        .engine
        .set_guide(items)
        .map_err(|e| e.to_string())
}

/// Async: conditions are republished without blocking the main thread on
/// the storage read they need.
#[tauri::command]
pub(crate) async fn school_refresh_event(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    event: school_refresh::RefreshEvent,
) -> Result<(), String> {
    require_local_main(&webview)?;
    let runtime = core_ports::runtime(&app);
    runtime
        .refresh_event(event)
        .map_err(|error| error.to_string())?;
    let visible = app.state::<SchoolWebController>().is_visible();
    let store = app.state::<crate::data::DataStore>().inner().clone();
    runtime
        .publish_conditions(&store, visible)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "../tests/commands__school__external_url_tests.rs"]
mod external_url_tests;
