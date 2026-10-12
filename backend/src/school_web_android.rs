//! Kotlin owns Android school views; the Rust callback channel owns all relays.
use super::NativeWebview;
use crate::lifecycle::SchoolSite;
use crate::school_web_shared::{ViewCallbacks, WebBounds, KYOMU_POPUP_SCRIPT, SCHOOL_LINKS_SCRIPT};
use serde::Serialize;
use tauri::{
    ipc::{Channel, InvokeResponseBody},
    AppHandle,
};
use url::Url;

pub(super) fn create(
    app: &AppHandle,
    site: SchoolSite,
    target: &str,
    bounds: WebBounds,
    visible: bool,
    callbacks: ViewCallbacks,
) -> Result<NativeWebview, String> {
    let view = NativeWebview {
        app: app.clone(),
        site,
    };
    let event_view = view.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let events = Channel::<serde_json::Value>::new(move |body| {
        let _ = tx.send(body);
        Ok(())
    });
    std::thread::spawn(move || {
        for body in rx {
            if let InvokeResponseBody::Json(raw) = body {
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(&raw) {
                    let value = event["value"].as_str().unwrap_or_default();
                    match event["kind"].as_str() {
                        Some("loaded") => (callbacks.loaded)(&event_view, value),
                        Some("title") => (callbacks.title)(value),
                        Some("navigation") => {
                            if let Ok(url) = Url::parse(value) {
                                (callbacks.navigation)(&url);
                            }
                        }
                        Some("newWindow") => {
                            if let Ok(url) = Url::parse(value) {
                                (callbacks.new_window)(url);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Create<'a> {
        operation: &'static str,
        site: SchoolSite,
        url: &'a str,
        bounds: WebBounds,
        visible: bool,
        scripts: [String; 1],
        events: Channel<serde_json::Value>,
        allowed_hosts: &'static [&'static str],
    }
    tauri_plugin_school_web::command(
        app,
        Create {
            operation: "create",
            site,
            url: target,
            bounds,
            visible,
            scripts: [
                include_str!("../scripts/android-school-support.js").replace(
                    "__ISCT_SUPPORT_SCRIPTS__",
                    &serde_json::to_string(&[KYOMU_POPUP_SCRIPT, SCHOOL_LINKS_SCRIPT])
                        .map_err(|error| error.to_string())?,
                ),
            ],
            events,
            allowed_hosts: crate::school_extract::SCHOOL_HOSTS,
        },
    )?;
    Ok(view)
}
pub(super) fn set_bounds(
    app: &AppHandle,
    site: SchoolSite,
    bounds: WebBounds,
) -> Result<(), String> {
    tauri_plugin_school_web::command(
        app,
        serde_json::json!({ "operation": "bounds", "site": site, "bounds": bounds }),
    )
    .map(|_| ())
}
pub(super) fn operate(
    app: &AppHandle,
    site: SchoolSite,
    operation: &str,
    value: Option<&str>,
) -> Result<(), String> {
    tauri_plugin_school_web::command(
        app,
        serde_json::json!({ "operation": operation, "site": site, "value": value }),
    )
    .map(|_| ())
}
