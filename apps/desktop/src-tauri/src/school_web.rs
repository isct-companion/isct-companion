//! Desktop resident school views: the pages the person opens. Signing in is
//! Core's, over its own HTTP sessions; the views only receive the identity
//! cookies Core writes (`school_cookies`).
use crate::lifecycle::SchoolSite;
use crate::school_browser_native::ViewCallbacks;
#[cfg(target_os = "macos")]
use crate::school_browser_native::SCHOOL_DATA_STORE;
pub use crate::school_browser_native::{
    SchoolWebController, SchoolWebPolicy, SchoolWebResult, WebBounds,
};
use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl};
use url::Url;
pub(crate) type NativeWebview = tauri::Webview;

pub(crate) fn apply_bounds(webview: &NativeWebview, bounds: WebBounds) -> Result<(), String> {
    let bounds = bounds.sanitized();
    webview
        .set_position(LogicalPosition::new(bounds.x, bounds.y))
        .and_then(|_| webview.set_size(LogicalSize::new(bounds.width, bounds.height)))
        .map_err(|_| "School view bounds could not be updated.".to_string())
}

/// The browser profile every school webview uses: one cookie store, so the
/// identity Core writes reaches every school view.
pub(crate) fn school_profile<R: tauri::Runtime>(
    app: &AppHandle<R>,
    builder: WebviewBuilder<R>,
) -> Result<WebviewBuilder<R>, String> {
    // Every view in this window leaves OS file drops to the browser's native
    // upload handling.
    let builder = builder.disable_drag_drop_handler();
    #[cfg(target_os = "macos")]
    let builder = builder.data_store_identifier(SCHOOL_DATA_STORE);
    // Views sharing a data directory share one WebView2 environment and must
    // pass identical arguments; the switch keeps the features wry disables by
    // default.
    #[cfg(target_os = "windows")]
    let builder = builder
        .data_directory(
            app.path()
                .app_data_dir()
                .map_err(|_| "School web data path is unavailable.")?
                .join("shared-webview-data"),
        )
        .additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    #[cfg(not(target_os = "windows"))]
    let _ = app;
    Ok(builder)
}

/// Steps a view through the engine's own history.
pub(crate) fn go(webview: &NativeWebview, back: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use crate::school_webkit::Command;
        let command = if back {
            Command::GoBack
        } else {
            Command::GoForward
        };
        webview
            .with_webview(move |platform| {
                // SAFETY: `inner()` is this view's live WKWebView, and
                // `with_webview` invokes this closure on the main thread.
                unsafe { crate::school_webkit::command(platform.inner(), command) }
            })
            .map_err(|error| error.to_string())
    }
    // Until the WebView2 host exists, the page's own history API.
    #[cfg(not(target_os = "macos"))]
    webview
        .eval(if back {
            "history.back()"
        } else {
            "history.forward()"
        })
        .map_err(|error| error.to_string())
}

/// A resident school view. School pages get no script of the app's: only
/// Core's own page scripts, which Core evaluates itself. The
/// platform host decides navigation and windows (`school_webkit.rs`; the
/// Windows interim's wry callbacks in `school_windows.rs`).
pub(crate) fn create_webview(
    app: &AppHandle,
    site: SchoolSite,
    target: &str,
    bounds: WebBounds,
    visible: bool,
    callbacks: ViewCallbacks,
    #[cfg(windows)] interim: crate::school_windows::Interim,
) -> Result<NativeWebview, String> {
    let main = app
        .get_window("main")
        .ok_or("Main window is unavailable.")?;
    let start = Url::parse(target).map_err(|_| "School URL is invalid.")?;
    let builder = school_profile(
        app,
        WebviewBuilder::new(site.webview_label(), WebviewUrl::External(start)),
    )?
    .on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            (callbacks.loaded)(payload.url().as_str());
        }
    })
    .on_download(move |_, event| (callbacks.download)(event));
    #[cfg(windows)]
    let builder = interim.attach(app, builder);
    let position = if visible {
        LogicalPosition::new(bounds.x, bounds.y)
    } else {
        LogicalPosition::new(0.0, 0.0)
    };
    let size = if visible {
        LogicalSize::new(bounds.width, bounds.height)
    } else {
        LogicalSize::new(1.0, 1.0)
    };
    main.add_child(builder, position, size)
        .map_err(|error| error.to_string())
}
