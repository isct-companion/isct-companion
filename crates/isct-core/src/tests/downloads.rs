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
    let directory = std::env::temp_dir().join(format!("isct-download-{name}-{}", Uuid::new_v4()));
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

/// Controlled folder access on Windows refuses the write like a read-only
/// folder does; the user must learn how to allow it, not see a network error.
#[cfg(unix)]
#[tokio::test]
async fn a_folder_that_refuses_writes_reports_denied_instead_of_a_network_failure() {
    use std::os::unix::fs::PermissionsExt;
    let transport = Arc::new(FakeTransport::new(
        10,
        [
            DownloadStep::Complete(b"complete".to_vec()),
            DownloadStep::Complete(b"complete".to_vec()),
        ],
    ));
    let downloader = Downloader::with_transport(transport);
    let path = target("denied");
    let folder = path.parent().unwrap().to_path_buf();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = downloader
        .fetch(session(), &url(), path.clone(), None, false)
        .await;
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result, Err(FetchError::WriteDenied));
    assert!(FetchError::WriteDenied
        .to_string()
        .contains("choose another course folder"));
    assert!(!path.exists());
    // Once the user allows the app, a retry saves normally: nothing remembers
    // the refusal.
    assert_eq!(
        downloader
            .fetch(session(), &url(), path.clone(), None, false)
            .await,
        Ok(Fetched::Saved)
    );
    assert_eq!(std::fs::read(path).unwrap(), b"complete");
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
    let resource = Url::parse("https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
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
