//! Authenticated LMS resource resolution and bounded file downloads.

pub use crate::school_parser::lms::ResolvedResource;
use crate::{
    runtime_tasks::RuntimeTasks,
    school_parser::lms::{is_lms_origin, is_plugin_file_url, parse_resource_page},
};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, RANGE};
use std::{
    collections::HashMap,
    ffi::OsString,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, Weak,
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
const USER_AGENT: &str = "Mozilla/5.0 (compatible; IsctCompanion/0.1)";
const SESSION_TIMEOUT: Duration = Duration::from_secs(230);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const RESOURCE_TIMEOUT: Duration = Duration::from_secs(260);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_RESOURCE_PAGE: u64 = 1_000_000;

pub trait DownloadSession: Send + Sync {
    fn cookie_header<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<String, FetchError>> + Send + 'a>>;
}

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
}

impl FetchError {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Invalid => "invalid",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
        }
    }
}

type FetchFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, FetchError>> + Send + 'a>>;

trait DownloadTransport: Send + Sync {
    fn probe<'a>(
        &'a self,
        session: Arc<dyn DownloadSession>,
        url: &'a Url,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, u64>;

    fn download<'a>(
        &'a self,
        session: Arc<dyn DownloadSession>,
        url: &'a Url,
        file: File,
        limit: Option<u64>,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, Fetched>;

    fn resolve<'a>(
        &'a self,
        session: Arc<dyn DownloadSession>,
        url: &'a Url,
        cmid: u64,
        module: &'a str,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, ResolvedResource>;
}

struct HttpTransport {
    client: Result<reqwest::Client, FetchError>,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if !is_lms_origin(attempt.url()) {
                        attempt.stop()
                    } else if attempt.previous().len() >= 5 {
                        attempt.error("too many LMS redirects")
                    } else {
                        attempt.follow()
                    }
                }))
                .user_agent(USER_AGENT)
                .build()
                .map_err(|_| FetchError::Network),
        }
    }
}

impl HttpTransport {
    async fn request(
        &self,
        session: Arc<dyn DownloadSession>,
        url: &Url,
        range: Option<&str>,
        tasks: &RuntimeTasks,
    ) -> Result<reqwest::Response, FetchError> {
        if !is_lms_origin(url) {
            return Err(FetchError::Invalid);
        }
        let cookie = tokio::select! {
            _ = tasks.closed() => return Err(FetchError::Cancelled),
            cookie = tokio::time::timeout(SESSION_TIMEOUT, session.cookie_header(url)) => {
                cookie.map_err(|_| FetchError::Timeout)??
            },
        };
        if cookie.trim().is_empty() {
            return Err(FetchError::Login);
        }
        let client = self.client.as_ref().map_err(|error| *error)?;
        let mut request = client.get(url.as_str()).header(COOKIE, cookie);
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        tokio::select! {
            _ = tasks.closed() => Err(FetchError::Cancelled),
            response = tokio::time::timeout(REQUEST_TIMEOUT, request.send()) => {
                response.map_err(|_| FetchError::Timeout)?
                    .map_err(|_| FetchError::Network)
            }
        }
    }
}

impl DownloadTransport for HttpTransport {
    fn probe<'a>(
        &'a self,
        session: Arc<dyn DownloadSession>,
        url: &'a Url,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, u64> {
        Box::pin(async move {
            let response = self
                .request(session, url, Some("bytes=0-0"), &tasks)
                .await?;
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
                .unwrap_or(0))
        })
    }

    fn download<'a>(
        &'a self,
        session: Arc<dyn DownloadSession>,
        url: &'a Url,
        mut file: File,
        limit: Option<u64>,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, Fetched> {
        Box::pin(async move {
            let mut response = self.request(session, url, None, &tasks).await?;
            let final_url = checked_response_url(&response)?;
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
        session: Arc<dyn DownloadSession>,
        url: &'a Url,
        cmid: u64,
        module: &'a str,
        tasks: RuntimeTasks,
    ) -> FetchFuture<'a, ResolvedResource> {
        Box::pin(async move {
            let mut response = self.request(session, url, None, &tasks).await?;
            let final_url = checked_response_url(&response)?;
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
            flights: tokio::sync::Mutex::new(HashMap::new()),
            target_locks: Mutex::new(HashMap::new()),
            source_locks: Mutex::new(HashMap::new()),
            small: Arc::new(SmallPool::default()),
            large: Arc::new(Semaphore::new(1)),
            tasks: RuntimeTasks::default(),
            temp_sequence: AtomicU64::new(1),
        }))
    }

    pub async fn fetch(
        &self,
        session: Arc<dyn DownloadSession>,
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
                        .fetch_once(worker_session, &worker_url, target, limit, foreground)
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
        session: Arc<dyn DownloadSession>,
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
                    .map_err(|_| FetchError::Network)?;
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
                Err(_) => return Err(FetchError::Network),
            }
        }
        Err(FetchError::Network)
    }

    pub async fn resolve(
        &self,
        session: Arc<dyn DownloadSession>,
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
        self.0
            .tasks
            .run(async move {
                let transport_tasks = worker_tasks.clone();
                tokio::select! {
                    _ = worker_tasks.closed() => Err(FetchError::Cancelled),
                    outcome = tokio::time::timeout(
                        RESOURCE_TIMEOUT,
                        transport.resolve(session, &url, cmid, &module, transport_tasks),
                    ) => outcome.map_err(|_| FetchError::Timeout)?,
                }
            })
            .await
            .map_err(|_| FetchError::Cancelled)?
    }

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
    }
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

fn checked_response_url(response: &reqwest::Response) -> Result<Url, FetchError> {
    let url = Url::parse(response.url().as_str()).map_err(|_| FetchError::Invalid)?;
    is_lms_origin(&url)
        .then_some(url)
        .ok_or(FetchError::Invalid)
}

fn is_login_url(url: &Url) -> bool {
    url.path_segments()
        .is_some_and(|mut segments| segments.any(|segment| segment == "login" || segment == "auth"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Semaphore as AsyncSemaphore;
    use uuid::Uuid;

    struct CookieSession;

    impl DownloadSession for CookieSession {
        fn cookie_header<'a>(&'a self, _url: &'a Url) -> FetchFuture<'a, String> {
            Box::pin(async { Ok("MoodleSession=test".into()) })
        }
    }

    enum DownloadStep {
        Complete(Vec<u8>),
        PartialThenError(Vec<u8>),
        Wait(Vec<u8>),
        Panic,
        Cancel,
    }

    struct FakeTransport {
        size: u64,
        probes: AtomicUsize,
        downloads: AtomicUsize,
        resolves: AtomicUsize,
        active: AtomicUsize,
        max_active: AtomicUsize,
        steps: Mutex<VecDeque<DownloadStep>>,
        probe_started: Notify,
        probe_release: AsyncSemaphore,
        wait_started: Notify,
        wait_release: AsyncSemaphore,
        gate_first_probe: bool,
    }

    impl FakeTransport {
        fn new(size: u64, steps: impl IntoIterator<Item = DownloadStep>) -> Self {
            Self {
                size,
                probes: AtomicUsize::new(0),
                downloads: AtomicUsize::new(0),
                resolves: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                steps: Mutex::new(steps.into_iter().collect()),
                probe_started: Notify::new(),
                probe_release: AsyncSemaphore::new(0),
                wait_started: Notify::new(),
                wait_release: AsyncSemaphore::new(0),
                gate_first_probe: false,
            }
        }

        fn with_gated_probe(mut self) -> Self {
            self.gate_first_probe = true;
            self
        }

        fn enter(&self) -> ActiveDownload<'_> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            ActiveDownload(self)
        }
    }

    struct ActiveDownload<'a>(&'a FakeTransport);

    impl Drop for ActiveDownload<'_> {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl DownloadTransport for FakeTransport {
        fn probe<'a>(
            &'a self,
            _session: Arc<dyn DownloadSession>,
            _url: &'a Url,
            _tasks: RuntimeTasks,
        ) -> FetchFuture<'a, u64> {
            Box::pin(async move {
                let call = self.probes.fetch_add(1, Ordering::SeqCst);
                if self.gate_first_probe && call == 0 {
                    self.probe_started.notify_waiters();
                    self.probe_release
                        .acquire()
                        .await
                        .map_err(|_| FetchError::Cancelled)?
                        .forget();
                }
                Ok(self.size)
            })
        }

        fn download<'a>(
            &'a self,
            _session: Arc<dyn DownloadSession>,
            _url: &'a Url,
            mut file: File,
            _limit: Option<u64>,
            tasks: RuntimeTasks,
        ) -> FetchFuture<'a, Fetched> {
            Box::pin(async move {
                self.downloads.fetch_add(1, Ordering::SeqCst);
                let _active = self.enter();
                let step = self
                    .steps
                    .lock()
                    .map_err(|_| FetchError::Network)?
                    .pop_front()
                    .ok_or(FetchError::Network)?;
                match step {
                    DownloadStep::Complete(bytes) => {
                        file.write_all(&bytes)
                            .await
                            .map_err(|_| FetchError::Network)?;
                        file.flush().await.map_err(|_| FetchError::Network)?;
                        Ok(Fetched::Saved)
                    }
                    DownloadStep::PartialThenError(bytes) => {
                        file.write_all(&bytes)
                            .await
                            .map_err(|_| FetchError::Network)?;
                        Err(FetchError::Network)
                    }
                    DownloadStep::Wait(bytes) => {
                        self.wait_started.notify_waiters();
                        self.wait_release
                            .acquire()
                            .await
                            .map_err(|_| FetchError::Cancelled)?
                            .forget();
                        file.write_all(&bytes)
                            .await
                            .map_err(|_| FetchError::Network)?;
                        Ok(Fetched::Saved)
                    }
                    DownloadStep::Panic => {
                        self.wait_started.notify_waiters();
                        self.wait_release
                            .acquire()
                            .await
                            .map_err(|_| FetchError::Cancelled)?
                            .forget();
                        panic!("transport panic")
                    }
                    DownloadStep::Cancel => {
                        tasks.closed().await;
                        Err(FetchError::Cancelled)
                    }
                }
            })
        }

        fn resolve<'a>(
            &'a self,
            _session: Arc<dyn DownloadSession>,
            _url: &'a Url,
            _cmid: u64,
            _module: &'a str,
            _tasks: RuntimeTasks,
        ) -> FetchFuture<'a, ResolvedResource> {
            Box::pin(async move {
                self.resolves.fetch_add(1, Ordering::SeqCst);
                Ok(ResolvedResource::Unresolved)
            })
        }
    }

    fn url() -> Url {
        Url::parse("https://lms.s.isct.ac.jp/2026/pluginfile.php/1/file.pdf").unwrap()
    }

    fn target(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("isct-download-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("file.pdf")
    }

    fn session() -> Arc<dyn DownloadSession> {
        Arc::new(CookieSession)
    }

    fn part_files(target: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".part-"))
            })
            .collect()
    }

    #[tokio::test]
    async fn duplicate_waiter_survives_the_original_callers_drop() {
        let transport = Arc::new(FakeTransport::new(
            10,
            [DownloadStep::Wait(b"complete".to_vec())],
        ));
        let downloader = Downloader::with_transport(transport.clone());
        let path = target("caller-drop");
        let started = transport.wait_started.notified();
        tokio::pin!(started);
        started.as_mut().enable();
        let first = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move { downloader.fetch(session(), &url(), path, None, false).await }
        });
        started.await;
        first.abort();
        let second = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move { downloader.fetch(session(), &url(), path, None, true).await }
        });
        tokio::task::yield_now().await;
        transport.wait_release.add_permits(1);
        assert_eq!(second.await.unwrap(), Ok(Fetched::Saved));
        assert_eq!(transport.downloads.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(path).unwrap(), b"complete");
    }

    #[tokio::test]
    async fn failed_flight_is_removed_and_a_retry_starts_fresh() {
        let transport = Arc::new(FakeTransport::new(
            10,
            [
                DownloadStep::PartialThenError(b"partial".to_vec()),
                DownloadStep::Complete(b"complete".to_vec()),
            ],
        ));
        let downloader = Downloader::with_transport(transport.clone());
        let path = target("retry");
        assert_eq!(
            downloader
                .fetch(session(), &url(), path.clone(), None, false)
                .await,
            Err(FetchError::Network)
        );
        assert!(!path.exists());
        assert!(part_files(&path).is_empty());
        assert_eq!(
            downloader
                .fetch(session(), &url(), path.clone(), None, false)
                .await,
            Ok(Fetched::Saved)
        );
        assert_eq!(transport.downloads.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read(path).unwrap(), b"complete");
    }

    #[tokio::test]
    async fn transport_panic_releases_every_waiter_and_allows_retry() {
        let transport = Arc::new(FakeTransport::new(
            10,
            [
                DownloadStep::Panic,
                DownloadStep::Complete(b"complete".to_vec()),
            ],
        ));
        let downloader = Downloader::with_transport(transport.clone());
        let path = target("panic");
        let started = transport.wait_started.notified();
        tokio::pin!(started);
        started.as_mut().enable();
        let first = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move { downloader.fetch(session(), &url(), path, None, false).await }
        });
        started.await;
        let second = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move { downloader.fetch(session(), &url(), path, None, false).await }
        });
        loop {
            let joined = downloader
                .0
                .flights
                .lock()
                .await
                .values()
                .next()
                .is_some_and(|flight| flight.result.receiver_count() >= 2);
            if joined {
                break;
            }
            tokio::task::yield_now().await;
        }
        transport.wait_release.add_permits(1);

        assert_eq!(first.await.unwrap(), Err(FetchError::Network));
        assert_eq!(second.await.unwrap(), Err(FetchError::Network));
        assert!(!path.exists());
        assert!(part_files(&path).is_empty());
        assert_eq!(
            downloader
                .fetch(session(), &url(), path.clone(), None, false)
                .await,
            Ok(Fetched::Saved)
        );
        assert_eq!(transport.downloads.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read(path).unwrap(), b"complete");
    }

    #[tokio::test]
    async fn a_small_limit_never_poisoned_a_larger_concurrent_request() {
        let transport = Arc::new(
            FakeTransport::new(100, [DownloadStep::Complete(vec![7; 100])]).with_gated_probe(),
        );
        let downloader = Downloader::with_transport(transport.clone());
        let path = target("mixed-limit");
        let probe_started = transport.probe_started.notified();
        tokio::pin!(probe_started);
        probe_started.as_mut().enable();
        let small = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move {
                downloader
                    .fetch(session(), &url(), path, Some(50), false)
                    .await
            }
        });
        probe_started.await;
        let large = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move {
                downloader
                    .fetch(session(), &url(), path, Some(200), true)
                    .await
            }
        });
        transport.probe_release.add_permits(1);
        assert_eq!(small.await.unwrap(), Ok(Fetched::TooLarge(100)));
        assert_eq!(large.await.unwrap(), Ok(Fetched::Saved));
        assert_eq!(transport.downloads.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::metadata(path).unwrap().len(), 100);
    }

    #[tokio::test]
    async fn shutdown_cancels_waiters_and_cleans_partial_files() {
        let transport = Arc::new(FakeTransport::new(0, [DownloadStep::Cancel]));
        let downloader = Downloader::with_transport(transport.clone());
        let path = target("shutdown");
        let request = tokio::spawn({
            let downloader = downloader.clone();
            let path = path.clone();
            async move { downloader.fetch(session(), &url(), path, None, false).await }
        });
        while transport.downloads.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        downloader.shutdown().await;
        assert_eq!(request.await.unwrap(), Err(FetchError::Cancelled));
        assert!(!path.exists());
        assert!(part_files(&path).is_empty());
        assert_eq!(
            downloader.fetch(session(), &url(), path, None, false).await,
            Err(FetchError::Cancelled)
        );
    }

    #[tokio::test]
    async fn rename_failure_keeps_the_existing_target_and_removes_temp() {
        let transport = Arc::new(FakeTransport::new(
            10,
            [DownloadStep::Complete(b"complete".to_vec())],
        ));
        let downloader = Downloader::with_transport(transport);
        let path = target("rename-failure");
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            downloader
                .fetch(session(), &url(), path.clone(), None, false)
                .await,
            Err(FetchError::Network)
        );
        assert!(path.is_dir());
        assert!(part_files(&path).is_empty());
    }

    #[tokio::test]
    async fn unknown_sizes_use_one_serial_lane() {
        let transport = Arc::new(FakeTransport::new(
            0,
            [
                DownloadStep::Wait(b"first".to_vec()),
                DownloadStep::Complete(b"second".to_vec()),
            ],
        ));
        let downloader = Downloader::with_transport(transport.clone());
        let first_path = target("unknown-first");
        let second_path = target("unknown-second");
        let started = transport.wait_started.notified();
        tokio::pin!(started);
        started.as_mut().enable();
        let first = tokio::spawn({
            let downloader = downloader.clone();
            async move {
                downloader
                    .fetch(session(), &url(), first_path, None, false)
                    .await
            }
        });
        started.await;
        let second = tokio::spawn({
            let downloader = downloader.clone();
            async move {
                downloader
                    .fetch(session(), &url(), second_path, None, false)
                    .await
            }
        });
        tokio::task::yield_now().await;
        assert_eq!(transport.max_active.load(Ordering::SeqCst), 1);
        transport.wait_release.add_permits(1);
        assert_eq!(first.await.unwrap(), Ok(Fetched::Saved));
        assert_eq!(second.await.unwrap(), Ok(Fetched::Saved));
        assert_eq!(transport.max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn foreground_waiters_take_the_next_small_lane() {
        let pool = Arc::new(SmallPool::default());
        let tasks = RuntimeTasks::default();
        let mut permits = Vec::new();
        for _ in 0..4 {
            permits.push(pool.acquire(false, &tasks).await.unwrap());
        }
        let order = Arc::new(Mutex::new(Vec::new()));
        let background = tokio::spawn({
            let pool = pool.clone();
            let tasks = tasks.clone();
            let order = order.clone();
            async move {
                let _permit = pool.acquire(false, &tasks).await.unwrap();
                order.lock().unwrap().push("background");
            }
        });
        let foreground = tokio::spawn({
            let pool = pool.clone();
            let tasks = tasks.clone();
            let order = order.clone();
            async move {
                let _permit = pool.acquire(true, &tasks).await.unwrap();
                order.lock().unwrap().push("foreground");
            }
        });
        loop {
            if pool
                .state
                .lock()
                .is_ok_and(|state| state.foreground_waiters == 1)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        permits.pop();
        foreground.await.unwrap();
        permits.pop();
        background.await.unwrap();
        assert_eq!(*order.lock().unwrap(), ["foreground", "background"]);
    }

    #[tokio::test]
    async fn resolve_delegates_once_and_rejects_non_lms_urls() {
        let transport = Arc::new(FakeTransport::new(0, []));
        let downloader = Downloader::with_transport(transport.clone());
        let resource =
            Url::parse("https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
        assert_eq!(
            downloader
                .resolve(session(), &resource, 42, "lecture.pdf")
                .await,
            Ok(ResolvedResource::Unresolved)
        );
        assert_eq!(transport.resolves.load(Ordering::SeqCst), 1);

        let outside = Url::parse("https://example.com/resource").unwrap();
        assert_eq!(
            downloader.resolve(session(), &outside, 42, "x").await,
            Err(FetchError::Invalid)
        );
        assert_eq!(transport.resolves.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn errors_keep_the_existing_wire_reasons_and_cancel_is_distinct() {
        for (error, reason) in [
            (FetchError::Login, "login"),
            (FetchError::Invalid, "invalid"),
            (FetchError::Network, "network"),
            (FetchError::Timeout, "timeout"),
            (FetchError::Cancelled, "cancelled"),
        ] {
            assert_eq!(error.reason(), reason);
            assert!(!error.to_string().is_empty());
        }
    }
}
