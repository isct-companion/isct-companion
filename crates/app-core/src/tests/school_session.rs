//! The session rules that need no school: the jar, the identity snapshot,
//! the account barrier and the web cookie queue. Sign-ins, verification and
//! reads are not simulated (`approach/school-sign-in-verification`): they
//! run against the real schools (`school_session__login.rs`, ignored by
//! default), in the dev app, and in `docs/verification/manual-checks.yaml`.
use super::*;
use std::time::Duration;

const EXTIC_SESSION: &str = "https://isct.ex-tic.com/auth/session";

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

/// A web view's cookie store that records what Core asked of it. A write
/// waits at its gate while the gate is shut.
struct WebStore {
    calls: Mutex<Vec<String>>,
    writes: Mutex<Vec<Vec<WebCookie>>>,
    /// Clears of every school website.
    clears: Mutex<usize>,
    gate: tokio::sync::Semaphore,
}

impl WebStore {
    fn open() -> Arc<Self> {
        let store = Self::shut();
        store.release();
        store
    }

    fn shut() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::default(),
            writes: Mutex::default(),
            clears: Mutex::default(),
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

    fn clear(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push("clear".into());
            *self.clears.lock().unwrap() += 1;
            Ok(())
        })
    }
}

// The queue never moves past a call the platform has not answered: the
// caller stops waiting after SEED_WAIT, the store becomes unusable after
// WEB_STORE_LIMIT, and the clear queued behind runs only once the write
// answers. A write queued before an account's admission closed is dropped.
#[tokio::test(start_paused = true)]
async fn the_web_queue_never_lets_a_later_call_overtake_an_unanswered_one() {
    let store = WebStore::shut();
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
    let mut clear = queue.clear(&tasks);
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
    assert_eq!(*store.clears.lock().unwrap(), 0);
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
    idle.attach(WebStore::open());
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
    busy.attach(WebStore::shut());
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

// A clear covers both accounts' websites (adpt-isct's table), and no
// other site.
#[test]
fn a_clear_covers_every_school_website_and_no_other() {
    for domain in [
        "lms.s.isct.ac.jp",
        ".isct.ac.jp",
        "isct.ac.jp",
        "isct.ex-tic.com",
        "portal.nap.gsic.titech.ac.jp",
        ".titech.ac.jp",
        "kyomu0.gakumu.titech.ac.jp",
    ] {
        assert!(is_school_domain(domain), "{domain}");
    }
    for domain in ["notisct.ac.jp", "example.com", "ac.jp"] {
        assert!(!is_school_domain(domain), "{domain}");
    }
}
