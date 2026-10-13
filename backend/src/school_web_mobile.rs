//! Mobile native-view adapter for the shared Rust school browser controller.
//! iOS views live on UIKit's main thread; Android views live in the Kotlin plugin.
use crate::lifecycle::SchoolSite;
use crate::school_browser_native::ViewCallbacks;
pub use crate::school_browser_native::{
    IngestOutcome, SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds,
};
use tauri::AppHandle;
use url::Url;

#[cfg(target_os = "ios")]
#[path = "school_web_ios.rs"]
mod platform;
#[cfg(target_os = "android")]
#[path = "school_web_android.rs"]
mod platform;

#[derive(Clone)]
pub(crate) struct NativeWebview {
    app: AppHandle,
    site: SchoolSite,
}
impl NativeWebview {
    pub(crate) fn close(&self) -> Result<(), String> {
        platform::operate(&self.app, self.site, "close", None)
    }
    pub(crate) fn eval(&self, script: &str) -> Result<(), String> {
        platform::operate(&self.app, self.site, "eval", Some(script))
    }
    pub(crate) fn navigate(&self, url: Url) -> Result<(), String> {
        platform::operate(&self.app, self.site, "navigate", Some(url.as_str()))
    }
    pub(crate) fn reload(&self) -> Result<(), String> {
        platform::operate(&self.app, self.site, "reload", None)
    }
    pub(crate) fn hide(&self) -> Result<(), String> {
        platform::operate(&self.app, self.site, "hide", None)
    }
    pub(crate) fn show(&self) -> Result<(), String> {
        platform::operate(&self.app, self.site, "show", None)
    }
    pub(crate) fn set_focus(&self) -> Result<(), String> {
        platform::operate(&self.app, self.site, "focus", None)
    }
    pub(crate) fn set_zoom(&self, scale: f64) -> Result<(), String> {
        platform::operate(&self.app, self.site, "zoom", Some(&scale.to_string()))
    }
}
pub(crate) fn apply_bounds(view: &NativeWebview, bounds: WebBounds) -> Result<(), String> {
    platform::set_bounds(&view.app, view.site, bounds.sanitized())
}
pub(crate) fn create_webview(
    app: &AppHandle,
    site: SchoolSite,
    target: &str,
    bounds: WebBounds,
    visible: bool,
    callbacks: ViewCallbacks,
) -> Result<NativeWebview, String> {
    platform::create(app, site, target, bounds.sanitized(), visible, callbacks)
}
