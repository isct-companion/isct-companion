//! The school views' cookie store as Core's write-only `WebCookieSeed`: Core
//! writes an account's verified identity cookies before a web operation
//! opens a school page and clears every school website, both accounts', when
//! an account goes.
//! Nothing here reads a cookie for Core, and nothing here decides: Core's
//! queue runs the calls one at a time, drops stale writes and bounds the
//! wait. A call answers once the platform confirmed it, however long that
//! takes.
//!
//! Every write goes through the platform's own `Set-Cookie` handling, never
//! Tauri's `Webview::set_cookie`: that builds the cookie from the `cookie`
//! crate, whose `domain()` drops a Domain attribute's leading dot, and a
//! dotless domain is host-only to WebKit, so `AUTH_SESSION_ID` for
//! `.titech.ac.jp` never reached 教務Web.
//!
//! - macOS: the school data store itself (`SCHOOL_DATA_STORE`), no view
//!   needed. `NSHTTPCookie cookiesWithResponseHeaderFields:forURL:` (WebKit's
//!   parser keeps the dot, the path, HttpOnly, Secure and SameSite) into its
//!   `WKHTTPCookieStore`, each write awaited through its completion handler;
//!   a clear removes the website data records (cookies, storage, caches) of
//!   every school domain.
//! - Windows: WebView2's `CookieManager`, with the leading dot of a Domain
//!   attribute put back. `AddOrUpdateCookie` has no completion; it applies
//!   in call order on the view's thread, before the navigation Core sends
//!   after it. A clear removes every school cookie.
//! - Linux: libsoup's `soup_cookie_parse` into WebKitGTK's cookie manager,
//!   each write awaited through its callback. A clear removes every school
//!   cookie.

use app_core::{
    net::BoxFuture,
    school_session::{is_school_domain, WebCookie, WebCookieSeed},
};
use tauri::AppHandle;
#[cfg(not(target_os = "macos"))]
use tauri::{
    webview::WebviewBuilder, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl,
};

/// The resident school views, which share the school store
/// (`school_web::school_profile`).
#[cfg(not(target_os = "macos"))]
const SCHOOL_VIEW_PREFIX: &str = "school-web-";
/// A view in the school store, made only when a clear finds no school view
/// in this process: cookies are reached through a view.
#[cfg(not(target_os = "macos"))]
const CLEAR_LABEL: &str = "school-cookie-clear";

pub(crate) struct DesktopWebCookies {
    app: AppHandle,
}

impl DesktopWebCookies {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }

    #[cfg(not(target_os = "macos"))]
    fn school_view(&self) -> Option<Webview> {
        self.app
            .webviews()
            .into_iter()
            .find(|(label, _)| label.starts_with(SCHOOL_VIEW_PREFIX))
            .map(|(_, view)| view)
    }
}

#[cfg(target_os = "macos")]
impl WebCookieSeed for DesktopWebCookies {
    fn write(&self, cookies: Vec<WebCookie>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            self.app
                .run_on_main_thread(move || native::write(&cookies, sender))
                .map_err(|error| error.to_string())?;
            receiver
                .await
                .map_err(|_| "the school store went away".to_string())?
        })
    }

    fn clear(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            self.app
                .run_on_main_thread(move || native::clear(sender))
                .map_err(|error| error.to_string())?;
            receiver
                .await
                .map_err(|_| "the school store went away".to_string())?
        })
    }
}

#[cfg(not(target_os = "macos"))]
impl WebCookieSeed for DesktopWebCookies {
    fn write(&self, cookies: Vec<WebCookie>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            // Core creates the site's view before it writes, so a school
            // view exists whenever a write is due.
            let view = self
                .school_view()
                .ok_or_else(|| "no school view holds the store".to_string())?;
            let (sender, receiver) = tokio::sync::oneshot::channel();
            view.with_webview(move |platform| native::write(platform, &cookies, sender))
                .map_err(|error| error.to_string())?;
            receiver
                .await
                .map_err(|_| "the school view went away".to_string())?
        })
    }

    fn clear(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
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

/// Deletes every school cookie of the school store, through a temporary
/// view when no school view exists.
#[cfg(not(target_os = "macos"))]
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
                .filter(|cookie| cookie.domain().is_some_and(is_school_domain))
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

#[cfg(not(target_os = "macos"))]
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

#[cfg(target_os = "macos")]
mod native {
    use super::{is_school_domain, WebCookie};
    use crate::school_browser_native::SCHOOL_DATA_STORE;
    use block2::RcBlock;
    use objc2::{rc::Retained, MainThreadMarker};
    use objc2_foundation::{NSArray, NSDictionary, NSHTTPCookie, NSString, NSURL, NSUUID};
    use objc2_web_kit::{WKWebsiteDataRecord, WKWebsiteDataStore};
    use std::{cell::RefCell, ptr::NonNull, rc::Rc};
    use tokio::sync::oneshot::Sender;

    type Done = Rc<RefCell<Option<Sender<Result<(), String>>>>>;

    fn answer(done: &Done, result: Result<(), String>) {
        if let Some(done) = done.borrow_mut().take() {
            let _ = done.send(result);
        }
    }

    /// The school views' persistent store (`data_store_identifier`).
    fn store(mtm: MainThreadMarker) -> Retained<WKWebsiteDataStore> {
        let identifier = NSUUID::from_bytes(SCHOOL_DATA_STORE);
        // SAFETY: on the main thread (`mtm`), with a non-zero identifier.
        unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&identifier, mtm) }
    }

    /// Parses each value with WebKit's own reader for its URL and hands it
    /// to the store's cookie store; answers once WebKit has called back for
    /// the last one. Runs on the main thread.
    pub(super) fn write(cookies: &[WebCookie], done: Sender<Result<(), String>>) {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = done.send(Err("the school store is reached on the main thread".into()));
            return;
        };
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
        if parsed.is_empty() {
            let _ = done.send(Ok(()));
            return;
        }
        // SAFETY: on the main thread, with the school store's own cookies.
        let cookie_store = unsafe { store(mtm).httpCookieStore() };
        let left = Rc::new(RefCell::new(parsed.len()));
        let done: Done = Rc::new(RefCell::new(Some(done)));
        for cookie in parsed {
            let left = left.clone();
            let done = done.clone();
            let handler = RcBlock::new(move || {
                let mut count = left.borrow_mut();
                *count -= 1;
                if *count == 0 {
                    answer(&done, Ok(()));
                }
            });
            // SAFETY: on the main thread, with the store's own cookie store.
            unsafe { cookie_store.setCookie_completionHandler(&cookie, Some(&handler)) };
        }
    }

    /// Removes the website data records (cookies, storage, caches) of every
    /// school domain and answers once WebKit has. Runs on the main thread.
    pub(super) fn clear(done: Sender<Result<(), String>>) {
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = done.send(Err("the school store is reached on the main thread".into()));
            return;
        };
        let store = store(mtm);
        // SAFETY: on the main thread.
        let types = unsafe { WKWebsiteDataStore::allWebsiteDataTypes(mtm) };
        let done: Done = Rc::new(RefCell::new(Some(done)));
        let fetched = {
            let store = store.clone();
            let types = types.clone();
            RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
                // SAFETY: WebKit hands a valid array for the call's duration.
                let records = unsafe { records.as_ref() };
                let chosen: Vec<Retained<WKWebsiteDataRecord>> = records
                    .iter()
                    // SAFETY: a record's display name is its registrable domain.
                    .filter(|record| is_school_domain(&unsafe { record.displayName() }.to_string()))
                    .collect();
                log::info!(
                    "[school-web] clearing {} school website records",
                    chosen.len()
                );
                if chosen.is_empty() {
                    answer(&done, Ok(()));
                    return;
                }
                let chosen = NSArray::from_retained_slice(&chosen);
                let removed = {
                    let done = done.clone();
                    RcBlock::new(move || answer(&done, Ok(())))
                };
                // SAFETY: on the main thread (WebKit calls back there), with
                // records this store fetched.
                unsafe {
                    store.removeDataOfTypes_forDataRecords_completionHandler(
                        &types, &chosen, &removed,
                    )
                };
            })
        };
        // SAFETY: on the main thread.
        unsafe { store.fetchDataRecordsOfTypes_completionHandler(&types, &fetched) };
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
