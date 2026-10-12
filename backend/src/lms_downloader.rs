//! LMS bytes and lazy resource resolution use WebKit's signed-in session.
//! Requests are single-flight per target, small known files use four lanes,
//! and large/unknown files use an independent serial lane.
use crate::school_parser::lms::{is_lms_origin, is_plugin_file_url, parse_resource_page};
pub(crate) use crate::school_parser::lms::{ResolvedFile, ResolvedResource};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, COOKIE, RANGE};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl,
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Notify, Semaphore},
};

const LABEL: &str = "lms-files";
const HOME: &str = "https://lms.s.isct.ac.jp/2025/my/";
const SMALL_FILE: u64 = 2 * 1024 * 1024;
// The LMS load balancer rejects bare application user agents before Moodle
// sees the request. Keep the native client identifiable while using the
// browser-compatible form accepted by the same origin.
const USER_AGENT: &str = "Mozilla/5.0 (compatible; IsctCompanion/0.1)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched {
    Saved,
    TooLarge(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    Login,
    Invalid,
    Network,
    Timeout,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Login => "LMS sign-in required",
            Self::Invalid => "LMS resource left the allowlist",
            Self::Network => "LMS resource request failed",
            Self::Timeout => "LMS resource request timed out",
        })
    }
}

struct Flight {
    notify: Notify,
    result: Mutex<Option<Result<Fetched, FetchError>>>,
}

#[derive(Default)]
struct SmallPoolState {
    active: usize,
    foreground_waiters: usize,
}

#[derive(Default)]
struct SmallPool {
    state: Mutex<SmallPoolState>,
    changed: Notify,
}

struct SmallPermit<'a>(&'a SmallPool);

struct ForegroundWaiter<'a> {
    pool: &'a SmallPool,
    registered: bool,
}

impl Drop for ForegroundWaiter<'_> {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        if let Ok(mut state) = self.pool.state.lock() {
            state.foreground_waiters = state.foreground_waiters.saturating_sub(1);
        }
        self.pool.changed.notify_waiters();
    }
}

impl Drop for SmallPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.active = state.active.saturating_sub(1);
        }
        self.0.changed.notify_waiters();
    }
}

impl SmallPool {
    async fn acquire(&self, foreground: bool) -> Result<SmallPermit<'_>, FetchError> {
        let mut waiter = ForegroundWaiter {
            pool: self,
            registered: false,
        };
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock().map_err(|_| FetchError::Network)?;
                if state.active < 4 && (foreground || state.foreground_waiters == 0) {
                    state.active += 1;
                    if waiter.registered {
                        state.foreground_waiters = state.foreground_waiters.saturating_sub(1);
                        waiter.registered = false;
                    }
                    return Ok(SmallPermit(self));
                }
                if foreground && !waiter.registered {
                    state.foreground_waiters += 1;
                    waiter.registered = true;
                }
            }
            changed.await;
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> (usize, usize) {
        self.state
            .lock()
            .map(|state| (state.active, state.foreground_waiters))
            .unwrap_or_default()
    }
}

pub struct Downloader {
    page: Mutex<Option<Url>>,
    flights: tokio::sync::Mutex<HashMap<PathBuf, Arc<Flight>>>,
    source_locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    small: SmallPool,
    large: Semaphore,
}

impl Default for Downloader {
    fn default() -> Self {
        Self {
            page: Mutex::new(None),
            flights: tokio::sync::Mutex::new(HashMap::new()),
            source_locks: Mutex::new(HashMap::new()),
            small: SmallPool::default(),
            large: Semaphore::new(1),
        }
    }
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

fn uses_small_lane(size: u64) -> bool {
    size > 0 && size <= SMALL_FILE
}

impl Downloader {
    async fn flight_for(&self, target: &PathBuf) -> (Arc<Flight>, bool) {
        let mut flights = self.flights.lock().await;
        if let Some(flight) = flights.get(target) {
            (flight.clone(), false)
        } else {
            let flight = Arc::new(Flight {
                notify: Notify::new(),
                result: Mutex::new(None),
            });
            flights.insert(target.clone(), flight.clone());
            (flight, true)
        }
    }

    pub async fn fetch(
        &self,
        app: &AppHandle,
        url: &Url,
        target: PathBuf,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Fetched, FetchError> {
        if target.is_file() {
            return Ok(Fetched::Saved);
        }
        let (flight, leader) = self.flight_for(&target).await;
        if !leader {
            // Register the waiter before inspecting the result. `notify_waiters`
            // does not retain a permit, so doing this in the opposite order can
            // miss a completion between the check and `notified().await`.
            loop {
                let notified = flight.notify.notified();
                if let Some(result) = flight.result.lock().ok().and_then(|result| result.clone()) {
                    return result;
                }
                notified.await;
            }
        }
        let result = self
            .fetch_once(app, url, target.clone(), limit, foreground)
            .await;
        if let Ok(mut stored) = flight.result.lock() {
            *stored = Some(result.clone());
        }
        self.flights.lock().await.remove(&target);
        flight.notify.notify_waiters();
        result
    }

    async fn fetch_once(
        &self,
        app: &AppHandle,
        url: &Url,
        target: PathBuf,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Fetched, FetchError> {
        // Keep duplicate source URLs serial even when they map to different
        // course folders. This avoids repeated probes and downloads while
        // preserving the per-target single-flight result.
        let source_lock = {
            let mut locks = self.source_locks.lock().map_err(|_| FetchError::Network)?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(url.as_str()).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(url.to_string(), Arc::downgrade(&lock));
                lock
            }
        };
        let _source_turn = source_lock.lock().await;
        if target.is_file() {
            return Ok(Fetched::Saved);
        }
        let view = self.ready(app).await?;
        let size = self.probe(&view, url).await.map_err(|error| {
            eprintln!("[lms-file] probe failed: {error}");
            error
        })?;
        if limit.is_some_and(|limit| size > limit) {
            return Ok(Fetched::TooLarge(size));
        }
        let (_small_permit, _large_permit) = if uses_small_lane(size) {
            (Some(self.small.acquire(foreground).await?), None)
        } else {
            (
                None,
                Some(
                    self.large
                        .acquire()
                        .await
                        .map_err(|_| FetchError::Network)?,
                ),
            )
        };
        let mut part = target.clone().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        let outcome = tokio::time::timeout(
            Duration::from_secs(180),
            self.download_http(&view, url, &part, limit),
        )
        .await
        .unwrap_or(Err(FetchError::Timeout));
        match outcome {
            Ok(Fetched::Saved) => {}
            Ok(Fetched::TooLarge(bytes)) => {
                let _ = tokio::fs::remove_file(&part).await;
                return Ok(Fetched::TooLarge(bytes));
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(error);
            }
        }
        tokio::fs::rename(&part, &target)
            .await
            .map(|_| Fetched::Saved)
            .map_err(|_| FetchError::Network)
    }

    pub async fn resolve(
        &self,
        app: &AppHandle,
        url: &Url,
        cmid: u64,
        module: &str,
    ) -> Result<ResolvedResource, FetchError> {
        let view = self.ready(app).await?;
        let response = self.request(&view, url, None).await?;
        let final_url = checked_response_url(&response)?;
        if response.status().is_redirection() {
            return Err(FetchError::Login);
        }
        if !response.status().is_success() {
            return Err(FetchError::Network);
        }
        if is_login_url(&final_url) {
            return Err(FetchError::Login);
        }
        if is_plugin_file_url(&final_url) {
            return Ok(ResolvedResource::File {
                url: final_url.into(),
            });
        }
        if final_url.path() != url.path()
            || final_url
                .query_pairs()
                .find(|(key, _)| key == "id")
                .is_none_or(|(_, value)| value != cmid.to_string())
        {
            return Err(FetchError::Invalid);
        }
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/html"))
        {
            return Ok(ResolvedResource::Unresolved);
        }
        if response
            .content_length()
            .is_some_and(|bytes| bytes > 1_000_000)
        {
            return Ok(ResolvedResource::Unresolved);
        }
        let bytes = response.bytes().await.map_err(|_| FetchError::Network)?;
        if bytes.len() > 1_000_000 {
            return Ok(ResolvedResource::Unresolved);
        }
        let html = String::from_utf8_lossy(&bytes);
        Ok(parse_resource_page(&html, &final_url, module))
    }

    async fn probe(&self, view: &Webview, url: &Url) -> Result<u64, FetchError> {
        let response = self.request(view, url, Some("bytes=0-0")).await?;
        let final_url = checked_response_url(&response)?;
        if is_login_url(&final_url) {
            return Err(FetchError::Login);
        }
        if !is_plugin_file_url(&final_url) || !response.status().is_success() {
            return Err(FetchError::Invalid);
        }
        Ok(response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.rsplit('/').next())
            .and_then(|value| value.parse().ok())
            .or_else(|| {
                response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse().ok())
            })
            .or_else(|| response.content_length())
            // Moodle may stream otherwise valid files without either length
            // header. Treat that as unknown size: it uses the serial lane and
            // the download loop still enforces the caller's byte limit.
            .unwrap_or(0))
    }

    async fn request(
        &self,
        view: &Webview,
        url: &Url,
        range: Option<&str>,
    ) -> Result<reqwest::Response, FetchError> {
        let cookie = cookie_header(view, url)?;
        let client = http_client()?;
        let mut request = client.get(url.as_str()).header(COOKIE, cookie);
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        tokio::time::timeout(Duration::from_secs(30), request.send())
            .await
            .map_err(|_| FetchError::Timeout)?
            .map_err(|_| FetchError::Network)
    }

    async fn download_http(
        &self,
        view: &Webview,
        url: &Url,
        target: &PathBuf,
        limit: Option<u64>,
    ) -> Result<Fetched, FetchError> {
        let mut response = self.request(view, url, None).await?;
        let final_url = checked_response_url(&response)?;
        if is_login_url(&final_url) {
            return Err(FetchError::Login);
        }
        if !is_plugin_file_url(&final_url) || !response.status().is_success() {
            return Err(FetchError::Invalid);
        }
        let mut file = tokio::fs::File::create(target)
            .await
            .map_err(|_| FetchError::Network)?;
        let mut written = 0_u64;
        while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Network)? {
            written = written.saturating_add(chunk.len() as u64);
            if limit.is_some_and(|limit| written > limit) {
                return Ok(Fetched::TooLarge(written));
            }
            file.write_all(&chunk)
                .await
                .map_err(|_| FetchError::Network)?;
        }
        file.flush().await.map_err(|_| FetchError::Network)?;
        Ok(Fetched::Saved)
    }

    async fn ready(&self, app: &AppHandle) -> Result<Webview, FetchError> {
        let school = app.state::<crate::school_web::SchoolWebController>();
        school
            .wait_for_session(app, crate::school_auth::SchoolAuthProfile::Science)
            .await
            .map_err(|_| FetchError::Login)?;
        let view = match app.get_webview(LABEL) {
            Some(view) => view,
            None => create(app).map_err(|_| FetchError::Network)?,
        };
        for attempt in 0..2 {
            for _ in 0..100 {
                if self.page.lock().is_ok_and(|page| on_lms(&page)) && has_lms_session(&view) {
                    return Ok(view);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            if attempt == 0 {
                let _ = school.request_session(app, crate::school_auth::SchoolAuthProfile::Science);
                tokio::time::sleep(Duration::from_secs(5)).await;
                view.navigate(Url::parse(HOME).expect("valid LMS URL"))
                    .map_err(|_| FetchError::Network)?;
            }
        }
        Err(FetchError::Login)
    }
}

fn create(app: &AppHandle) -> Result<Webview, String> {
    let main = app.get_window("main").ok_or("Main window is unavailable")?;
    let builder = WebviewBuilder::new(
        LABEL,
        WebviewUrl::External(Url::parse(HOME).expect("valid LMS URL")),
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
            if let Ok(mut page) = loaded.state::<Downloader>().page.lock() {
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

fn http_client() -> Result<reqwest::Client, FetchError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let allowed = is_lms_origin(attempt.url());
            if !allowed {
                return attempt.stop();
            }
            if attempt.previous().len() >= 5 {
                attempt.error("too many LMS redirects")
            } else {
                attempt.follow()
            }
        }))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| FetchError::Network)
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

fn checked_response_url(response: &reqwest::Response) -> Result<Url, FetchError> {
    let url = Url::parse(response.url().as_str()).map_err(|_| FetchError::Invalid)?;
    if !is_lms_origin(&url) {
        return Err(FetchError::Invalid);
    }
    Ok(url)
}

fn is_login_url(url: &Url) -> bool {
    url.path_segments()
        .is_some_and(|mut segments| segments.any(|segment| segment == "login" || segment == "auth"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pool_sizes_match_the_resource_policy() {
        let downloader = Downloader::default();
        assert_eq!(SMALL_FILE, 2 * 1024 * 1024);
        assert_eq!(downloader.small.snapshot(), (0, 0));
        assert_eq!(downloader.large.available_permits(), 1);
        assert!(!uses_small_lane(0));
        assert!(uses_small_lane(SMALL_FILE));
        assert!(!uses_small_lane(SMALL_FILE + 1));
    }

    #[test]
    fn concurrent_target_requests_share_one_flight() {
        tauri::async_runtime::block_on(async {
            let downloader = Downloader::default();
            let target = PathBuf::from("synthetic.pdf");
            let (first, leader) = downloader.flight_for(&target).await;
            let (second, second_leader) = downloader.flight_for(&target).await;
            assert!(leader);
            assert!(!second_leader);
            assert!(Arc::ptr_eq(&first, &second));
        });
    }

    #[test]
    fn allowlist_is_the_exact_https_lms_origin_not_an_academic_year_path() {
        for raw in [
            "https://lms.s.isct.ac.jp/",
            "https://lms.s.isct.ac.jp/2024/pluginfile.php/1/a.pdf",
            "https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42",
        ] {
            assert!(is_lms_origin(&Url::parse(raw).unwrap()), "{raw}");
        }
        for raw in [
            "http://lms.s.isct.ac.jp/2026/pluginfile.php/1/a.pdf",
            "https://evil.lms.s.isct.ac.jp/2026/pluginfile.php/1/a.pdf",
            "https://lms.s.isct.ac.jp:8443/2026/pluginfile.php/1/a.pdf",
        ] {
            assert!(!is_lms_origin(&Url::parse(raw).unwrap()), "{raw}");
        }
    }
}
