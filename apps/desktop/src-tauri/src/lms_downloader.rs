//! The school WebView store as Core's `SessionCookies` port: course files,
//! LMS and 教務Web requests all read and write the cookies of the views that
//! share `school_profile`, never the main window's store.
use isct_core::{
    lms_session::{SchoolCookiePort, SessionState, LMS_HOME as HOME},
    net::{BoxFuture, NetError, SessionCookies},
    school_browser::is_login_url,
    school_parser::lms::is_lms_origin,
};
use std::sync::{Arc, Mutex};
use tauri::{
    webview::{Cookie, PageLoadEvent, WebviewBuilder},
    AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl,
};
const LABEL: &str = "lms-files";
/// Labels of the resident school views (`SchoolSite::webview_label`).
const SCHOOL_VIEW_PREFIX: &str = "school-web-";

#[derive(Default)]
pub struct SessionView {
    page: Mutex<Option<Url>>,
}

struct NativeSession(AppHandle);
impl SessionCookies for NativeSession {
    fn cookie_header<'a>(&'a self, url: &'a Url) -> BoxFuture<'a, Result<String, NetError>> {
        Box::pin(async move {
            crate::core_ports::runtime(&self.0)
                .lms_session
                .cookie_header(self, url)
                .await
        })
    }
    fn store_set_cookies<'a>(
        &'a self,
        url: &'a Url,
        set_cookie: &'a [String],
    ) -> BoxFuture<'a, Result<(), NetError>> {
        Box::pin(async move {
            crate::core_ports::runtime(&self.0)
                .lms_session
                .store_set_cookies(self, url, set_cookie)
        })
    }
}
pub(crate) fn session(app: &AppHandle) -> Arc<dyn SessionCookies> {
    Arc::new(NativeSession(app.clone()))
}

fn unavailable(what: &str) -> NetError {
    NetError::Connect(format!("school view {what}"))
}

fn on_lms(page: &Option<Url>) -> bool {
    page.as_ref()
        .is_some_and(|url| is_lms_origin(url) && !is_login_url(url))
}

fn has_lms_session(view: &Webview) -> bool {
    let Ok(url) = Url::parse(HOME) else {
        return false;
    };
    view.cookies_for_url(url).is_ok_and(|cookies| {
        cookies
            .iter()
            .any(|cookie| cookie.name().eq_ignore_ascii_case("MoodleSession"))
    })
}

impl NativeSession {
    /// A view in the school store. They all share one cookie store, so any
    /// will do; none means no school sign-in has run in this process.
    fn store_view(&self) -> Result<Webview, NetError> {
        if let Some(view) = self.0.get_webview(LABEL) {
            return Ok(view);
        }
        self.0
            .webviews()
            .into_iter()
            .find(|(label, _)| label.starts_with(SCHOOL_VIEW_PREFIX))
            .map(|(_, view)| view)
            .ok_or(NetError::SignInRequired)
    }
}

impl SchoolCookiePort for NativeSession {
    fn school_state(&self) -> Result<SessionState, NetError> {
        let snapshot = crate::core_ports::core(&self.0)
            .school_browser()
            .and_then(|engine| engine.session_snapshot())
            .map_err(|_| unavailable("state is unavailable"))?;
        use isct_core::school_browser::SchoolSessionState;
        Ok(match snapshot.states.science {
            SchoolSessionState::Active => SessionState::Active,
            SchoolSessionState::Pending => SessionState::Pending,
            SchoolSessionState::Maintenance => return Err(unavailable("is under maintenance")),
            SchoolSessionState::Failed | SchoolSessionState::NotConfigured => SessionState::Failed,
        })
    }
    fn request_school_session(&self) -> Result<(), NetError> {
        let engine = crate::core_ports::core(&self.0)
            .school_browser()
            .map_err(|_| unavailable("engine is unavailable"))?
            .clone();
        engine
            .request_session(isct_core::school_auth::SchoolAuthProfile::Science)
            .map(|_| ())
            .map_err(|_| unavailable("sign-in could not start"))
    }
    fn ensure_lms_view(&self) -> Result<(), NetError> {
        if self.0.get_webview(LABEL).is_none() {
            create(&self.0).map_err(|_| unavailable("could not be created"))?;
        }
        Ok(())
    }
    fn lms_ready(&self) -> Result<bool, NetError> {
        let view = self
            .0
            .get_webview(LABEL)
            .ok_or_else(|| unavailable("is missing"))?;
        let native = self.0.state::<SessionView>();
        let page = native
            .page
            .lock()
            .map_err(|_| unavailable("is unavailable"))?;
        Ok(on_lms(&page) && has_lms_session(&view))
    }
    fn navigate_lms_home(&self) -> Result<(), NetError> {
        self.0
            .get_webview(LABEL)
            .ok_or_else(|| unavailable("is missing"))?
            .navigate(Url::parse(HOME).map_err(|_| NetError::Invalid("LMS home".into()))?)
            .map_err(|_| unavailable("could not navigate"))
    }
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

fn create(app: &AppHandle) -> Result<Webview, String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let builder = crate::school_web::school_profile(
        app,
        WebviewBuilder::new(
            LABEL,
            WebviewUrl::External(Url::parse(HOME).map_err(|error| error.to_string())?),
        ),
    )?;
    let loaded = app.clone();
    let builder = builder.on_page_load(move |_, payload| {
        if payload.event() == PageLoadEvent::Finished {
            if let Ok(mut page) = loaded.state::<SessionView>().page.lock() {
                *page = Some(payload.url().clone());
            }
        }
    });
    let view = main
        .add_child(
            builder,
            LogicalPosition::new(0.0, 0.0),
            LogicalSize::new(1.0, 1.0),
        )
        .map_err(|error| format!("Downloader could not be created: {error}"))?;
    view.hide().map_err(|error| error.to_string())?;
    Ok(view)
}
