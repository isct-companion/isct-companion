//! Authenticated LMS resource resolution and bounded file downloads, and the
//! school files a page hands over (`save_school_file`).

use crate::{
    net::{NetError, SessionCookies, SignInGate, USER_AGENT},
    runtime_tasks::RuntimeTasks,
    school_auth::SchoolAuthProfile,
    school_browser::{browsing_url, is_login_url, school_file_url},
    school_files::{self, SchoolFile, SchoolFileRequest},
};
pub use adpt_isct::lms::ResolvedResource;
use adpt_isct::lms::{is_lms_origin, is_plugin_file_url, parse_resource_page};
use reqwest::header::{
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, LOCATION, RANGE,
    REFERER, SET_COOKIE,
};
use std::{
    collections::HashMap,
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::Duration,
};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::{watch, Notify, Semaphore},
};
use url::Url;

const SMALL_FILE: u64 = 2 * 1024 * 1024;
const SESSION_TIMEOUT: Duration = Duration::from_secs(230);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const RESOURCE_TIMEOUT: Duration = Duration::from_secs(260);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_RESOURCE_PAGE: u64 = 1_000_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Fetched {
    Saved,
    TooLarge(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FetchError {
    #[error("LMS sign-in required")]
    Login,
    #[error("LMS resource left the allowlist")]
    Invalid,
    #[error("LMS resource request failed")]
    Network,
    #[error("LMS resource request timed out")]
    Timeout,
    #[error("LMS resource request was cancelled")]
    Cancelled,
    #[error("{}", crate::storage::WRITE_DENIED)]
    WriteDenied,
}

/// What a failed cookie read means for a download. Cookie-store failures stay
/// network errors so they never ask the user to sign in again.
fn session_error(error: NetError) -> FetchError {
    match error {
        NetError::SignInRequired => FetchError::Login,
        NetError::Invalid(_) => FetchError::Invalid,
        NetError::Stalled | NetError::Deadline => FetchError::Timeout,
        NetError::Cancelled => FetchError::Cancelled,
        _ => FetchError::Network,
    }
}

/// The course folder refused the file; any other local failure stays a
/// failed request the user can retry.
fn write_error(error: std::io::Error) -> FetchError {
    if crate::storage::denied(&error) {
        FetchError::WriteDenied
    } else {
        FetchError::Network
    }
}

type FetchFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FetchError>> + Send + 'a>>;

/// Which URLs a request may reach, checked on every redirect hop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    /// Course files: the LMS only, always with its session.
    Lms,
    /// Files a school page hands over: any school host, with the session's
    /// cookies when the store holds some (public pages need none).
    School,
}

impl Scope {
    fn admits(self, url: &Url) -> bool {
        match self {
            Scope::Lms => is_lms_origin(url),
            Scope::School => school_file_url(url.as_str()).is_some(),
        }
    }

    /// A sign-in page instead of the file: the session is gone. A school
    /// file is never served by an identity provider, so any page there
    /// (GetAccess's resource list after a 教務Web timeout) counts.
    fn signed_out(self, url: &Url) -> bool {
        is_login_url(url)
            || (self == Scope::School
                && url
                    .host_str()
                    .is_some_and(|host| SchoolAuthProfile::for_auth_host(host).is_some()))
    }
}

/// What the school answered for a file Core streamed into the staging file.
#[derive(Clone, Debug, Default)]
struct SchoolResponse {
    url: Option<Url>,
    disposition: Option<Vec<u8>>,
    content_type: Option<String>,
}

trait DownloadTransport: Send + Sync {
    fn probe<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, u64>;

    fn download<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        file: File,
        limit: Option<u64>,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, Fetched>;

    fn resolve<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        cmid: u64,
        module: &'a str,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, ResolvedResource>;

    /// Streams the school file at `url` into `file`, whatever its size.
    fn school_file<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        referer: Option<&'a Url>,
        file: File,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, SchoolResponse>;
}

/// One client per host, each built with the TLS its host needs
/// (`net::host_client_builder`): 教務Web files take its vendored intermediate.
#[derive(Default)]
struct HttpTransport {
    clients: Mutex<HashMap<String, reqwest::Client>>,
}

impl HttpTransport {
    fn client(&self, url: &Url) -> Result<reqwest::Client, FetchError> {
        let host = url.host_str().ok_or(FetchError::Invalid)?;
        let mut clients = self.clients.lock().map_err(|_| FetchError::Network)?;
        if let Some(client) = clients.get(host) {
            return Ok(client.clone());
        }
        let client = crate::net::host_client_builder(host)
            .map_err(|_| FetchError::Network)?
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| FetchError::Network)?;
        clients.insert(host.to_owned(), client.clone());
        Ok(client)
    }

    async fn request(
        &self,
        session: Arc<dyn SessionCookies>,
        url: &Url,
        scope: Scope,
        referer: Option<&Url>,
        range: Option<&str>,
        tasks: &RuntimeTasks,
    ) -> Result<reqwest::Response, FetchError> {
        if !scope.admits(url) {
            return Err(FetchError::Invalid);
        }
        let mut current = url.clone();
        for _ in 0..=5 {
            let cookie = tokio::select! {
                _ = tasks.closed() => return Err(FetchError::Cancelled),
                cookie = tokio::time::timeout(SESSION_TIMEOUT, session.cookie_header(&current)) => {
                    match (cookie.map_err(|_| FetchError::Timeout)?, scope) {
                        (Ok(cookie), _) => cookie,
                        // A public school page needs no session.
                        (Err(NetError::SignInRequired), Scope::School) => String::new(),
                        (Err(error), _) => return Err(session_error(error)),
                    }
                },
            };
            if cookie.trim().is_empty() && scope == Scope::Lms {
                return Err(FetchError::Login);
            }
            let mut request = self.client(&current)?.get(current.as_str());
            if !cookie.trim().is_empty() {
                request = request.header(COOKIE, cookie);
            }
            // Only to the linking page's own origin, as a browser's default
            // policy sends a full Referer; the page URL may carry state.
            if let Some(referer) = referer.filter(|referer| referer.origin() == current.origin()) {
                request = request.header(REFERER, referer.as_str());
            }
            if let Some(range) = range {
                request = request.header(RANGE, range);
            }
            let response = tokio::select! {
                _ = tasks.closed() => return Err(FetchError::Cancelled),
                response = tokio::time::timeout(REQUEST_TIMEOUT, request.send()) => {
                    response.map_err(|_| FetchError::Timeout)?
                        .map_err(|_| FetchError::Network)
                }
            }?;
            // Redirects can rotate the browser's session too. Store every hop
            // and re-read its destination cookies; never forward the old header.
            let set_cookie: Vec<String> = response
                .headers()
                .get_all(SET_COOKIE)
                .iter()
                .filter_map(|value| value.to_str().ok().map(str::to_owned))
                .collect();
            if !set_cookie.is_empty() {
                tokio::select! {
                    _ = tasks.closed() => return Err(FetchError::Cancelled),
                    saved = tokio::time::timeout(REQUEST_TIMEOUT, session.store_set_cookies(&current, &set_cookie)) => {
                        saved.map_err(|_| FetchError::Timeout)?.map_err(session_error)?;
                    }
                }
            }
            if matches!(response.status().as_u16(), 401 | 403) {
                return Err(FetchError::Login);
            }
            if !response.status().is_redirection() {
                return Ok(response);
            }
            current = download_redirect(
                scope,
                &current,
                response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(FetchError::Invalid)?,
            )?;
        }
        Err(FetchError::Network)
    }
}

fn download_redirect(scope: Scope, current: &Url, location: &str) -> Result<Url, FetchError> {
    let target = current.join(location).map_err(|_| FetchError::Invalid)?;
    // Recognize login before the allowlist check: an SSO redirect is a
    // renewable session, not a permanently invalid file URL.
    if crate::school_browser::automation_target(target.as_str()).is_some()
        && scope.signed_out(&target)
    {
        return Err(FetchError::Login);
    }
    if !scope.admits(&target) {
        return Err(FetchError::Invalid);
    }
    Ok(target)
}

impl DownloadTransport for HttpTransport {
    fn probe<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, u64> {
        Box::pin(async move {
            let response = self
                .request(session, url, Scope::Lms, None, Some("bytes=0-0"), &tasks)
                .await?;
            let final_url = checked_response_url(Scope::Lms, &response)?;
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
                .unwrap_or(0))
        })
    }

    fn download<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        mut file: File,
        limit: Option<u64>,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, Fetched> {
        Box::pin(async move {
            let mut response = self
                .request(session, url, Scope::Lms, None, None, &tasks)
                .await?;
            let final_url = checked_response_url(Scope::Lms, &response)?;
            if is_login_url(&final_url) {
                return Err(FetchError::Login);
            }
            if !is_plugin_file_url(&final_url) || !response.status().is_success() {
                return Err(FetchError::Invalid);
            }
            let mut written = 0_u64;
            loop {
                let chunk = tokio::select! {
                    _ = tasks.closed() => return Err(FetchError::Cancelled),
                    chunk = response.chunk() => chunk.map_err(|_| FetchError::Network)?,
                };
                let Some(chunk) = chunk else { break };
                written = written.saturating_add(chunk.len() as u64);
                if limit.is_some_and(|limit| written > limit) {
                    return Ok(Fetched::TooLarge(written));
                }
                file.write_all(&chunk)
                    .await
                    .map_err(|_| FetchError::Network)?;
            }
            file.flush().await.map_err(|_| FetchError::Network)?;
            file.sync_all().await.map_err(|_| FetchError::Network)?;
            Ok(Fetched::Saved)
        })
    }

    fn resolve<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        cmid: u64,
        module: &'a str,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, ResolvedResource> {
        Box::pin(async move {
            let mut response = self
                .request(session, url, Scope::Lms, None, None, &tasks)
                .await?;
            let final_url = checked_response_url(Scope::Lms, &response)?;
            if response.status().is_redirection() || is_login_url(&final_url) {
                return Err(FetchError::Login);
            }
            if !response.status().is_success() {
                return Err(FetchError::Network);
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
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/html"))
                || response
                    .content_length()
                    .is_some_and(|bytes| bytes > MAX_RESOURCE_PAGE)
            {
                return Ok(ResolvedResource::Unresolved);
            }
            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! {
                    _ = tasks.closed() => return Err(FetchError::Cancelled),
                    chunk = response.chunk() => chunk.map_err(|_| FetchError::Network)?,
                };
                let Some(chunk) = chunk else { break };
                if bytes.len().saturating_add(chunk.len()) > MAX_RESOURCE_PAGE as usize {
                    return Ok(ResolvedResource::Unresolved);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(parse_resource_page(
                &String::from_utf8_lossy(&bytes),
                &final_url,
                module,
            ))
        })
    }

    fn school_file<'a>(
        &'a self,
        session: Arc<dyn SessionCookies>,
        url: &'a Url,
        referer: Option<&'a Url>,
        mut file: File,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, SchoolResponse> {
        Box::pin(async move {
            let mut response = self
                .request(session, url, Scope::School, referer, None, &tasks)
                .await?;
            let final_url = checked_response_url(Scope::School, &response)?;
            let status = response.status();
            if Scope::School.signed_out(&final_url) {
                return Err(FetchError::Login);
            }
            if status.is_server_error() {
                return Err(FetchError::Network);
            }
            if !status.is_success() {
                return Err(FetchError::Invalid);
            }
            let answer = SchoolResponse {
                url: Some(final_url),
                disposition: response
                    .headers()
                    .get(CONTENT_DISPOSITION)
                    .map(|value| value.as_bytes().to_vec()),
                content_type: response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            };
            loop {
                let chunk = tokio::select! {
                    _ = tasks.closed() => return Err(FetchError::Cancelled),
                    chunk = response.chunk() => chunk.map_err(|_| FetchError::Network)?,
                };
                let Some(chunk) = chunk else { break };
                file.write_all(&chunk).await.map_err(write_error)?;
            }
            file.flush().await.map_err(write_error)?;
            file.sync_all().await.map_err(write_error)?;
            Ok(answer)
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FlightKey {
    target: PathBuf,
    limit: Option<u64>,
}

struct Flight {
    result: watch::Sender<Option<Result<Fetched, FetchError>>>,
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

struct SmallPermit(Arc<SmallPool>);

impl Drop for SmallPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.active = state.active.saturating_sub(1);
        }
        self.0.changed.notify_waiters();
    }
}

struct ForegroundWaiter {
    pool: Arc<SmallPool>,
    registered: bool,
}

impl Drop for ForegroundWaiter {
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

impl SmallPool {
    async fn acquire(
        self: &Arc<Self>,
        foreground: bool,
        tasks: &RuntimeTasks,
    ) -> Result<SmallPermit, FetchError> {
        let mut waiter = ForegroundWaiter {
            pool: self.clone(),
            registered: false,
        };
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            // `notified()` alone is lazy. Enable it before inspecting state so
            // notify_waiters cannot be lost between the check and the await.
            changed.as_mut().enable();
            {
                let mut state = self.state.lock().map_err(|_| FetchError::Network)?;
                if state.active < 4 && (foreground || state.foreground_waiters == 0) {
                    state.active += 1;
                    if waiter.registered {
                        state.foreground_waiters = state.foreground_waiters.saturating_sub(1);
                        waiter.registered = false;
                    }
                    return Ok(SmallPermit(self.clone()));
                }
                if foreground && !waiter.registered {
                    state.foreground_waiters += 1;
                    waiter.registered = true;
                }
            }
            tokio::select! {
                _ = tasks.closed() => return Err(FetchError::Cancelled),
                _ = &mut changed => {}
            }
        }
    }
}

/// The process that wrote a partial download temp file named
/// `<target>.part-<pid>-<sequence>` (see `create_temp`). Partials of another
/// process were orphaned by a crash or OS kill; cleanup reclaims them.
pub(crate) fn partial_download_owner(name: &str) -> Option<u32> {
    let (_, suffix) = name.rsplit_once(".part-")?;
    let (pid, sequence) = suffix.split_once('-')?;
    sequence.parse::<u64>().ok()?;
    pid.parse().ok()
}

struct TempPath {
    path: PathBuf,
    armed: bool,
}

impl TempPath {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

struct DownloaderInner {
    transport: Arc<dyn DownloadTransport>,
    sign_in: OnceLock<SignInGate>,
    flights: tokio::sync::Mutex<HashMap<FlightKey, Arc<Flight>>>,
    target_locks: Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>,
    source_locks: Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    small: Arc<SmallPool>,
    large: Arc<Semaphore>,
    tasks: RuntimeTasks,
    temp_sequence: AtomicU64,
}

#[derive(Clone)]
pub struct Downloader(Arc<DownloaderInner>);

impl Default for Downloader {
    fn default() -> Self {
        Self::with_transport(Arc::new(HttpTransport::default()))
    }
}

impl Downloader {
    fn with_transport(transport: Arc<dyn DownloadTransport>) -> Self {
        Self(Arc::new(DownloaderInner {
            transport,
            sign_in: OnceLock::new(),
            flights: tokio::sync::Mutex::new(HashMap::new()),
            target_locks: Mutex::new(HashMap::new()),
            source_locks: Mutex::new(HashMap::new()),
            small: Arc::new(SmallPool::default()),
            large: Arc::new(Semaphore::new(1)),
            tasks: RuntimeTasks::default(),
            temp_sequence: AtomicU64::new(1),
        }))
    }

    /// Shares the browser's one sign-in flight with ordinary school reads.
    /// Called once when Core attaches the native browser.
    pub(crate) fn attach_sign_in(&self, sign_in: SignInGate) {
        let _ = self.0.sign_in.set(sign_in);
    }

    async fn authenticated<T, F: Future<Output = Result<T, FetchError>>>(
        &self,
        mut read: impl FnMut() -> F,
    ) -> Result<T, FetchError> {
        let result = read().await;
        if !matches!(result, Err(FetchError::Login)) {
            return result;
        }
        let Some(sign_in) = self.0.sign_in.get() else {
            return result;
        };
        tokio::select! {
            _ = self.0.tasks.closed() => return Err(FetchError::Cancelled),
            renewed = tokio::time::timeout(SESSION_TIMEOUT,
                sign_in.ensure(crate::school_auth::SchoolAuthProfile::Science)) => {
                renewed.map_err(|_| FetchError::Timeout)?.map_err(session_error)?;
            }
        }
        // A rejected second read is final; never loop on a wrong password.
        read().await
    }

    pub async fn fetch(
        &self,
        session: Arc<dyn SessionCookies>,
        url: &Url,
        target: PathBuf,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Fetched, FetchError> {
        if self.0.tasks.is_closed() {
            return Err(FetchError::Cancelled);
        }
        if target.is_file() {
            return Ok(Fetched::Saved);
        }
        if !is_lms_origin(url) {
            return Err(FetchError::Invalid);
        }
        let key = FlightKey {
            target: target.clone(),
            limit,
        };
        let (flight, leader) = {
            let mut flights = self.0.flights.lock().await;
            if let Some(flight) = flights.get(&key) {
                (flight.clone(), false)
            } else {
                let (result, _) = watch::channel(None);
                let flight = Arc::new(Flight { result });
                flights.insert(key.clone(), flight.clone());
                (flight, true)
            }
        };
        let mut result = flight.result.subscribe();
        if leader {
            let downloader = self.clone();
            let worker_session = session.clone();
            let worker_url = url.clone();
            let worker_flight = flight.clone();
            let worker_key = key.clone();
            if !self.0.tasks.spawn(async move {
                let work_downloader = downloader.clone();
                let outcome = match tokio::spawn(async move {
                    work_downloader
                        .authenticated(|| {
                            work_downloader.fetch_once(
                                worker_session.clone(),
                                &worker_url,
                                target.clone(),
                                limit,
                                foreground,
                            )
                        })
                        .await
                })
                .await
                {
                    Ok(outcome) => outcome,
                    Err(error) if error.is_panic() => Err(FetchError::Network),
                    Err(_) => Err(FetchError::Cancelled),
                };
                downloader.0.flights.lock().await.remove(&worker_key);
                // Remove the completed flight before publishing its result so
                // a retry begun by a woken caller always starts fresh work.
                let _ = worker_flight.result.send_replace(Some(outcome));
            }) {
                self.0.flights.lock().await.remove(&key);
                return Err(FetchError::Cancelled);
            }
        }
        loop {
            if let Some(value) = result.borrow().clone() {
                return value;
            }
            tokio::select! {
                _ = self.0.tasks.closed() => {
                    if let Some(value) = result.borrow().clone() { return value; }
                    return Err(FetchError::Cancelled);
                }
                changed = result.changed() => {
                    if changed.is_err() { return Err(FetchError::Cancelled); }
                }
            }
        }
    }

    async fn fetch_once(
        &self,
        session: Arc<dyn SessionCookies>,
        url: &Url,
        target: PathBuf,
        limit: Option<u64>,
        foreground: bool,
    ) -> Result<Fetched, FetchError> {
        let target_lock = keyed_lock(&self.0.target_locks, target.clone())?;
        let _target_turn = cancellable_lock(target_lock, &self.0.tasks).await?;
        if target.is_file() {
            return Ok(Fetched::Saved);
        }
        let source_lock = keyed_lock(&self.0.source_locks, url.to_string())?;
        let _source_turn = cancellable_lock(source_lock, &self.0.tasks).await?;
        if target.is_file() {
            return Ok(Fetched::Saved);
        }
        let size = tokio::select! {
            _ = self.0.tasks.closed() => return Err(FetchError::Cancelled),
            outcome = tokio::time::timeout(
                RESOURCE_TIMEOUT,
                self.0.transport.probe(session.clone(), url, self.0.tasks.clone()),
            ) => outcome.map_err(|_| FetchError::Timeout)??,
        };
        if limit.is_some_and(|limit| size > limit) {
            return Ok(Fetched::TooLarge(size));
        }
        let (_small, _large) = if uses_small_lane(size) {
            (
                Some(self.0.small.acquire(foreground, &self.0.tasks).await?),
                None,
            )
        } else {
            let permit = tokio::select! {
                _ = self.0.tasks.closed() => return Err(FetchError::Cancelled),
                permit = self.0.large.clone().acquire_owned() => {
                    permit.map_err(|_| FetchError::Cancelled)?
                }
            };
            (None, Some(permit))
        };
        let (file, mut temp) = self.create_temp(&target).await?;
        let outcome = tokio::select! {
            _ = self.0.tasks.closed() => Err(FetchError::Cancelled),
            outcome = tokio::time::timeout(
                DOWNLOAD_TIMEOUT,
                self.0.transport.download(session, url, file, limit, self.0.tasks.clone()),
            ) => outcome.map_err(|_| FetchError::Timeout)?,
        };
        match outcome? {
            Fetched::TooLarge(bytes) => Ok(Fetched::TooLarge(bytes)),
            Fetched::Saved => {
                if target.is_file() {
                    return Ok(Fetched::Saved);
                }
                tokio::fs::rename(&temp.path, &target)
                    .await
                    .map_err(write_error)?;
                temp.disarm();
                Ok(Fetched::Saved)
            }
        }
    }

    async fn create_temp(&self, target: &Path) -> Result<(File, TempPath), FetchError> {
        let base = target
            .file_name()
            .map(|name| name.to_os_string())
            .unwrap_or_else(|| OsString::from("download"));
        for _ in 0..16 {
            let sequence = self.0.temp_sequence.fetch_add(1, Ordering::Relaxed);
            let mut name = base.clone();
            name.push(format!(".part-{}-{sequence}", std::process::id()));
            let path = target.with_file_name(name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(file) => return Ok((file, TempPath { path, armed: true })),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(write_error(error)),
            }
        }
        Err(FetchError::Network)
    }

    pub async fn resolve(
        &self,
        session: Arc<dyn SessionCookies>,
        url: &Url,
        cmid: u64,
        module: &str,
    ) -> Result<ResolvedResource, FetchError> {
        if self.0.tasks.is_closed() {
            return Err(FetchError::Cancelled);
        }
        if !is_lms_origin(url) {
            return Err(FetchError::Invalid);
        }
        let transport = self.0.transport.clone();
        let url = url.clone();
        let module = module.to_owned();
        let worker_tasks = self.0.tasks.clone();
        let downloader = self.clone();
        self.0
            .tasks
            .run(async move {
                let transport_tasks = worker_tasks.clone();
                tokio::select! {
                    _ = worker_tasks.closed() => Err(FetchError::Cancelled),
                    outcome = tokio::time::timeout(
                        RESOURCE_TIMEOUT,
                        downloader.authenticated(|| transport.resolve(session.clone(), &url, cmid, &module, transport_tasks.clone())),
                    ) => outcome.map_err(|_| FetchError::Timeout)?,
                }
            })
            .await
            .map_err(|_| FetchError::Cancelled)?
    }

    /// Saves a file a school page handed over into `directory`, the shell's
    /// staging folder, under the name Core gives it
    /// (`school_files::file_name`). The shell moves it to the person's
    /// downloads. A file off the school hosts is answered `External` and never
    /// fetched. No sign-in is renewed: the person is on the signed-in page
    /// that linked the file, so a lost session is reported as `Login`.
    pub async fn save_school_file(
        &self,
        session: Arc<dyn SessionCookies>,
        request: SchoolFileRequest,
        directory: PathBuf,
    ) -> Result<SchoolFile, FetchError> {
        if self.0.tasks.is_closed() {
            return Err(FetchError::Cancelled);
        }
        let Some(url) = school_file_url(&request.url) else {
            return browsing_url(&request.url)
                .map(|url| SchoolFile::External { url })
                .ok_or(FetchError::Invalid);
        };
        let referer = request
            .referer
            .as_deref()
            .and_then(|raw| Url::parse(raw).ok())
            .filter(|referer| referer.scheme() == "https");
        let downloader = self.clone();
        let worker_tasks = self.0.tasks.clone();
        self.0
            .tasks
            .run(async move {
                tokio::select! {
                    _ = worker_tasks.closed() => Err(FetchError::Cancelled),
                    outcome = tokio::time::timeout(
                        DOWNLOAD_TIMEOUT,
                        downloader.school_file_once(session, &url, referer.as_ref(), &request, &directory),
                    ) => outcome.map_err(|_| FetchError::Timeout)?,
                }
            })
            .await
            .map_err(|_| FetchError::Cancelled)?
    }

    async fn school_file_once(
        &self,
        session: Arc<dyn SessionCookies>,
        url: &Url,
        referer: Option<&Url>,
        request: &SchoolFileRequest,
        directory: &Path,
    ) -> Result<SchoolFile, FetchError> {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(write_error)?;
        school_files::sweep_staging(directory);
        let (file, temp) = self.create_temp(&directory.join("download")).await?;
        let answer = self
            .0
            .transport
            .school_file(session, url, referer, file, self.0.tasks.clone())
            .await?;
        // The bare type (`application/pdf`), as a file store records it.
        let essence = |value: &str| {
            let essence = value.split(';').next().unwrap_or("").trim();
            (!essence.is_empty()).then(|| essence.to_ascii_lowercase())
        };
        let mime_type = answer
            .content_type
            .as_deref()
            .and_then(essence)
            .or_else(|| request.mime_type.as_deref().and_then(essence))
            .unwrap_or_else(|| "application/octet-stream".into());
        // An HTML page where the engine saw a file is a sign-in or error
        // page; the person keeps the page they are on.
        if mime_type == "text/html"
            && request.mime_type.as_deref().and_then(essence).as_deref() != Some("text/html")
        {
            return Err(FetchError::Invalid);
        }
        let name = school_files::file_name(
            answer
                .disposition
                .as_deref()
                .or(request.content_disposition.as_deref().map(str::as_bytes)),
            answer.url.as_ref().unwrap_or(url),
            Some(&mime_type),
        );
        publish(temp, directory, &name, mime_type).await
    }

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }
}

/// Moves a finished staging file to the first free name in `directory`.
async fn publish(
    mut temp: TempPath,
    directory: &Path,
    name: &str,
    mime_type: String,
) -> Result<SchoolFile, FetchError> {
    for candidate in school_files::name_candidates(name) {
        let path = directory.join(&candidate);
        if tokio::fs::try_exists(&path).await.map_err(write_error)? {
            continue;
        }
        tokio::fs::rename(&temp.path, &path)
            .await
            .map_err(write_error)?;
        temp.disarm();
        return Ok(SchoolFile::Saved {
            path,
            name: candidate,
            mime_type,
        });
    }
    Err(FetchError::WriteDenied)
}

fn uses_small_lane(size: u64) -> bool {
    size > 0 && size <= SMALL_FILE
}

fn keyed_lock<K: Eq + std::hash::Hash + Clone>(
    locks: &Mutex<HashMap<K, Weak<tokio::sync::Mutex<()>>>>,
    key: K,
) -> Result<Arc<tokio::sync::Mutex<()>>, FetchError> {
    let mut locks = locks.lock().map_err(|_| FetchError::Network)?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        Ok(lock)
    } else {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        Ok(lock)
    }
}

async fn cancellable_lock(
    lock: Arc<tokio::sync::Mutex<()>>,
    tasks: &RuntimeTasks,
) -> Result<tokio::sync::OwnedMutexGuard<()>, FetchError> {
    tokio::select! {
        _ = tasks.closed() => Err(FetchError::Cancelled),
        guard = lock.lock_owned() => Ok(guard),
    }
}

fn checked_response_url(scope: Scope, response: &reqwest::Response) -> Result<Url, FetchError> {
    let url = Url::parse(response.url().as_str()).map_err(|_| FetchError::Invalid)?;
    scope.admits(&url).then_some(url).ok_or(FetchError::Invalid)
}

#[cfg(test)]
#[path = "tests/downloads.rs"]
mod tests;
