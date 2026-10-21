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
    stored: StdMutex<Vec<String>>,
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
            stored: StdMutex::new(Vec::new()),
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        }
    }
}

impl SchoolCookiePort for FakePort {
    fn school_state(&self) -> Result<SessionState, NetError> {
        self.school
            .lock()
            .map(|state| *state)
            .map_err(|_| NetError::Invalid("poisoned".into()))
    }

    fn request_school_session(&self) -> Result<(), NetError> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn ensure_lms_view(&self) -> Result<(), NetError> {
        self.ensure_calls.fetch_add(1, Ordering::Relaxed);
        let in_flight = self.in_flight.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_in_flight.fetch_max(in_flight, Ordering::AcqRel);
        if !self.view_exists.swap(true, Ordering::AcqRel) {
            self.view_creations.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    fn lms_ready(&self) -> Result<bool, NetError> {
        let poll = self.ready_polls.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(poll > self.ready_after)
    }

    fn navigate_lms_home(&self) -> Result<(), NetError> {
        self.navigations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn cookies(&self, _url: &Url) -> Result<Vec<(String, String)>, NetError> {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
        let cookies = self
            .cookies
            .lock()
            .map_err(|_| NetError::Invalid("poisoned".into()))?;
        Ok(cookies
            .split("; ")
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect())
    }

    fn set_cookies(&self, _url: &Url, set_cookie: &[String]) -> Result<(), NetError> {
        self.stored
            .lock()
            .map_err(|_| NetError::Invalid("poisoned".into()))?
            .extend_from_slice(set_cookie);
        Ok(())
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
        Err(NetError::SignInRequired)
    );
}

#[tokio::test(start_paused = true)]
async fn failed_school_session_stops_before_view_creation() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Failed, 0, "unused");
    assert_eq!(
        session.cookie_header(&port, &lms_url()).await,
        Err(NetError::SignInRequired)
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
        Err(NetError::SignInRequired)
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
async fn rejects_non_school_urls_before_calling_the_port() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Active, 0, "unused=1");
    for raw in [
        "https://example.test/file.pdf",
        "http://lms.s.isct.ac.jp/2026/",
        "https://syllabus.s.isct.ac.jp/",
    ] {
        let url = Url::parse(raw).unwrap();
        assert!(matches!(
            session.cookie_header(&port, &url).await,
            Err(NetError::Invalid(_))
        ));
    }
    assert_eq!(port.ensure_calls.load(Ordering::Relaxed), 0);
}

/// 教務Web and GetAccess are read straight from the store, without the LMS
/// view; 教務Web needs its ASP.NET session cookie, not just any cookie.
#[tokio::test]
async fn other_school_origins_read_the_store_directly() {
    let session = LmsSession::default();
    let kyomu =
        Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/PID1_0.aspx").unwrap();
    let portal = Url::parse("https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList").unwrap();

    let port = FakePort::new(SessionState::Pending, 0, "TitechForm=x");
    assert_eq!(
        session.cookie_header(&port, &kyomu).await,
        Err(NetError::SignInRequired)
    );
    assert_eq!(
        session.cookie_header(&port, &portal).await.unwrap(),
        "TitechForm=x"
    );
    let account = Url::parse("https://isct.ex-tic.com/idm/user/mfa_setting/").unwrap();
    assert_eq!(
        session.cookie_header(&port, &account).await.unwrap(),
        "TitechForm=x"
    );
    let signed_out = FakePort::new(SessionState::Pending, 0, "");
    assert_eq!(
        session.cookie_header(&signed_out, &account).await,
        Err(NetError::SignInRequired)
    );

    let port = FakePort::new(
        SessionState::Pending,
        0,
        "ASP.NET_SessionId=s; TitechForm=x",
    );
    assert_eq!(
        session.cookie_header(&port, &kyomu).await.unwrap(),
        "ASP.NET_SessionId=s; TitechForm=x"
    );
    assert_eq!(port.ensure_calls.load(Ordering::Relaxed), 0);
    assert_eq!(port.requests.load(Ordering::Relaxed), 0);
}

/// A Rust response may only write the cookies a browser would accept from
/// it: host-only, or a Domain covering the host that names a whole site.
#[test]
fn set_cookie_write_back_keeps_only_cookies_for_the_response_host() {
    let session = LmsSession::default();
    let port = FakePort::new(SessionState::Active, 0, "");
    let kyomu = Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx").unwrap();
    let values = [
        "KYOMU-http=1; path=/; HttpOnly",
        "a=1; Domain=.titech.ac.jp; Path=/",
        "b=1; Domain=gakumu.titech.ac.jp",
        "c=1; Domain=ac.jp",
        "d=1; Domain=isct.ac.jp",
        "=nameless; Path=/",
        "no-pair",
    ]
    .map(String::from);
    session.store_set_cookies(&port, &kyomu, &values).unwrap();
    assert_eq!(
        *port.stored.lock().unwrap(),
        [
            "KYOMU-http=1; path=/; HttpOnly",
            "a=1; Domain=.titech.ac.jp; Path=/",
            "b=1; Domain=gakumu.titech.ac.jp",
        ]
    );

    let public = Url::parse("https://syllabus.s.isct.ac.jp/").unwrap();
    session.store_set_cookies(&port, &public, &values).unwrap();
    assert_eq!(port.stored.lock().unwrap().len(), 3);
    let account = Url::parse("https://isct.ex-tic.com/idm/user/mfa_setting/").unwrap();
    session
        .store_set_cookies(
            &port,
            &account,
            &[
                "session=school; Path=/; Secure".into(),
                "shared=school; Domain=ex-tic.com".into(),
                "foreign=x; Domain=isct.ac.jp".into(),
            ],
        )
        .unwrap();
    assert_eq!(
        &port.stored.lock().unwrap()[3..],
        &["session=school; Path=/; Secure"]
    );
}
