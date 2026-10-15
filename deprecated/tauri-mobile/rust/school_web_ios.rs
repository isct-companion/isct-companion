//! wry's iOS set_bounds is a no-op, so UIKit sets the child's frame explicitly.
use super::NativeWebview;
use crate::lifecycle::SchoolSite;
use crate::school_browser_native::{
    ViewCallbacks, WebBounds, KYOMU_POPUP_SCRIPT, SCHOOL_DATA_STORE, SCHOOL_LINKS_SCRIPT,
};
use objc2::{rc::Retained, MainThreadMarker};
use objc2_ui_kit::{UIView, UIViewAutoresizing};
use std::{cell::RefCell, collections::HashMap, sync::mpsc, time::Duration};
use tauri::{AppHandle, Manager};
use url::Url;
use wry::{
    BackgroundThrottlingPolicy, NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder,
    WebViewBuilderExtDarwin, WebViewExtIOS,
};

struct ResidentView {
    view: WebView,
    shell: Retained<UIView>,
}
thread_local! { static VIEWS: RefCell<HashMap<SchoolSite, ResidentView>> = RefCell::new(HashMap::new()); }

fn on_ui<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    if MainThreadMarker::new().is_some() {
        return work();
    }
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(work());
    })
    .map_err(|error| error.to_string())?;
    rx.recv_timeout(Duration::from_secs(30))
        .map_err(|_| "School view main-thread operation timed out.".to_string())?
}

fn frame(resident: &ResidentView, bounds: WebBounds) -> Result<(), String> {
    let view = resident.view.webview();
    let parent = view.superview().ok_or("School view has no parent.")?;
    let scale = view
        .window()
        .ok_or("School view has no window.")?
        .screen()
        .scale();
    // CSS px -> device px -> UIKit points. Convert from the shell's actual
    // origin; it may already be inset, so never add safe-area padding twice.
    let mut rect = resident.shell.bounds();
    rect.origin.x = (bounds.x * scale).round() / scale;
    rect.origin.y = (bounds.y * scale).round() / scale;
    rect.size.width = (bounds.width * scale).round() / scale;
    rect.size.height = (bounds.height * scale).round() / scale;
    let mut rect = resident.shell.convertRect_toView(rect, Some(&parent));
    let safe = parent.safeAreaInsets();
    let extent = parent.bounds();
    let right = (rect.origin.x + rect.size.width).min(extent.size.width - safe.right);
    let bottom = (rect.origin.y + rect.size.height).min(extent.size.height - safe.bottom);
    rect.origin.x = rect.origin.x.max(safe.left);
    rect.origin.y = rect.origin.y.max(safe.top);
    rect.size.width = (right - rect.origin.x).max(0.0);
    rect.size.height = (bottom - rect.origin.y).max(0.0);
    view.setAutoresizingMask(UIViewAutoresizing::None);
    view.setFrame(rect);
    // Native sign-in controllers (notably Google Drive) can reorder UIKit's
    // view hierarchy while they dismiss. Reassert the school WKWebView's
    // z-order whenever its DOM host bounds are applied, so opening Web does
    // not require an app restart after cloud authorization.
    parent.bringSubviewToFront(&view);
    Ok(())
}

pub(super) fn create(
    app: &AppHandle,
    site: SchoolSite,
    target: &str,
    bounds: WebBounds,
    visible: bool,
    callbacks: ViewCallbacks,
) -> Result<NativeWebview, String> {
    let main = app
        .get_webview("main")
        .ok_or("Main webview is unavailable.")?;
    let window = main.window();
    let view_handle = NativeWebview {
        app: app.clone(),
        site,
    };
    let target = target.to_string();
    let (tx, rx) = mpsc::channel();
    // with_webview runs this closure on UIKit's main thread. A raw UIKit handle
    // is only borrowed here; resident views never cross threads.
    main.with_webview(move |platform| {
        let result: Result<(), String> = (|| {
            let shell = unsafe { Retained::retain(platform.inner().cast::<UIView>()) }
                .ok_or("Main UIKit view is unavailable.")?;
            let view = WebViewBuilder::new()
                .with_url(&target)
                .with_visible(visible)
                .with_data_store_identifier(SCHOOL_DATA_STORE)
                .with_background_throttling(BackgroundThrottlingPolicy::Disabled)
                .with_initialization_script_for_main_only(KYOMU_POPUP_SCRIPT, false)
                .with_initialization_script_for_main_only(SCHOOL_LINKS_SCRIPT, false)
                .with_on_page_load_handler(move |event, url| {
                    if matches!(event, PageLoadEvent::Finished) {
                        (callbacks.loaded)(&url);
                    }
                })
                .with_document_title_changed_handler(move |title| {
                    (callbacks.title)(&title);
                })
                .with_navigation_handler(move |raw| {
                    Url::parse(&raw).is_ok_and(|url| (callbacks.navigation)(&url))
                })
                .with_new_window_req_handler(move |raw, _| {
                    if let Ok(url) = Url::parse(&raw) {
                        (callbacks.new_window)(url);
                    }
                    NewWindowResponse::Deny
                })
                .build_as_child(&window)
                .map_err(|error| error.to_string())?;
            let resident = ResidentView { view, shell };
            if visible {
                frame(&resident, bounds)?;
            }
            VIEWS.with(|views| views.borrow_mut().insert(site, resident));
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|error| error.to_string())?;
    rx.recv_timeout(Duration::from_secs(30))
        .map_err(|_| "School view creation timed out.".to_string())??;
    Ok(view_handle)
}

pub(super) fn set_bounds(
    app: &AppHandle,
    site: SchoolSite,
    bounds: WebBounds,
) -> Result<(), String> {
    on_ui(app, move || {
        VIEWS.with(|views| {
            frame(
                views
                    .borrow()
                    .get(&site)
                    .ok_or("School view is unavailable.")?,
                bounds,
            )
        })
    })
}
pub(super) fn operate(
    app: &AppHandle,
    site: SchoolSite,
    operation: &str,
    value: Option<&str>,
) -> Result<(), String> {
    let operation = operation.to_string();
    let value = value.map(str::to_string);
    on_ui(app, move || {
        VIEWS.with(|views| {
            let mut views = views.borrow_mut();
            if operation == "close" {
                views.remove(&site);
                return Ok(());
            }
            let resident = views.get(&site).ok_or("School view is unavailable.")?;
            let view = &resident.view;
            match operation.as_str() {
                "eval" => view.evaluate_script(value.as_deref().unwrap_or_default()),
                "navigate" => view.load_url(value.as_deref().unwrap_or_default()),
                "reload" => view.reload(),
                "hide" => {
                    view.webview().endEditing(true);
                    view.set_visible(false)
                }
                "show" => view.set_visible(true),
                "focus" => view.focus(),
                "zoom" => view.zoom(
                    value
                        .as_deref()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(1.0),
                ),
                _ => return Err("Unknown school view operation.".into()),
            }
            .map_err(|error| error.to_string())
        })
    })
}
