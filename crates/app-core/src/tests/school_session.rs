//! The session rules: the jar, the identity snapshot, the account barrier,
//! verification, sign-ins and the Connect rules, and the web cookie queue,
//! over a scripted Science Tokyo (`School`, which the web operation suite
//! uses too). The real sign-in
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

// The barrier waits for every permit, no permit (not even one of the
// running generation, for a vault write) is issued while it runs, and it
// reopens one generation on.
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
    assert!(
        owner.permit_of(1).is_none(),
        "no write begins once it started"
    );
    assert!(!closing.is_finished(), "it waits for the permit");
    drop(permit);
    assert!(closing.await.unwrap());
    assert_eq!(owner.permit().unwrap().generation(), 2);
    assert!(owner.permit_of(1).is_none());
}

/// Science Tokyo as the session and web operation suites need it, without a
/// network: the LMS, Extic's password and app one-time-password steps, and
/// the SAML trip back to the LMS. It counts the passwords it receives.
#[derive(Default)]
pub(crate) struct School {
    pub(crate) state: Mutex<SchoolState>,
    /// Every request waits forever while set: a school that never answers.
    pub(crate) hang: AtomicBool,
    /// Requests left waiting by `hang`.
    pub(crate) hung: std::sync::atomic::AtomicUsize,
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
    /// An accepted password is answered with the emailed code's page.
    pub(crate) email_otp: bool,
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
                if field("password").as_deref() != Some(school.password.as_str()) {
                    status(&url, 401)
                } else if school.email_otp {
                    page(&url, EMAIL_OTP_FORM, Some("_pending=1; path=/; secure"))
                } else {
                    page(&url, TOTP_FORM, Some("_pending=1; path=/; secure"))
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
const EMAIL_OTP_FORM: &str =
    r#"<div id="emailotp-form-wrapper"><form><input name="code"></form></div>"#;
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
                self.hung.fetch_add(1, Ordering::SeqCst);
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
        .restore(Snapshot {
            cookies: vec![WebCookie {
                url: EXTIC_SESSION.into(),
                set_cookie: format!("_idp={session}; path=/; secure"),
            }],
        })
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
    assert_eq!(science_state(&sessions), SchoolSessionState::Normal);
    assert_eq!(sessions.fresh(SchoolAuthProfile::Science), None);
    let checked = sessions.0.verify(SchoolAuthProfile::Science).await;
    let Checked::Verified(generation) = checked else {
        panic!("not verified: {checked:?}");
    };
    assert!(
        generation > restored_generation,
        "a new verified generation"
    );
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
    assert_eq!(offline.fresh(SchoolAuthProfile::Science), None);
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
    assert_eq!(sessions.fresh(SchoolAuthProfile::Science), None);
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

/// Science Tokyo credentials with `password`.
pub(crate) fn science_credentials(password: &str) -> SchoolCredentials {
    SchoolCredentials {
        username: "user".into(),
        password: password.into(),
        otp_secret: "JBSWY3DPEHPK3PXP".into(),
        matrix: String::new(),
    }
}

/// A school auth service with the Science Tokyo account saved (verified)
/// with password `right`.
fn science_auth() -> Arc<SchoolAuthService> {
    saved_auth("right")
}

/// A school auth service with the Science Tokyo account saved (verified)
/// with `password`.
pub(crate) fn saved_auth(password: &str) -> Arc<SchoolAuthService> {
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    auth.promote(SchoolAuthProfile::Science, &science_credentials(password))
        .unwrap();
    auth
}

/// A sign-in on Core's own, as a read that found no identity starts it.
async fn background(sessions: &SchoolSessions) -> Result<u64, Ended> {
    sessions
        .0
        .sign_in(SchoolAuthProfile::Science, 0, identity::How::Background)
        .await
}

fn science(sessions: &SchoolSessions) -> SchoolProfileSession {
    sessions.snapshot().profiles.science
}

// The person's Connect is authoritative: it replaces a sign-in already
// running at once, the replaced sign-in's result changes nothing, and a
// caller waiting for it waits for the Connect's instead. The school's
// acceptance makes the input the verified credentials.
#[tokio::test(start_paused = true)]
async fn a_connect_replaces_the_running_sign_in_and_only_its_own_result_applies() {
    let school = School::accepting("new");
    let auth = science_auth();
    let sessions = SchoolSessions::with_transport(school.clone(), auth.clone());
    school.hang.store(true, Ordering::SeqCst);
    let waiting = {
        let sessions = sessions.clone();
        tokio::spawn(async move { background(&sessions).await })
    };
    while school.hung.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(science_state(&sessions), SchoolSessionState::SigningIn);
    school.hang.store(false, Ordering::SeqCst);
    let outcome = sessions
        .connect(SchoolAuthProfile::Science, science_credentials("new"))
        .await;
    assert_eq!(outcome, SchoolConnectOutcome::Connected);
    let generation = waiting.await.unwrap().expect("the Connect's identity");
    assert_eq!(sessions.fresh(SchoolAuthProfile::Science), Some(generation));
    assert_eq!(school.passwords(), 1, "only the Connect sent a password");
    assert_eq!(
        auth.load(SchoolAuthProfile::Science)
            .unwrap()
            .unwrap()
            .password,
        "new"
    );
    assert_eq!(science_state(&sessions), SchoolSessionState::Normal);
}

// The school's refusal of the saved credentials is kept with exactly them,
// across launches: Core sends them no more on its own, and the account
// shows it, until the person retries or the credentials change. The
// identity and token Core already holds are not touched.
#[tokio::test(start_paused = true)]
async fn a_refusal_of_the_saved_credentials_is_kept_with_them() {
    let school = School::accepting("right");
    let vault = MemoryVault::default();
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    auth.promote(SchoolAuthProfile::Science, &science_credentials("old"))
        .unwrap();
    let sessions = SchoolSessions::with_transport(school.clone(), auth);
    let refused = background(&sessions).await;
    assert!(
        matches!(&refused, Err(Ended::Refused(problem)) if problem.kind == SchoolSignInProblemKind::InvalidCredentials),
        "{refused:?}"
    );
    assert_eq!(science_state(&sessions), SchoolSessionState::Refused);
    assert_eq!(background(&sessions).await, refused);
    assert_eq!(
        school.passwords(),
        1,
        "a refused password is not sent again"
    );

    // A relaunch reads the refusal back and still sends nothing.
    let auth = Arc::new(SchoolAuthService::new(vault));
    let relaunched = SchoolSessions::with_transport(school.clone(), auth.clone());
    relaunched.0.restore(SchoolAuthProfile::Science).await;
    assert_eq!(science_state(&relaunched), SchoolSessionState::Refused);
    assert!(matches!(
        background(&relaunched).await,
        Err(Ended::Refused(_))
    ));
    assert_eq!(school.passwords(), 1);

    // Retry sends them once more, through the refusal.
    let retried = relaunched.retry(SchoolAuthProfile::Science).await;
    assert_eq!(retried.profiles.science.state, SchoolSessionState::Refused);
    assert_eq!(school.passwords(), 2);

    // Other credentials (here imported from another device) are not
    // refused: the account is normal, and the next sign-in uses them.
    auth.import_sync_payload(
        &serde_json::json!({"version": 1, "accounts": {"science": science_credentials("right")}})
            .to_string(),
    )
    .unwrap();
    relaunched.credentials_changed(SchoolAuthProfile::Science);
    assert_eq!(science_state(&relaunched), SchoolSessionState::Normal);
    assert!(background(&relaunched).await.is_ok());
    assert_eq!(school.passwords(), 3);
}

// A Connect with credentials other than the saved ones that the school
// refuses, or that hits a page needing the person, or that gets no answer,
// is the Connect form's to show: the saved credentials, the account's
// state and every fact stay as they were. Connect with the saved
// credentials themselves, refused, keeps the refusal on them.
#[tokio::test(start_paused = true)]
async fn a_connect_that_fails_changes_nothing_but_a_refusal_of_the_saved_credentials() {
    let school = School::accepting("right");
    let auth = science_auth();
    let sessions = SchoolSessions::with_transport(school.clone(), auth.clone());
    let saved = || auth.load(SchoolAuthProfile::Science).unwrap().unwrap();

    let refused = sessions
        .connect(SchoolAuthProfile::Science, science_credentials("typo"))
        .await;
    assert_eq!(
        refused,
        SchoolConnectOutcome::Refused {
            reason: SchoolSignInProblemKind::InvalidCredentials
        }
    );
    assert_eq!(science(&sessions).state, SchoolSessionState::Normal);
    assert_eq!(saved().password, "right");
    assert_eq!(auth.refusal(SchoolAuthProfile::Science), None);

    school.state.lock().unwrap().email_otp = true;
    let emailed = sessions
        .connect(SchoolAuthProfile::Science, science_credentials("right"))
        .await;
    assert_eq!(
        emailed,
        SchoolConnectOutcome::NeedsWeb {
            reason: SchoolSignInProblemKind::EmailOtp
        }
    );
    assert_eq!(
        science(&sessions).state,
        SchoolSessionState::Normal,
        "only a sign-in on Core's own makes the account need the web"
    );
    school.state.lock().unwrap().email_otp = false;

    sessions.network(crate::sync_engine::Network::Offline);
    let offline = sessions
        .connect(SchoolAuthProfile::Science, science_credentials("right"))
        .await;
    assert_eq!(offline, SchoolConnectOutcome::Unreachable);
    assert_eq!(science(&sessions).state, SchoolSessionState::Normal);
    assert_eq!(science(&sessions).retry_at, None, "a Connect never pauses");
    sessions.network(crate::sync_engine::Network::Wifi);

    // The school no longer takes the saved password: Connect with it is
    // refused, and the refusal stays with it.
    school.state.lock().unwrap().password = "changed".into();
    let stale = sessions
        .connect(SchoolAuthProfile::Science, science_credentials("right"))
        .await;
    assert!(matches!(stale, SchoolConnectOutcome::Refused { .. }));
    assert_eq!(science(&sessions).state, SchoolSessionState::Refused);
    assert_eq!(saved().password, "right");

    // Connect with what the school takes now replaces them.
    let connected = sessions
        .connect(SchoolAuthProfile::Science, science_credentials("changed"))
        .await;
    assert_eq!(connected, SchoolConnectOutcome::Connected);
    assert_eq!(science(&sessions).state, SchoolSessionState::Normal);
    assert_eq!(saved().password, "changed");
}

// The school accepts a Connect and the vault refuses to save its
// credentials: the Connect says so instead of `Connected`, so the form keeps
// what was typed; the saved credentials are as they were, and the identity
// the school gave stands.
#[tokio::test(start_paused = true)]
async fn a_connect_the_vault_did_not_save_says_so() {
    #[derive(Clone, Default)]
    struct Refusing(MemoryVault, Arc<AtomicBool>);
    impl crate::ports::SecretVault for Refusing {
        fn store(&self, account: &str, secret: &str) -> Result<(), crate::ports::SecretError> {
            if account == "school-accounts" && self.1.load(Ordering::SeqCst) {
                return Err(crate::ports::SecretError::Access("denied".into()));
            }
            self.0.store(account, secret)
        }
        fn load(&self, account: &str) -> Result<Option<String>, crate::ports::SecretError> {
            self.0.load(account)
        }
        fn delete(&self, account: &str) -> Result<(), crate::ports::SecretError> {
            self.0.delete(account)
        }
    }
    let vault = Refusing::default();
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    auth.promote(SchoolAuthProfile::Science, &science_credentials("old"))
        .unwrap();
    vault.1.store(true, Ordering::SeqCst);
    let sessions = SchoolSessions::with_transport(School::accepting("new"), auth.clone());
    assert_eq!(
        sessions
            .connect(SchoolAuthProfile::Science, science_credentials("new"))
            .await,
        SchoolConnectOutcome::NotSaved
    );
    assert_eq!(
        auth.load(SchoolAuthProfile::Science)
            .unwrap()
            .unwrap()
            .password,
        "old"
    );
    assert!(sessions.fresh(SchoolAuthProfile::Science).is_some());
}

// A sign-in on Core's own that hits a page needing the person makes the
// account need the web, kept with the saved credentials; one without a
// verdict pauses sign-ins on Core's own until its retry time, which only
// the person's Retry (or Reload on the sign-in page) goes through, and the
// shell's wake after it lifts.
#[tokio::test(start_paused = true)]
async fn a_sign_in_on_cores_own_needs_the_web_or_pauses() {
    let school = School::accepting("right");
    school.state.lock().unwrap().email_otp = true;
    let sessions = SchoolSessions::with_transport(school.clone(), science_auth());
    assert!(matches!(
        background(&sessions).await,
        Err(Ended::Refused(_))
    ));
    let session = science(&sessions);
    assert_eq!(session.state, SchoolSessionState::NeedsWeb);
    assert_eq!(
        session.problem.map(|problem| problem.kind),
        Some(SchoolSignInProblemKind::EmailOtp)
    );

    let school = School::accepting("right");
    school.state.lock().unwrap().down = true;
    let sessions = SchoolSessions::with_transport(school.clone(), science_auth());
    assert!(matches!(
        background(&sessions).await,
        Err(Ended::Unavailable(_))
    ));
    let session = science(&sessions);
    assert_eq!(session.state, SchoolSessionState::Unavailable);
    assert!(session.retry_at.is_some());
    let asked = school.requests();
    assert!(matches!(
        background(&sessions).await,
        Err(Ended::Unavailable(_))
    ));
    assert_eq!(school.requests(), asked, "the pause holds it back");

    school.state.lock().unwrap().down = false;
    tokio::time::advance(RETRY_NOT_BEFORE).await;
    sessions.wake();
    assert_eq!(science_state(&sessions), SchoolSessionState::Normal);
    assert!(background(&sessions).await.is_ok());
}

/// One request to the LMS home: with the site session's cookies, or bare,
/// as the token REST service sends its own credentials.
struct Home {
    cookies: bool,
}

impl Operation for Home {
    type Output = u16;
    type Error = NetError;

    fn attempt(&mut self, attempt: Attempt) -> BoxFuture<'_, Result<u16, NetError>> {
        let cookies = self.cookies;
        Box::pin(async move {
            let request = NetRequest::get(Host::Moodle, Url::parse(login::LMS_HOME).unwrap());
            let response = if cookies {
                attempt.send(request).await?
            } else {
                attempt.bare(request).await?
            };
            Ok(response.status)
        })
    }
}

async fn read_home(sessions: &SchoolSessions, cookies: bool) -> Result<u16, RunError<NetError>> {
    let permit = sessions.permit(SchoolAuthProfile::Science).unwrap();
    let deadline = tokio::time::Instant::now() + crate::sync_engine::KEY_DEADLINE;
    sessions
        .run(Site::Lms, &permit, deadline, &mut Home { cookies })
        .await
}

// A read whose requests carry their own credentials (the LMS token) needs
// no site session: it builds none, takes no SSO and signs in nowhere, even
// for an account the school refused. Only a request that needs cookies gets
// the instance; when the account cannot sign in, that read ends with what
// lets the account's reads go on: the person, a pause, the network.
#[tokio::test(start_paused = true)]
async fn a_read_gets_the_site_session_only_when_a_request_needs_cookies() {
    use crate::sync_engine::AccountWait;
    let school = School::accepting("right");
    let sessions = SchoolSessions::with_transport(school.clone(), saved_auth("old"));
    assert_eq!(read_home(&sessions, false).await.unwrap(), 302);
    assert_eq!((school.requests(), school.passwords()), (1, 0));
    assert!(matches!(
        read_home(&sessions, true).await,
        Err(RunError::Blocked(AccountWait::Person))
    ));
    assert_eq!(school.passwords(), 1, "no identity: one sign-in, refused");
    assert!(matches!(
        read_home(&sessions, true).await,
        Err(RunError::Blocked(AccountWait::Person))
    ));
    assert_eq!(school.passwords(), 1, "the refusal holds the next one back");
    assert_eq!(read_home(&sessions, false).await.unwrap(), 302);

    let down = School::accepting("right");
    down.state.lock().unwrap().down = true;
    let paused = SchoolSessions::with_transport(down, science_auth());
    assert!(matches!(
        read_home(&paused, true).await,
        Err(RunError::Blocked(AccountWait::Paused { .. }))
    ));

    let offline = SchoolSessions::with_transport(School::accepting("right"), science_auth());
    offline.network(crate::sync_engine::Network::Offline);
    assert!(matches!(
        read_home(&offline, true).await,
        Err(RunError::Blocked(AccountWait::Network))
    ));
}

/// A vault whose writes of some items wait while it holds them.
#[derive(Clone, Default)]
struct HeldVault {
    inner: MemoryVault,
    prefix: &'static str,
    holding: Arc<(Mutex<bool>, std::sync::Condvar)>,
    /// Told when a write starts waiting. An async signal, never a blocking
    /// wait: under a paused clock a blocking task stops time, and a sign-in
    /// waiting for a fresh one-time password window would never get one.
    waiting: Arc<tokio::sync::Notify>,
}

impl HeldVault {
    /// Holds the writes of the items whose account starts with `prefix`.
    fn on(prefix: &'static str) -> Self {
        Self {
            prefix,
            ..Self::default()
        }
    }

    fn hold(&self, holding: bool) {
        let (lock, signal) = &*self.holding;
        *lock.lock().unwrap() = holding;
        signal.notify_all();
    }

    /// Until a write is held.
    async fn wait_for_a_write(&self) {
        self.waiting.notified().await;
    }
}

impl crate::ports::SecretVault for HeldVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), crate::ports::SecretError> {
        if account.starts_with(self.prefix) {
            let (lock, signal) = &*self.holding;
            let mut holding = lock.lock().unwrap();
            if *holding {
                self.waiting.notify_one();
            }
            while *holding {
                holding = signal.wait(holding).unwrap();
            }
        }
        self.inner.store(account, secret)
    }

    fn load(&self, account: &str) -> Result<Option<String>, crate::ports::SecretError> {
        self.inner.load(account)
    }

    fn delete(&self, account: &str) -> Result<(), crate::ports::SecretError> {
        self.inner.delete(account)
    }
}

// A vault write a read started holds the account's permit until it lands,
// even once the read was cancelled and dropped: signing out waits for it,
// and its clean-up comes after it, so nothing of the account is left.
#[tokio::test]
async fn a_vault_write_of_a_cancelled_read_lands_before_the_sign_out_cleans_up() {
    let vault = HeldVault::on("school-session");
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    auth.promote(SchoolAuthProfile::Science, &science_credentials("right"))
        .unwrap();
    let sessions = SchoolSessions::with_transport(School::accepting("right"), auth);
    restored(&sessions, "t9");
    vault.hold(true);
    let read = {
        let sessions = sessions.clone();
        tokio::spawn(async move {
            let generation = sessions.account_generation(SchoolAuthProfile::Science);
            sessions
                .0
                .persist(SchoolAuthProfile::Science, generation)
                .await
        })
    };
    vault.wait_for_a_write().await;
    read.abort();
    let signing_out = {
        let sessions = sessions.clone();
        tokio::spawn(async move { sessions.sign_out(SchoolAuthProfile::Science).await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !signing_out.is_finished(),
        "the sign-out waits for the write"
    );
    vault.hold(false);
    assert_eq!(signing_out.await.unwrap(), Ok(true));
    assert!(
        crate::ports::SecretVault::load(&vault.inner, "school-session-science")
            .unwrap()
            .is_none(),
        "the sign-out's delete came after the write"
    );
}

// The person signs out while a Connect the school already accepted is
// saving its credentials. The sign-out moves the account generation, waits
// for that write and every other operation of the account, and only then
// deletes: the account ends with no credentials, identity or session, and
// nothing of the Connect is written after.
#[tokio::test(start_paused = true)]
async fn a_sign_out_during_an_accepted_connect_deletes_after_its_write() {
    let vault = HeldVault::on("school-accounts");
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    auth.promote(SchoolAuthProfile::Science, &science_credentials("old"))
        .unwrap();
    let sessions = SchoolSessions::with_transport(School::accepting("new"), auth.clone());
    vault.hold(true);
    let connect = {
        let sessions = sessions.clone();
        tokio::spawn(async move {
            sessions
                .connect(SchoolAuthProfile::Science, science_credentials("new"))
                .await
        })
    };
    vault.wait_for_a_write().await;
    let signing_out = {
        let sessions = sessions.clone();
        tokio::spawn(async move { sessions.sign_out(SchoolAuthProfile::Science).await })
    };
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(
        !signing_out.is_finished(),
        "the sign-out waits for the Connect's write"
    );
    vault.hold(false);
    assert_eq!(signing_out.await.unwrap(), Ok(true));
    connect.await.unwrap();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    let relaunched = SchoolAuthService::new(vault.inner.clone());
    assert!(relaunched
        .load(SchoolAuthProfile::Science)
        .unwrap()
        .is_none());
    assert!(
        crate::ports::SecretVault::load(&vault.inner, "school-session-science")
            .unwrap()
            .is_none()
    );
    assert_eq!(science(&sessions).state, SchoolSessionState::NotConfigured);
    assert_eq!(sessions.fresh(SchoolAuthProfile::Science), None);
}
