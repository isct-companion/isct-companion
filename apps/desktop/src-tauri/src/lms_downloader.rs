//! The school WebView store as Core's `SessionCookies` port: course files,
//! LMS and 教務Web requests all read and write the cookies of the views that
//! share `school_profile`, never the main window's store. Reads take what the
//! store holds, as on iOS and Android; signing in is Core's.
use app_core::{
    lms_session::{self, SchoolCookiePort},
    net::{BoxFuture, NetError, SessionCookies},
};
use std::sync::Arc;
use tauri::{webview::Cookie, AppHandle, Manager, Url, Webview};
/// Labels of the resident school views (`SchoolSite::webview_label`).
const SCHOOL_VIEW_PREFIX: &str = "school-web-";

struct NativeSession(AppHandle);
impl SessionCookies for NativeSession {
    fn cookie_header<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<String, NetError>> {
        // A store without the session answers SignInRequired; the caller's
        // transport then asks Core's school browser for a sign-in.
        Box::pin(async move { lms_session::cookie_header(self, url) })
    }
    fn store_set_cookies<'a>(
        &'a self,
        url: &'a Url,
        set_cookie: &'a [String],
    ) -> BoxFuture<'a, Result<(), NetError>> {
        Box::pin(async move { lms_session::store_set_cookies(self, url, set_cookie) })
    }
}
pub(crate) fn session(app: &AppHandle) -> Arc<dyn SessionCookies> {
    Arc::new(NativeSession(app.clone()))
}

fn unavailable(what: &str) -> NetError {
    NetError::Connect(format!("school view {what}"))
}

impl NativeSession {
    /// A view in the school store. They all share one cookie store, so any
    /// will do; none means no school sign-in has run in this process.
    fn store_view(&self) -> Result<Webview, NetError> {
        self.0
            .webviews()
            .into_iter()
            .find(|(label, _)| label.starts_with(SCHOOL_VIEW_PREFIX))
            .map(|(_, view)| view)
            .ok_or(NetError::SignInRequired)
    }
}

impl SchoolCookiePort for NativeSession {
    fn cookies(&self, url: &Url) -> Result<Vec<(String, String)>, NetError> {
        Ok(self
            .store_view()?
            .cookies_for_url(url.clone())
            .map_err(|_| unavailable("cookies are unreadable"))?
            .into_iter()
            .map(|cookie| (cookie.name().to_owned(), cookie.value().to_owned()))
            .collect())
    }
    fn set_cookies(&self, url: &Url, set_cookie: &[String]) -> Result<(), NetError> {
        let view = self.store_view()?;
        let host = url.host_str().unwrap_or_default().to_owned();
        for value in set_cookie {
            let Ok(mut cookie) = Cookie::parse(value.clone()) else {
                continue;
            };
            // WebKit and WebView2 need both; a host-only cookie gets the
            // response host, and a missing Path the RFC 6265 default path.
            if cookie.domain().is_none() {
                cookie.set_domain(host.clone());
            }
            if cookie.path().is_none() {
                cookie.set_path(default_path(url));
            }
            view.set_cookie(cookie)
                .map_err(|_| unavailable("cookie could not be stored"))?;
        }
        Ok(())
    }
}

/// RFC 6265 §5.1.4: the request path up to its last `/`, or `/`.
fn default_path(url: &Url) -> String {
    match url.path().rfind('/') {
        Some(0) | None => "/".into(),
        Some(end) => url.path()[..end].to_owned(),
    }
}
