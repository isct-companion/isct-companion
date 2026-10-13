//! Native session adapter for the shared Core course downloader.
use isct_core::{
    downloads::{DownloadSession, FetchError},
    lms_session::{LmsSessionPort, SessionState, LMS_HOME as HOME},
    school_parser::lms::is_lms_origin,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl,
};
const LABEL: &str = "lms-files";

#[derive(Default)]
pub struct SessionView {
    page: Mutex<Option<Url>>,
}

struct NativeSession(AppHandle);
impl DownloadSession for NativeSession {
    fn cookie_header<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>> {
        Box::pin(async move {
            crate::core_ports::runtime(&self.0)
                .lms_session
                .cookie_header(self, url)
                .await
        })
    }
}
pub(crate) fn session(app: &AppHandle) -> Arc<dyn DownloadSession> {
    Arc::new(NativeSession(app.clone()))
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

impl LmsSessionPort for NativeSession {
    fn school_state(&self) -> Result<SessionState, FetchError> {
        let snapshot = crate::core_ports::core(&self.0)
            .school_browser()
            .and_then(|engine| engine.session_snapshot())
            .map_err(|_| FetchError::Network)?;
        use isct_core::school_browser::SchoolSessionState;
        Ok(match snapshot.states.science {
            SchoolSessionState::Active => SessionState::Active,
            SchoolSessionState::Pending => SessionState::Pending,
            SchoolSessionState::Failed | SchoolSessionState::NotConfigured => SessionState::Failed,
        })
    }
    fn request_school_session(&self) -> Result<(), FetchError> {
        let engine = crate::core_ports::core(&self.0)
            .school_browser()
            .map_err(|_| FetchError::Network)?
            .clone();
        engine
            .request_session(isct_core::school_auth::SchoolAuthProfile::Science)
            .map(|_| ())
            .map_err(|_| FetchError::Network)
    }
    fn ensure_lms_view(&self) -> Result<(), FetchError> {
        if self.0.get_webview(LABEL).is_none() {
            create(&self.0).map_err(|_| FetchError::Network)?;
        }
        Ok(())
    }
    fn lms_ready(&self) -> Result<bool, FetchError> {
        let view = self.0.get_webview(LABEL).ok_or(FetchError::Network)?;
        let native = self.0.state::<SessionView>();
        let page = native.page.lock().map_err(|_| FetchError::Network)?;
        Ok(on_lms(&page) && has_lms_session(&view))
    }
    fn navigate_lms_home(&self) -> Result<(), FetchError> {
        self.0
            .get_webview(LABEL)
            .ok_or(FetchError::Network)?
            .navigate(Url::parse(HOME).map_err(|_| FetchError::Invalid)?)
            .map_err(|_| FetchError::Network)
    }
    fn cookies(&self, url: &Url) -> Result<String, FetchError> {
        let view = self.0.get_webview(LABEL).ok_or(FetchError::Network)?;
        cookie_header(&view, url)
    }
}

fn create(app: &AppHandle) -> Result<Webview, String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let builder = WebviewBuilder::new(
        LABEL,
        WebviewUrl::External(Url::parse(HOME).map_err(|error| error.to_string())?),
    );
    #[cfg(target_os = "macos")]
    let builder = builder.data_store_identifier(crate::school_web::SCHOOL_DATA_STORE);
    #[cfg(target_os = "windows")]
    let builder = builder.data_directory(
        app.path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("shared-webview-data"),
    );
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

fn cookie_header(view: &Webview, url: &Url) -> Result<String, FetchError> {
    let cookies = view
        .cookies_for_url(url.clone())
        .map_err(|_| FetchError::Network)?;
    let header = cookies
        .into_iter()
        .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
        .collect::<Vec<_>>()
        .join("; ");
    if header.is_empty() {
        Err(FetchError::Login)
    } else {
        Ok(header)
    }
}

fn is_login_url(url: &Url) -> bool {
    url.path_segments()
        .is_some_and(|mut segments| segments.any(|segment| segment == "login" || segment == "auth"))
}
