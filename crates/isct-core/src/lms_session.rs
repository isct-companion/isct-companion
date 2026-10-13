//! LMS session readiness policy independent of native webview handles.

use crate::{downloads::FetchError, school_parser::lms::is_lms_origin};
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

const POLL_INTERVAL: Duration = Duration::from_millis(200);
const SCHOOL_TIMEOUT: Duration = Duration::from_secs(180);
const LMS_ATTEMPTS: usize = 2;
const LMS_POLLS_PER_ATTEMPT: usize = 100;
const RELOAD_DELAY: Duration = Duration::from_secs(5);
pub const LMS_HOME: &str = "https://lms.s.isct.ac.jp/2025/my/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionState {
    Pending,
    Active,
    Failed,
}

pub trait LmsSessionPort: Send + Sync {
    fn school_state(&self) -> Result<SessionState, FetchError>;
    fn request_school_session(&self) -> Result<(), FetchError>;
    fn ensure_lms_view(&self) -> Result<(), FetchError>;
    fn lms_ready(&self) -> Result<bool, FetchError>;
    fn navigate_lms_home(&self) -> Result<(), FetchError>;
    fn cookies(&self, url: &Url) -> Result<String, FetchError>;
}

#[derive(Default)]
pub struct LmsSession {
    gate: Mutex<()>,
}

impl LmsSession {
    pub async fn cookie_header(
        &self,
        port: &dyn LmsSessionPort,
        url: &Url,
    ) -> Result<String, FetchError> {
        if !is_lms_origin(url) {
            return Err(FetchError::Invalid);
        }
        let _gate = self.gate.lock().await;
        self.wait_for_school(port).await?;
        port.ensure_lms_view()?;

        for attempt in 0..LMS_ATTEMPTS {
            for _ in 0..LMS_POLLS_PER_ATTEMPT {
                if port.lms_ready()? {
                    let cookies = port.cookies(url)?;
                    return if cookies.trim().is_empty() {
                        Err(FetchError::Login)
                    } else {
                        Ok(cookies)
                    };
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            if attempt == 0 {
                // The legacy recovery path treated this request as a hint; the
                // explicit navigation below remains the authoritative retry.
                let _ = port.request_school_session();
                tokio::time::sleep(RELOAD_DELAY).await;
                port.navigate_lms_home()?;
            }
        }
        Err(FetchError::Login)
    }

    async fn wait_for_school(&self, port: &dyn LmsSessionPort) -> Result<(), FetchError> {
        if port.school_state()? == SessionState::Active {
            return Ok(());
        }
        port.request_school_session()?;
        tokio::time::timeout(SCHOOL_TIMEOUT, async {
            loop {
                match port.school_state()? {
                    SessionState::Active => return Ok(()),
                    SessionState::Failed => return Err(FetchError::Login),
                    SessionState::Pending => tokio::time::sleep(POLL_INTERVAL).await,
                }
            }
        })
        .await
        .map_err(|_| FetchError::Login)?
    }
}

#[cfg(test)]
mod tests {
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
    async fn concurrent_readiness_never_creates_parallel_views() {
        let session = LmsSession::default();
        let port = FakePort::new(SessionState::Active, 1, "MoodleSession=ready");
        let url = lms_url();
        let (first, second) = tokio::join!(
            session.cookie_header(&port, &url),
            session.cookie_header(&port, &url)
        );
        assert!(first.is_ok());
        assert!(second.is_ok());
        assert_eq!(port.view_creations.load(Ordering::Relaxed), 1);
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
}
