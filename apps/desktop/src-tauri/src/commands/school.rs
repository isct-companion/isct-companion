use crate::{coded, CommandError};
use crate::{
    kyomu_guide,
    lifecycle::SchoolSite,
    require_local_main,
    school_web::{SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds},
};
use app_core::school_browser::{SchoolWebEntry, SchoolWebPage};
use tauri_plugin_opener::OpenerExt;

/// Explicit user browsing preserves the full web address, including search
/// state. Core automation uses its separate official-origin sanitizer.
pub(crate) fn external_url(raw: &str) -> Option<String> {
    app_core::school_browser::browsing_url(raw)
}

#[tauri::command]
pub(crate) fn resolve_school_url(
    webview: tauri::Webview,
    url: String,
    site: SchoolSite,
) -> Result<app_core::school_browser::SchoolWebTarget, CommandError> {
    require_local_main(&webview)?;
    app_core::school_browser::browsing_target(&url, site).map_err(coded)
}

#[tauri::command]
pub(crate) fn open_external(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    url: String,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    let safe = external_url(&url).ok_or_else(|| "This link cannot be opened.".to_string())?;
    Ok(app
        .opener()
        .open_url(safe, None::<&str>)
        .map_err(|_| "external URL could not be opened".to_string())?)
}

#[tauri::command]
pub(crate) fn configure_school_web_policy(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    policy: SchoolWebPolicy,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(controller.configure(&app, policy)?)
}

/// Shows `site`'s view for `surface`, the app page presenting it, entering
/// as `entry` says (Core decides what that means for the view).
#[tauri::command]
// Tauri supplies the first two parameters and the remaining fields preserve
// the established IPC command shape.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open_school_site(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    css: String,
    js: String,
    bounds: Option<WebBounds>,
    surface: String,
    entry: SchoolWebEntry,
) -> Result<SchoolWebResult, CommandError> {
    require_local_main(&webview)?;
    if css.len() > 256 * 1024 || js.len() > 256 * 1024 {
        return Err("school customization exceeds the safe size".into());
    }
    Ok(controller
        .open(site, css, js, bounds, &surface, entry)
        .await?)
}

/// The page the person sees of `site`. A surface reads it once its
/// `school-web-page` listener is registered and keeps the higher revision.
#[tauri::command]
pub(crate) fn school_web_page(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<SchoolWebPage, CommandError> {
    require_local_main(&webview)?;
    Ok(controller.engine.page(site))
}

#[tauri::command]
pub(crate) fn set_school_web_bounds(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    bounds: WebBounds,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(controller.set_bounds(bounds)?)
}

#[tauri::command]
pub(crate) fn hide_school_web(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
) -> Result<bool, CommandError> {
    if let Err(error) = require_local_main(&webview) {
        log::error!("hide_school_web rejected for {}: {error}", webview.label());
        return Err(error);
    }
    Ok(controller.hide_all()?)
}

#[tauri::command]
pub(crate) fn reload_school_web(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    controller.engine.reload(site).map_err(coded)
}

#[tauri::command]
pub(crate) fn navigate_school_web(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    url: String,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    controller.engine.navigate(site, &url).map_err(coded)
}

/// The app's Back for the school view (or the popup in front of it), through
/// the engine's history; Core keeps it from resending a form.
#[tauri::command]
pub(crate) fn school_web_back(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    controller.engine.back(site).map_err(coded)
}

/// The app's Forward, the counterpart of Back.
#[tauri::command]
pub(crate) fn school_web_forward(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    controller.engine.forward(site).map_err(coded)
}

/// Closes the popup in front of the school view.
#[tauri::command]
pub(crate) fn close_school_web_popup(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(controller.close_popup(site)?)
}

/// Shows a file a school page saved this session in Finder; other paths are
/// refused.
#[tauri::command]
pub(crate) fn reveal_school_download(
    webview: tauri::Webview,
    app: tauri::AppHandle,
    controller: tauri::State<'_, SchoolWebController>,
    path: String,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(controller.reveal_download(&app, &path)?)
}

#[tauri::command]
pub(crate) fn set_school_web_zoom(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    site: SchoolSite,
    scale: f64,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    Ok(controller.set_zoom(site, scale)?)
}

#[tauri::command]
pub(crate) fn set_kyomu_guide(
    webview: tauri::Webview,
    controller: tauri::State<'_, SchoolWebController>,
    items: Vec<kyomu_guide::GuideItem>,
) -> Result<bool, CommandError> {
    require_local_main(&webview)?;
    controller.engine.set_guide(items).map_err(coded)
}

#[cfg(test)]
#[path = "../tests/commands__school__external_url_tests.rs"]
mod external_url_tests;
