//! The session rules: the jar, the identity snapshot, the account barrier,
//! verification and the web cookie queue, over a scripted Science Tokyo
//! (`School`, which the web operation suite uses too). The real sign-in
//! flows run against the real schools (`school_session__login.rs`, ignored
//! by default).
use super::*;
use std::{sync::atomic::AtomicBool, time::Duration};

#[test]
fn a_jar_keeps_session_cookies_and_refuses_a_public_suffix() {
    let url = Url::parse("https://portal.nap.gsic.titech.ac.jp/GetAccess/Login").unwrap();
    let mut jar = Jar::default();
    jar.store(
        &url,
        &[
            "AUTH_SESSION_ID=a; Domain=.titech.ac.jp; Path=/; HttpOnly".into(),
            "wide=1; Domain=ac.jp; Path=/".into(),
            "gone=1; Path=/; Max-Age=0".into(),
        ],
    );
    let kyomu = Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx").unwrap();
    assert_eq!(jar.header(&kyomu).as_deref(), Some("AUTH_SESSION_ID=a"));
    // Identity cookies are session cookies: a saved jar must keep them.
    let restored = Jar::load(&jar.save().unwrap()).unwrap();
    assert_eq!(
        restored.header(&kyomu).as_deref(),
        Some("AUTH_SESSION_ID=a")
    );
}

#[test]
fn a_snapshot_keeps_the_last_value_and_writes_a_lifetime_as_an_absolute_expiry() {
    let now = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
        .unwrap()
        .to_utc();
    let from = |line: &str| WebCookie {
        url: "https://isct.ex-tic.com/auth/session".into(),
        set_cookie: line.into(),
    };
    let snapshot = Snapshot::from_responses(
        vec![
            from("_exgen_idp_session=anon; path=/; secure"),
            from("_exgen_idp_session=signed; path=/; secure"),
            from("dropped=x; path=/; Max-Age=0"),
            from("kept=1; path=/; Max-Age=60"),
        ],
        now,
    );
    let lines: Vec<_> = snapshot
        .cookies
        .iter()
        .map(|cookie| cookie.set_cookie.as_str())
        .collect();
    assert_eq!(
        lines,
        [
            "_exgen_idp_session=signed; path=/; secure",
            "kept=1; path=/; Expires=Sun, 01 Nov 2026 00:01:00 GMT"
        ]
    );
}

// The barrier waits for every permit, and no permit is issued while it runs.
#[tokio::test]
async fn the_account_barrier_drains_permits_before_the_next_generation() {
    let owner = Owner::new();
    let permit = owner.permit().unwrap();
    assert_eq!(permit.generation(), 1);
    let closing = {
        let owner = owner.clone();
        tokio::spawn(async move {
            let closed = owner.close().await;
            let during = owner.permit().is_none();
            drop(closed);
            during
        })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(permit.is_cancelled(), "the barrier cancels the reads");
    assert!(!closing.is_finished(), "it waits for the permit");
    drop(permit);
    assert!(closing.await.unwrap());
    assert_eq!(owner.permit().unwrap().generation(), 2);
}

/// Science Tokyo as the session and web operation suites need it, without a
/// network: the LMS, Extic's password and app one-time-password steps, and
/// the SAML trip back to the LMS. It counts the passwords it receives.
#[derive(Default)]
pub(crate) struct School {
    pub(crate) state: Mutex<SchoolState>,
    /// Every request waits forever while set: a school that never answers.
    pub(crate) hang: AtomicBool,
}

#[derive(Default)]
pub(crate) struct SchoolState {
    /// The password Extic accepts.
    pub(crate) password: String,
    /// The Extic session the school accepts; `None` once it ran out.
    pub(crate) session: Option<String>,
    issued: u64,
    /// Password posts Extic received.
    pub(crate) passwords: usize,
    pub(crate) requests: usize,
    /// Every request fails as offline.
    pub(crate) down: bool,
}

impl School {
    pub(crate) fn accepting(password: &str) -> Arc<Self> {
        let school = Self::default();
        school.state.lock().unwrap().password = password.into();
        Arc::new(school)
    }

    pub(crate) fn passwords(&self) -> usize {
        self.state.lock().unwrap().passwords
    }

    pub(crate) fn requests(&self) -> usize {
        self.state.lock().unwrap().requests
    }

    fn answer(&self, request: &NetRequest) -> Result<Response, NetError> {
        let mut school = self.state.lock().unwrap();
        school.requests += 1;
        if school.down {
            return Err(NetError::Offline);
        }
        let cookie = request
            .headers
            .iter()
            .find(|(name, _)| *name == reqwest::header::COOKIE)
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let field = |name: &str| match &request.body {
            crate::net::Body::Form(fields) => fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone()),
            _ => None,
        };
        let post = request.method == reqwest::Method::POST;
        let url = request.url.clone();
        Ok(match (url.host_str().unwrap_or(""), post, url.path()) {
            ("lms.s.isct.ac.jp", false, _) if cookie.contains("MoodleSession=signed") => page(
                &url,
                r#"<a href="https://lms.s.isct.ac.jp/login/logout.php">Log out</a>"#,
                None,
            ),
            ("lms.s.isct.ac.jp", false, _) => redirect(&url, EXTIC_SESSION, None),
            ("lms.s.isct.ac.jp", true, path) if path.contains("/auth/saml2/sp/saml2-acs.php") => {
                redirect(
                    &url,
                    login::LMS_HOME,
                    Some("MoodleSession=signed; path=/; secure"),
                )
            }
            ("isct.ex-tic.com", false, "/auth/session") => {
                let signed = school
                    .session
                    .as_ref()
                    .is_some_and(|session| cookie.contains(&format!("_idp={session}")));
                if signed {
                    page(&url, SAML_FORM, None)
                } else {
                    page(&url, LOGIN_FORM, None)
                }
            }
            ("isct.ex-tic.com", false, "/auth/session/second_factor") => {
                page(&url, TOTP_FORM, None)
            }
            ("isct.ex-tic.com", true, "/auth/session") => {
                school.passwords += 1;
                if field("password").as_deref() == Some(school.password.as_str()) {
                    page(&url, TOTP_FORM, Some("_pending=1; path=/; secure"))
                } else {
                    status(&url, 401)
                }
            }
            ("isct.ex-tic.com", true, "/auth/session/second_factor") => {
                school.issued += 1;
                let session = format!("t{}", school.issued);
                let line = format!("_idp={session}; path=/; secure");
                school.session = Some(session);
                redirect(&url, EXTIC_SESSION, Some(&line))
            }
            _ => status(&url, 404),
        })
    }
}

const EXTIC_SESSION: &str = "https://isct.ex-tic.com/auth/session";
const LOGIN_FORM: &str = r#"<form id="login" method="post" action="https://isct.ex-tic.com/auth/session"><input name="identifier"><input name="password" type="password"></form>"#;
const TOTP_FORM: &str = r#"<form id="totp-form" method="post" action="https://isct.ex-tic.com/auth/session/second_factor"><input name="totp"></form>"#;
const SAML_FORM: &str = r#"<form method="post" action="https://lms.s.isct.ac.jp/auth/saml2/sp/saml2-acs.php/lms.s.isct.ac.jp"><input type="hidden" name="SAMLResponse" value="assertion"></form>"#;

fn response(url: &Url, code: u16, cookie: Option<&str>) -> Response {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(cookie) = cookie {
        headers.append(reqwest::header::SET_COOKIE, cookie.parse().unwrap());
    }
    Response {
        status: code,
        url: url.clone(),
        headers,
        body: bytes::Bytes::new(),
    }
}

fn page(url: &Url, html: &str, cookie: Option<&str>) -> Response {
    let mut response = response(url, 200, cookie);
    response.headers.insert(
        reqwest::header::CONTENT_TYPE,
        "text/html; charset=utf-8".parse().unwrap(),
    );
    response.body = bytes::Bytes::from(html.to_owned());
    response
}

fn redirect(url: &Url, to: &str, cookie: Option<&str>) -> Response {
    let mut response = response(url, 302, cookie);
    response
        .headers
        .insert(reqwest::header::LOCATION, to.parse().unwrap());
    response
}

fn status(url: &Url, code: u16) -> Response {
    response(url, code, None)
}

impl Transport for School {
    fn send(&self, request: NetRequest) -> BoxFuture<'_, Result<Response, NetError>> {
        Box::pin(async move {
            if self.hang.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            self.answer(&request)
        })
    }

    fn report_maintenance(&self, _host: Host) {}

    fn paused_until(&self, _host: Host) -> Option<i64> {
        None
    }
}

/// A web view's cookie store that records what Core asked of it. A write
/// waits at its gate while the gate is shut.
pub(crate) struct WebStore {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) writes: Mutex<Vec<Vec<WebCookie>>>,
    pub(crate) clears: Mutex<Vec<WebClear>>,
    pub(crate) by_site: bool,
    gate: tokio::sync::Semaphore,
}

impl WebStore {
    pub(crate) fn open(by_site: bool) -> Arc<Self> {
        let store = Self::shut(by_site);
        store.release();
        store
    }

    fn shut(by_site: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            writes: Mutex::default(),
            clears: Mutex::default(),
            by_site,
            gate: tokio::sync::Semaphore::new(0),
        })
    }

    /// Lets every write through from now on.
    fn release(&self) {
        self.gate
            .add_permits(tokio::sync::Semaphore::MAX_PERMITS / 2);
    }
}

impl WebCookieSeed for WebStore {
    fn write(&self, cookies: Vec<WebCookie>) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let _pass = self
                .gate
                .acquire()
                .await
                .map_err(|error| error.to_string())?;
            self.calls.lock().unwrap().push("write".into());
            self.writes.lock().unwrap().push(cookies);
            Ok(())
        })
    }

    fn clear(&self, scope: WebClear) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("clear".into());
            self.clears.lock().unwrap().push(scope);
            Ok(())
        })
    }

    fn clears_by_site(&self) -> bool {
        self.by_site
    }
}

/// A restored, unverified identity holding Extic session `session`.
fn restored(sessions: &SchoolSessions, session: &str) -> u64 {
    sessions
        .0
        .account(SchoolAuthProfile::Science)
        .identity
        .restore(
            Snapshot {
                cookies: vec![WebCookie {
                    url: EXTIC_SESSION.into(),
                    set_cookie: format!("_idp={session}; path=/; secure"),
                }],
            },
            "user".into(),
        )
        .unwrap()
}

fn science_state(sessions: &SchoolSessions) -> SchoolSessionState {
    sessions.snapshot().profiles.science.state
}

// A restored identity is only unverified. Verifying it is one SSO from a new
// jar holding it, and no password: the school's answer makes it a new
// verified generation (with what the school set on the way), finds it
// expired, or, without an answer, leaves it unverified.
#[tokio::test]
async fn a_verification_is_one_sso_from_the_snapshot_and_sends_no_password() {
    let school = School::accepting("right");
    school.state.lock().unwrap().session = Some("t9".into());
    let auth = science_auth();
    let sessions = SchoolSessions::with_transport(school.clone(), auth.clone());
    let restored_generation = restored(&sessions, "t9");
    assert_eq!(science_state(&sessions), SchoolSessionState::Unverified);
    assert_eq!(sessions.fresh(SchoolAuthProfile::Science), None);
    let checked = sessions.0.verify(SchoolAuthProfile::Science).await;
    let Checked::Verified(generation) = checked else {
        panic!("not verified: {checked:?}");
    };
    assert!(
        generation > restored_generation,
        "a new verified generation"
    );
    assert_eq!(science_state(&sessions), SchoolSessionState::Active);
    assert_eq!(sessions.fresh(SchoolAuthProfile::Science), Some(generation));
    // Fresh: used as it is, without asking the school again.
    let asked = school.requests();
    assert_eq!(
        sessions.0.verify(SchoolAuthProfile::Science).await,
        Checked::Verified(generation)
    );
    assert_eq!(school.requests(), asked);

    // The school's session ran out: the generation is invalid, and nothing
    // is left to write or to read with.
    let expired = SchoolSessions::with_transport(school.clone(), auth.clone());
    restored(&expired, "t1");
    assert_eq!(
        expired.0.verify(SchoolAuthProfile::Science).await,
        Checked::Expired
    );
    assert_eq!(science_state(&expired), SchoolSessionState::Idle);
    assert!(expired
        .0
        .account(SchoolAuthProfile::Science)
        .identity
        .snapshot()
        .is_none());

    // No answer: still unverified.
    school.state.lock().unwrap().down = true;
    let offline = SchoolSessions::with_transport(school.clone(), auth);
    restored(&offline, "t9");
    assert_eq!(
        offline.0.verify(SchoolAuthProfile::Science).await,
        Checked::Unconfirmed
    );
    assert_eq!(science_state(&offline), SchoolSessionState::Unverified);
    assert_eq!(
        school.passwords(),
        0,
        "a verification never sends a password"
    );
}

// One deadline bounds a verification, however long the school takes.
#[tokio::test(start_paused = true)]
async fn a_verification_the_school_never_answers_ends_at_its_deadline() {
    let school = School::accepting("right");
    school.hang.store(true, Ordering::SeqCst);
    let sessions = SchoolSessions::with_transport(school, science_auth());
    restored(&sessions, "t9");
    let started = tokio::time::Instant::now();
    assert_eq!(
        sessions.0.verify(SchoolAuthProfile::Science).await,
        Checked::Unconfirmed
    );
    assert_eq!(started.elapsed(), VERIFY_DEADLINE);
    assert_eq!(science_state(&sessions), SchoolSessionState::Unverified);
}

// The queue never moves past a call the platform has not answered: the
// caller stops waiting after SEED_WAIT, the store becomes unusable after
// WEB_STORE_LIMIT, and the clear queued behind runs only once the write
// answers. A write queued before an account's admission closed is dropped.
#[tokio::test(start_paused = true)]
async fn the_web_queue_never_lets_a_later_call_overtake_an_unanswered_one() {
    let store = WebStore::shut(true);
    let queue = WebQueue::default();
    assert!(queue.attach(store.clone()));
    let tasks = RuntimeTasks::default();
    let cookie = || {
        vec![WebCookie {
            url: EXTIC_SESSION.into(),
            set_cookie: "_idp=t1; path=/".into(),
        }]
    };
    let mut first = queue.write(
        &tasks,
        SchoolAuthProfile::Science,
        cookie(),
        Box::new(|| true),
    );
    assert!(
        tokio::time::timeout(SEED_WAIT, &mut first).await.is_err(),
        "the caller stops waiting; the write stays queued"
    );
    queue.close_admission(&[SchoolAuthProfile::Science]);
    let mut clear = queue.clear(&tasks, WebClear::account(SchoolAuthProfile::Science));
    tokio::time::sleep(WEB_STORE_LIMIT).await;
    assert!(!queue.usable(), "past its limit the store takes no write");
    let refused = queue.write(
        &tasks,
        SchoolAuthProfile::TokyoTech,
        cookie(),
        Box::new(|| true),
    );
    assert_eq!(refused.await.unwrap(), Err(WebError::Unavailable));
    assert!(
        tokio::time::timeout(Duration::from_secs(60), &mut clear)
            .await
            .is_err(),
        "the clear waits behind the unanswered write"
    );
    assert!(store.clears.lock().unwrap().is_empty());
    store.release();
    assert_eq!(first.await.unwrap(), Ok(()));
    assert_eq!(clear.await.unwrap(), Ok(()));
    assert_eq!(*store.calls.lock().unwrap(), ["write", "clear"]);
    assert_eq!(queue.storage(), 1, "a confirmed clear moves the store on");
    assert!(queue.usable());

    // Queued under the old admission, or no longer the identity to write:
    // answered without reaching the platform.
    let stale = queue.write(
        &tasks,
        SchoolAuthProfile::Science,
        cookie(),
        Box::new(|| true),
    );
    queue.close_admission(&[SchoolAuthProfile::Science]);
    assert_eq!(stale.await.unwrap(), Err(WebError::Stale));
    let replaced = queue.write(
        &tasks,
        SchoolAuthProfile::Science,
        cookie(),
        Box::new(|| false),
    );
    assert_eq!(replaced.await.unwrap(), Err(WebError::Stale));
    assert_eq!(store.writes.lock().unwrap().len(), 1);
}

// Core's shutdown ends the queue at once when it is idle, and waits for a
// call the platform was handed only until that call's limit.
#[tokio::test(start_paused = true)]
async fn the_web_queue_ends_with_core() {
    let idle = WebQueue::default();
    idle.attach(WebStore::open(true));
    let tasks = RuntimeTasks::default();
    let written = idle.write(
        &tasks,
        SchoolAuthProfile::Science,
        Vec::new(),
        Box::new(|| true),
    );
    assert_eq!(written.await.unwrap(), Ok(()));
    let started = tokio::time::Instant::now();
    tasks.close();
    tasks.wait().await;
    assert_eq!(started.elapsed(), Duration::ZERO);

    let busy = WebQueue::default();
    busy.attach(WebStore::shut(true));
    let tasks = RuntimeTasks::default();
    let _pending = busy.write(
        &tasks,
        SchoolAuthProfile::Science,
        Vec::new(),
        Box::new(|| true),
    );
    tokio::task::yield_now().await;
    let started = tokio::time::Instant::now();
    tasks.close();
    tasks.wait().await;
    assert_eq!(started.elapsed(), WEB_STORE_LIMIT);
}

// An account's clear covers its own websites (adpt-isct's table) only.
#[test]
fn a_clear_covers_its_accounts_websites_only() {
    let science = WebClear::account(SchoolAuthProfile::Science);
    let tokyo = WebClear::account(SchoolAuthProfile::TokyoTech);
    for domain in [
        "lms.s.isct.ac.jp",
        ".isct.ac.jp",
        "isct.ac.jp",
        "isct.ex-tic.com",
    ] {
        assert!(science.covers(domain), "{domain}");
        assert!(!tokyo.covers(domain), "{domain}");
        assert!(WebClear::AllSchool.covers(domain), "{domain}");
    }
    for domain in [
        "portal.nap.gsic.titech.ac.jp",
        ".titech.ac.jp",
        "kyomu0.gakumu.titech.ac.jp",
    ] {
        assert!(tokyo.covers(domain), "{domain}");
        assert!(!science.covers(domain), "{domain}");
        assert!(WebClear::AllSchool.covers(domain), "{domain}");
    }
    for domain in ["notisct.ac.jp", "example.com", "ac.jp"] {
        assert!(!WebClear::AllSchool.covers(domain), "{domain}");
    }
}

#[derive(Clone, Default)]
pub(crate) struct MemoryVault(Arc<Mutex<std::collections::HashMap<String, String>>>);

impl crate::ports::SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), crate::ports::SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, crate::ports::SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<(), crate::ports::SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

/// A school auth service with the Science Tokyo account saved.
fn science_auth() -> Arc<SchoolAuthService> {
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    auth.stage(
        SchoolAuthProfile::Science,
        SchoolCredentials {
            username: "user".into(),
            password: "right".into(),
            otp_secret: "JBSWY3DPEHPK3PXP".into(),
            matrix: String::new(),
        },
    )
    .unwrap();
    auth
}
