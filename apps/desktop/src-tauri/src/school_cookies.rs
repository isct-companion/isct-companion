//! The school views' cookie store as Core's write-only `WebCookieSeed`: Core
//! writes an account's identity cookies before an App-initiated school
//! navigation and clears the school cookies when an account goes. Nothing
//! here reads a cookie for Core.
//!
//! Every write goes through the platform's own `Set-Cookie` handling, never
//! Tauri's `Webview::set_cookie`: that builds the cookie from the `cookie`
//! crate, whose `domain()` drops a Domain attribute's leading dot, and a
//! dotless domain is host-only to WebKit, so `AUTH_SESSION_ID` for
//! `.titech.ac.jp` never reached 教務Web.
//!
//! - macOS: `NSHTTPCookie cookiesWithResponseHeaderFields:forURL:` (WebKit's
//!   parser keeps the dot, the path, HttpOnly, Secure and SameSite) into the
//!   view's `WKHTTPCookieStore`, each write awaited through its completion
//!   handler.
//! - Windows: WebView2's `CookieManager`, with the leading dot of a Domain
//!   attribute put back. `AddOrUpdateCookie` has no completion; it applies
//!   in call order on the view's thread, before the navigation Core sends
//!   after it.
//! - Linux: libsoup's `soup_cookie_parse` into WebKitGTK's cookie manager,
//!   each write awaited through its callback.
//!
//! Calls run one at a time (Core's queue), and a write of a generation
//! older than the last clear is dropped here too.

use app_core::{
    net::BoxFuture,
    school_session::{WebCookie, WebCookieSeed},
};
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    webview::WebviewBuilder, AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview,
    WebviewUrl,
};

/// The resident school views, which share the school store
/// (`school_web::school_profile`).
const SCHOOL_VIEW_PREFIX: &str = "school-web-";
/// A view in the school store, made only when a clear finds no school view
/// in this process: cookies are reached through a view.
const CLEAR_LABEL: &str = "school-cookie-clear";
/// The longest a platform may take to confirm one write or clear.
const CONFIRM: std::time::Duration = std::time::Duration::from_secs(10);

pub(crate) struct DesktopWebCookies {
    app: AppHandle,
    cleared: AtomicU64,
}

impl DesktopWebCookies {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self {
            app,
            cleared: AtomicU64::new(0),
        }
    }

    fn school_view(&self) -> Option<Webview> {
        self.app
            .webviews()
            .into_iter()
            .find(|(label, _)| label.starts_with(SCHOOL_VIEW_PREFIX))
            .map(|(_, view)| view)
    }
}

impl WebCookieSeed for DesktopWebCookies {
    fn write(&self, generation: u64, cookies: Vec<WebCookie>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if generation < self.cleared.load(Ordering::SeqCst) {
                return Ok(());
            }
            // Core creates the site's view before it writes, so a school
            // view exists whenever a write is due.
            let view = self
                .school_view()
                .ok_or_else(|| "no school view holds the store".to_string())?;
            let (sender, receiver) = tokio::sync::oneshot::channel();
            view.with_webview(move |platform| native::write(platform, &cookies, sender))
                .map_err(|error| error.to_string())?;
            tokio::time::timeout(CONFIRM, receiver)
                .await
                .map_err(|_| "the school store did not confirm the write".to_string())?
                .map_err(|_| "the school view went away".to_string())?
        })
    }

    fn clear(&self, generation: u64) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.cleared.fetch_max(generation, Ordering::SeqCst);
            let app = self.app.clone();
            let existing = self.school_view();
            // Tauri's cookie calls must not run on the main thread, which
            // WebView2 needs free to answer them.
            tauri::async_runtime::spawn_blocking(move || clear(&app, existing))
                .await
                .map_err(|error| error.to_string())?
        })
    }
}

/// Deletes the school store's cookies, through a temporary view when no
/// school view exists.
fn clear(app: &AppHandle, existing: Option<Webview>) -> Result<(), String> {
    let (view, temporary) = match existing {
        Some(view) => (view, false),
        None => (store_view(app)?, true),
    };
    let result = view
        .cookies()
        .map_err(|error| format!("School cookies are unreadable: {error}"))
        .and_then(|cookies| {
            for cookie in cookies
                .into_iter()
                .filter(|cookie| removable(cookie.domain()))
            {
                view.delete_cookie(cookie)
                    .map_err(|error| format!("A school cookie could not be removed: {error}"))?;
            }
            Ok(())
        });
    if temporary {
        let _ = view.close();
    }
    result
}

fn store_view(app: &AppHandle) -> Result<Webview, String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let blank = Url::parse("about:blank").map_err(|error| error.to_string())?;
    let builder = crate::school_web::school_profile(
        app,
        WebviewBuilder::new(CLEAR_LABEL, WebviewUrl::External(blank)),
    )?;
    let view = main
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| format!("School data store is unavailable: {error}"))?;
    view.hide().map_err(|error| error.to_string())?;
    Ok(view)
}

/// macOS and Windows give the school views their own data store, so every
/// cookie in it is a school session.
#[cfg(not(target_os = "linux"))]
fn removable(_domain: Option<&str>) -> bool {
    true
}

/// WebKitGTK views share the app's default store; only school sessions go.
#[cfg(target_os = "linux")]
fn removable(domain: Option<&str>) -> bool {
    domain.is_some_and(app_core::school_browser::is_school_cookie_domain)
}

#[cfg(target_os = "macos")]
mod native {
    use super::WebCookie;
    use block2::RcBlock;
    use objc2_foundation::{NSDictionary, NSHTTPCookie, NSString, NSURL};
    use objc2_web_kit::WKWebView;
    use std::{cell::RefCell, rc::Rc};
    use tokio::sync::oneshot::Sender;

    /// Parses each value with WebKit's own reader for its URL and hands it
    /// to the view's cookie store; answers once WebKit has called back for
    /// the last one. Runs on the main thread (`with_webview`).
    pub(super) fn write(
        platform: tauri::webview::PlatformWebview,
        cookies: &[WebCookie],
        done: Sender<Result<(), String>>,
    ) {
        let mut parsed = Vec::new();
        for cookie in cookies {
            let Some(url) = NSURL::URLWithString(&NSString::from_str(&cookie.url)) else {
                let _ = done.send(Err("a cookie URL is invalid".into()));
                return;
            };
            let key = NSString::from_str("Set-Cookie");
            let value = NSString::from_str(&cookie.set_cookie);
            let headers = NSDictionary::from_slices(&[&*key], &[&*value]);
            let found = NSHTTPCookie::cookiesWithResponseHeaderFields_forURL(&headers, &url);
            if found.is_empty() {
                let _ = done.send(Err("WebKit could not read a cookie".into()));
                return;
            }
            parsed.extend(found.iter());
        }
        let webview = platform.inner();
        if webview.is_null() {
            let _ = done.send(Err("the school view is not ready".into()));
            return;
        }
        if parsed.is_empty() {
            let _ = done.send(Ok(()));
            return;
        }
        // SAFETY: `inner()` is this view's live WKWebView, and `with_webview`
        // runs this on the main thread.
        let webview = unsafe { &*(webview as *const WKWebView) };
        let store = unsafe { webview.configuration().websiteDataStore().httpCookieStore() };
        let left = Rc::new(RefCell::new(parsed.len()));
        let done = Rc::new(RefCell::new(Some(done)));
        for cookie in parsed {
            let left = left.clone();
            let done = done.clone();
            let handler = RcBlock::new(move || {
                let mut count = left.borrow_mut();
                *count -= 1;
                if *count == 0 {
                    if let Some(done) = done.borrow_mut().take() {
                        let _ = done.send(Ok(()));
                    }
                }
            });
            // SAFETY: on the main thread, with the view's own store.
            unsafe { store.setCookie_completionHandler(&cookie, Some(&handler)) };
        }
    }
}

#[cfg(windows)]
mod native {
    use super::WebCookie;
    use tauri::webview::cookie::{Cookie, SameSite};
    use tokio::sync::oneshot::Sender;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_2, COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX,
        COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE, COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT,
    };
    use windows::core::{Interface, HSTRING};

    /// Writes every cookie into WebView2's cookie manager on the view's
    /// thread. `AddOrUpdateCookie` applies in call order on this thread,
    /// before the navigation Core sends next, so returning is the
    /// completion.
    pub(super) fn write(
        platform: tauri::webview::PlatformWebview,
        cookies: &[WebCookie],
        done: Sender<Result<(), String>>,
    ) {
        let _ = done.send(add(&platform, cookies));
    }

    fn add(
        platform: &tauri::webview::PlatformWebview,
        cookies: &[WebCookie],
    ) -> Result<(), String> {
        let error = |error: windows::core::Error| error.message().to_string();
        // SAFETY: WebView2 calls on the view's own (UI) thread, where
        // `with_webview` runs this.
        unsafe {
            let manager = platform
                .controller()
                .CoreWebView2()
                .map_err(error)?
                .cast::<ICoreWebView2_2>()
                .map_err(error)?
                .CookieManager()
                .map_err(error)?;
            for raw in cookies {
                let url = url::Url::parse(&raw.url).map_err(|error| error.to_string())?;
                let cookie =
                    Cookie::parse(raw.set_cookie.clone()).map_err(|error| error.to_string())?;
                // A Domain attribute keeps its leading dot, which WebView2
                // needs to reach the subdomains; without one it is host-only.
                let domain = match cookie.domain() {
                    Some(domain) => format!(".{}", domain.trim_start_matches('.')),
                    None => url.host_str().unwrap_or_default().to_owned(),
                };
                let path = cookie
                    .path()
                    .map(str::to_owned)
                    .unwrap_or_else(|| default_path(&url));
                let native = manager
                    .CreateCookie(
                        &HSTRING::from(cookie.name()),
                        &HSTRING::from(cookie.value()),
                        &HSTRING::from(domain),
                        &HSTRING::from(path),
                    )
                    .map_err(error)?;
                native
                    .SetIsHttpOnly(cookie.http_only().unwrap_or(false))
                    .map_err(error)?;
                native
                    .SetIsSecure(cookie.secure().unwrap_or(false))
                    .map_err(error)?;
                if let Some(same_site) = cookie.same_site() {
                    native
                        .SetSameSite(match same_site {
                            SameSite::Strict => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_STRICT,
                            SameSite::Lax => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_LAX,
                            SameSite::None => COREWEBVIEW2_COOKIE_SAME_SITE_KIND_NONE,
                        })
                        .map_err(error)?;
                }
                // Core sends a lifetime only as an absolute Expires.
                if let Some(at) = cookie.expires_datetime() {
                    native
                        .SetExpires(at.unix_timestamp() as f64)
                        .map_err(error)?;
                }
                manager.AddOrUpdateCookie(&native).map_err(error)?;
            }
        }
        Ok(())
    }

    /// RFC 6265 §5.1.4: the request path up to its last `/`, or `/`.
    fn default_path(url: &url::Url) -> String {
        match url.path().rfind('/') {
            Some(0) | None => "/".into(),
            Some(end) => url.path()[..end].to_owned(),
        }
    }
}

#[cfg(target_os = "linux")]
mod native {
    use super::WebCookie;
    use soup3::glib;
    use std::{cell::RefCell, rc::Rc};
    use tokio::sync::oneshot::Sender;
    use webkit2gtk::{gio, CookieManagerExt, WebContextExt, WebViewExt};

    /// Parses each value with libsoup for its URL, as WebKitGTK receives a
    /// response's cookies, adds it to the view's cookie manager, and answers
    /// once WebKitGTK has called back for the last one.
    pub(super) fn write(
        platform: tauri::webview::PlatformWebview,
        cookies: &[WebCookie],
        done: Sender<Result<(), String>>,
    ) {
        let Some(manager) = platform
            .inner()
            .context()
            .and_then(|context| context.cookie_manager())
        else {
            let _ = done.send(Err("the school store has no cookie manager".into()));
            return;
        };
        let mut parsed = Vec::new();
        for cookie in cookies {
            let Ok(origin) = glib::Uri::parse(&cookie.url, glib::UriFlags::NONE) else {
                let _ = done.send(Err("a cookie URL is invalid".into()));
                return;
            };
            let Some(cookie) = soup3::Cookie::parse(&cookie.set_cookie, Some(&origin)) else {
                let _ = done.send(Err("libsoup could not read a cookie".into()));
                return;
            };
            parsed.push(cookie);
        }
        if parsed.is_empty() {
            let _ = done.send(Ok(()));
            return;
        }
        let left = Rc::new(RefCell::new(parsed.len()));
        let done = Rc::new(RefCell::new(Some(done)));
        for mut cookie in parsed {
            let left = left.clone();
            let done = done.clone();
            manager.add_cookie(&mut cookie, None::<&gio::Cancellable>, move |result| {
                let failed = result.err().map(|error| error.to_string());
                let mut count = left.borrow_mut();
                *count -= 1;
                if failed.is_some() || *count == 0 {
                    if let Some(done) = done.borrow_mut().take() {
                        let _ = done.send(failed.map_or(Ok(()), Err));
                    }
                }
            });
        }
    }
}
