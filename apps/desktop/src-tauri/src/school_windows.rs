//! Windows interim school-view host, until a WebView2 host takes the
//! engine's own events. wry passes only a URL, so every request reaches Core
//! as `NavigationKind::Unknown` with no method or gesture, and Core reloads
//! with a GET (`HostCapabilities`). Popups are small separate windows Tauri
//! creates in the opener's WebView2 environment (`NewWindowResponse::Create`),
//! so `window.opener` and a posted form survive.
//!
//! A temporary exception to the history every other host reports: this one
//! reports none, so the page's Back and Forward stay `unknown` and Core asks
//! the engine anyway. It ends when `school_webview2.rs` reports history
//! (`approach/school-web-windows-host-later`).
use crate::school_browser_native::{NativeBrowser, PopupRecord};
use app_core::school_browser::{
    BrowserAction, BrowserId, NavigationDecision, NavigationKind, NavigationRequest,
    NewWindowDecision, NewWindowRequest, WindowFeatures,
};
use std::sync::{atomic::AtomicU64, Mutex, Weak};
use tauri::webview::{NewWindowFeatures, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, WebviewUrl, WebviewWindow, Wry};
use tauri_plugin_opener::OpenerExt;
use url::Url;

/// What a main view needs to put its requests to Core.
pub(crate) struct Interim {
    pub(crate) native: Weak<NativeBrowser>,
    pub(crate) view: BrowserId,
}

impl Interim {
    pub(crate) fn attach(
        self,
        app: &AppHandle,
        builder: WebviewBuilder<Wry>,
    ) -> WebviewBuilder<Wry> {
        let Self { native, view } = self;
        let navigation = native.clone();
        let app = app.clone();
        builder
            .on_navigation(move |url| allowed(&navigation, view, url))
            .on_new_window(move |url, features| open(&app, &native, view, url, features))
    }
}

/// Core's decision for a main-frame navigation of a view or popup.
fn allowed(native: &Weak<NativeBrowser>, view: BrowserId, url: &Url) -> bool {
    let Some(native) = native.upgrade() else {
        return false;
    };
    let (Some(page), Some(listener)) = (native.current_page(view), native.listener()) else {
        return false;
    };
    match listener.decide_navigation(NavigationRequest {
        page,
        url: url.clone(),
        method: None,
        main_frame: true,
        kind: NavigationKind::Unknown,
        user_gesture: false,
    }) {
        NavigationDecision::Allow => true,
        NavigationDecision::External(url) => {
            if let Ok(app) = native.app() {
                let _ = app.opener().open_url(url.as_str(), None::<&str>);
            }
            false
        }
        // WebView2 does not resend forms through history (unverified), and
        // reports no resubmission to skip.
        NavigationDecision::Deny(_) | NavigationDecision::SkipEntry => false,
    }
}

fn open(
    app: &AppHandle,
    weak: &Weak<NativeBrowser>,
    opener: BrowserId,
    url: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    let Some(native) = weak.upgrade() else {
        return NewWindowResponse::Deny;
    };
    let (Some(page), Some(listener)) = (native.current_page(opener), native.listener()) else {
        return NewWindowResponse::Deny;
    };
    let size = features.size();
    let decision = listener.new_window(NewWindowRequest {
        opener: page,
        url: url.clone(),
        method: None,
        kind: NavigationKind::Unknown,
        user_gesture: false,
        name: None,
        features: WindowFeatures {
            width: size.map(|size| size.width),
            height: size.map(|size| size.height),
        },
    });
    match decision {
        NewWindowDecision::Popup(id) => match popup_window(app, weak, id, &url, features) {
            Ok(window) => {
                if let Ok(mut p) = native.presentation.lock() {
                    p.popups.insert(
                        id,
                        PopupRecord {
                            page: AtomicU64::new(0),
                            url: Mutex::new(None),
                            window: Some(window.clone()),
                        },
                    );
                }
                let gone = weak.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Destroyed) {
                        if let Some(native) = gone.upgrade() {
                            native.popup_gone(id);
                        }
                    }
                });
                NewWindowResponse::Create { window }
            }
            Err(error) => {
                log::error!("[school-web] popup window could not be created: {error}");
                listener.closed(id);
                NewWindowResponse::Deny
            }
        },
        NewWindowDecision::External(url) => {
            let _ = app.opener().open_url(url.as_str(), None::<&str>);
            NewWindowResponse::Deny
        }
        NewWindowDecision::Deny(_) => NewWindowResponse::Deny,
    }
}

fn popup_window(
    app: &AppHandle,
    weak: &Weak<NativeBrowser>,
    id: BrowserId,
    url: &Url,
    features: NewWindowFeatures,
) -> tauri::Result<WebviewWindow> {
    let loaded = weak.clone();
    let navigation = weak.clone();
    let nested = weak.clone();
    let nested_app = app.clone();
    tauri::WebviewWindowBuilder::new(
        app,
        format!("school-popup-{}", id.generation),
        // As in Tauri's own example: WebView2 loads the request it already
        // started into this window, not this URL.
        WebviewUrl::External(Url::parse("about:blank").map_err(tauri::Error::InvalidUrl)?),
    )
    // The opener's environment and profile: one cookie store, and the
    // posted form and `window.opener` carry over.
    .window_features(features)
    .title(url.host_str().unwrap_or_default())
    .on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            if let Some(native) = loaded.upgrade() {
                native.popup_loaded(id, payload.url().clone());
            }
        }
    })
    .on_navigation(move |url| allowed(&navigation, id, url))
    .on_new_window(move |url, features| open(&nested_app, &nested, id, url, features))
    .build()
}

pub(crate) fn popup_command(window: &WebviewWindow, command: BrowserAction) -> Result<(), String> {
    match command {
        BrowserAction::Navigate(url) => window.navigate(url),
        BrowserAction::Reload => window.reload(),
        BrowserAction::GoBack => window.eval("history.back()"),
        BrowserAction::GoForward => window.eval("history.forward()"),
        BrowserAction::Eval(script) => window.eval(script),
        // Core stops only the views its hidden work runs in.
        BrowserAction::Stop => Ok(()),
    }
    .map_err(|error| error.to_string())
}
