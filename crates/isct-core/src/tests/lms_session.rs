use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex as StdMutex,
};

struct FakePort {
    school: StdMutex<SessionState>,
    requests: AtomicUsize,
    ensure_calls: AtomicUsize,
    view_exists: AtomicBool,
    view_creations: AtomicUsize,
    ready_after: usize,
    ready_polls: AtomicUsize,
    navigations: AtomicUsize,
    cookies: StdMutex<String>,
    /// Callers between opening the view and reading its cookies.
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}

impl FakePort {
    fn new(school: SessionState, ready_after: usize, cookies: &str) -> Self {
        Self {
            school: StdMutex::new(school),
            requests: AtomicUsize::new(0),
            ensure_calls: AtomicUsize::new(0),
            view_exists: AtomicBool::new(false),
            view_creations: AtomicUsize::new(0),
            ready_after,
            ready_polls: AtomicUsize::new(0),
            navigations: AtomicUsize::new(0),
            cookies: StdMutex::new(cookies.into()),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        }
    }
}

impl LmsSessionPort for FakePort {
    fn school_state(&self) -> Result<SessionState, FetchError> {
        self.school
            .lock()
            .map(|state| *state)
            .map_err(|_| FetchError::Network)
    }

    fn request_school_session(&self) -> Result<(), FetchError> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn ensure_lms_view(&self) -> Result<(), FetchError> {
        self.ensure_calls.fetch_add(1, Ordering::Relaxed);
        let in_flight = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::AcqRel);
        if !self.view_exists.swap(true, Ordering::AcqRel) {
            self.view_creations.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn lms_ready(&self) -> Result<bool, FetchError> {
        let poll = self.ready_polls.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(poll > self.ready_after)
    }

    fn navigate_lms_home(&self) -> Result<(), FetchError> {
        self.navigations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn cookies(&self, _url: &Url) -> Result<String, FetchError> {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
        self.cookies
            .lock()
            .map(|cookies| cookies.clone())
            .map_err(|_| FetchError::Network)
    }
}

fn lms_url() -> Url {
    Url::parse("https://lms.s.isct.ac.jp/2026/pluginfile.php/1/file.pdf").unwrap()
}

#[tokio::test(start_paused = true)]
async fn ready_session_returns_nonempty_cookies() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Active, 0, "MoodleSession=secret");
    assert_eq!(
        session.cookie_header(&port, &lms_url()).await.unwrap(),
        "MoodleSession=secret"
    );
    assert_eq!(port.requests.load(Ordering::Relaxed), 0);
    assert_eq!(port.view_creations.load(Ordering::Relaxed), 1);

    *port.cookies.lock().unwrap() = String::new();
    assert_eq!(
        session.cookie_header(&port, &lms_url()).await,
        Err(FetchError::Login)
    );
}

#[tokio::test(start_paused = true)]
async fn failed_school_session_stops_before_view_creation() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Failed, 0, "unused");
    assert_eq!(
        session.cookie_header(&port, &lms_url()).await,
        Err(FetchError::Login)
    );
    assert_eq!(port.requests.load(Ordering::Relaxed), 1);
    assert_eq!(port.ensure_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn exhausted_first_attempt_requeues_then_navigates() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Active, 100, "MoodleSession=ready");
    assert_eq!(
        session.cookie_header(&port, &lms_url()).await.unwrap(),
        "MoodleSession=ready"
    );
    assert_eq!(port.ready_polls.load(Ordering::Relaxed), 101);
    assert_eq!(port.requests.load(Ordering::Relaxed), 1);
    assert_eq!(port.navigations.load(Ordering::Relaxed), 1);
}

#[tokio::test(start_paused = true)]
async fn pending_school_session_times_out_after_180_seconds() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Pending, 0, "unused");
    let started = tokio::time::Instant::now();
    assert_eq!(
        session.cookie_header(&port, &lms_url()).await,
        Err(FetchError::Login)
    );
    assert!(started.elapsed() >= SCHOOL_TIMEOUT);
    assert_eq!(port.requests.load(Ordering::Relaxed), 1);
    assert_eq!(port.ensure_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test(start_paused = true)]
async fn concurrent_readiness_waits_are_serialized() {
    let session = LmsSession::default();
    // The first caller is not ready on its first poll, so it sleeps while
    // holding the view; an ungated second caller would drive it meanwhile.
    let port = FakePort::new(SessionState::Active, 1, "MoodleSession=ready");
    let url = lms_url();
    let (first, second) = tokio::join!(
        session.cookie_header(&port, &url),
        session.cookie_header(&port, &url)
    );
    assert!(first.is_ok());
    assert!(second.is_ok());
    assert_eq!(port.ensure_calls.load(Ordering::Relaxed), 2);
    assert_eq!(port.max_in_flight.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn rejects_non_lms_urls_before_calling_the_port() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Active, 0, "unused");
    let url = Url::parse("https://example.test/file.pdf").unwrap();
    assert_eq!(
        session.cookie_header(&port, &url).await,
        Err(FetchError::Invalid)
    );
    assert_eq!(port.ensure_calls.load(Ordering::Relaxed), 0);
}
