use super::*;
use crate::{
    data::DataStore,
    events::CoreEvent,
    lifecycle::SchoolSite,
    ports::{EventError, EventSink, SecretError, SecretVault},
    pull_coordinator::{
        PullCoordinator, PullNetwork, PullOptions, PullPriority, PullState, PullTarget,
    },
    school_auth::{SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    school_refresh::SchoolRefresh,
    school_service::IngestOutcome,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
};
use url::Url;
use uuid::Uuid;

#[derive(Default)]
struct EmptyVault;

impl SecretVault for EmptyVault {
    fn store(&self, _account: &str, _secret: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn delete(&self, _account: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<CoreEvent>>);

impl EventSink for Events {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[derive(Default)]
struct FakeBrowser {
    listener: Mutex<Option<Weak<dyn SchoolBrowserListener>>>,
    commands: Mutex<Vec<BrowserCommand>>,
    current: Mutex<HashMap<BrowserId, Url>>,
}

impl FakeBrowser {
    fn listener(&self) -> Arc<dyn SchoolBrowserListener> {
        self.listener
            .lock()
            .unwrap()
            .as_ref()
            .and_then(Weak::upgrade)
            .unwrap()
    }

    async fn first_navigation(&self) -> BrowserCommand {
        for _ in 0..100 {
            if let Some(command) = self
                .commands
                .lock()
                .unwrap()
                .iter()
                .find(|command| matches!(command.command, BrowserAction::Navigate(_)))
                .cloned()
            {
                return command;
            }
            tokio::task::yield_now().await;
        }
        panic!("navigation was not dispatched");
    }

    async fn navigation(&self, index: usize) -> BrowserCommand {
        for _ in 0..1_000 {
            let commands: Vec<_> = self
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
                .cloned()
                .collect();
            if let Some(command) = commands.get(index) {
                return command.clone();
            }
            tokio::task::yield_now().await;
        }
        panic!("navigation {index} was not dispatched");
    }

    async fn extraction_script(&self) -> String {
        self.extraction_script_at(0).await
    }

    async fn extraction_script_at(&self, index: usize) -> String {
        for _ in 0..1_000 {
            let scripts: Vec<_> = self
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter_map(|command| match &command.command {
                    BrowserAction::Eval(script) if script.contains("isct-companion-extract://") => {
                        Some(script.clone())
                    }
                    _ => None,
                })
                .collect();
            if let Some(script) = scripts.get(index) {
                return script.clone();
            }
            tokio::task::yield_now().await;
        }
        panic!("extraction {index} was not dispatched");
    }

    async fn eval(&self, index: usize) -> String {
        for _ in 0..1_000 {
            let scripts: Vec<_> = self
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter_map(|command| match &command.command {
                    BrowserAction::Eval(script) => Some(script.clone()),
                    _ => None,
                })
                .collect();
            if let Some(script) = scripts.get(index) {
                return script.clone();
            }
            tokio::task::yield_now().await;
        }
        panic!("eval {index} was not dispatched");
    }

    fn set_current_url(&self, browser: BrowserId, url: Url) {
        self.current.lock().unwrap().insert(browser, url);
    }
}

impl SchoolBrowser for FakeBrowser {
    fn attach(&self, listener: Weak<dyn SchoolBrowserListener>) -> Result<(), BrowserError> {
        self.listener.lock().unwrap().replace(listener);
        Ok(())
    }

    fn ensure(&self, browser: BrowserId, initial: Url) -> Result<(), BrowserError> {
        self.current.lock().unwrap().insert(browser, initial);
        Ok(())
    }

    fn dispatch(&self, command: BrowserCommand) -> Result<(), BrowserError> {
        if let BrowserAction::Navigate(url) = &command.command {
            self.current
                .lock()
                .unwrap()
                .insert(command.browser, url.clone());
        }
        self.commands.lock().unwrap().push(command);
        Ok(())
    }

    fn current_url(&self, browser: BrowserId) -> Result<Option<Url>, BrowserError> {
        Ok(self.current.lock().unwrap().get(&browser).cloned())
    }

    fn dispose(&self, browser: BrowserId) -> Result<(), BrowserError> {
        self.current.lock().unwrap().remove(&browser);
        Ok(())
    }

    fn shutdown(&self) -> BrowserFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

fn engine() -> (
    Arc<SchoolBrowserEngine>,
    Arc<FakeBrowser>,
    Arc<Events>,
    DataStore,
) {
    engine_with_auth(Arc::new(SchoolAuthService::new(EmptyVault)))
}

fn engine_with_auth(
    auth: Arc<SchoolAuthService>,
) -> (
    Arc<SchoolBrowserEngine>,
    Arc<FakeBrowser>,
    Arc<Events>,
    DataStore,
) {
    let path = std::env::temp_dir().join(format!("isct-browser-{}.db", Uuid::new_v4()));
    let store = DataStore::initialize(path).unwrap().0;
    let browser = Arc::new(FakeBrowser::default());
    let events = Arc::new(Events::default());
    let engine = SchoolBrowserEngine::new(
        store.clone(),
        auth,
        events.clone(),
        browser.clone(),
        SchoolRefresh::new(PullNetwork::Unknown),
    )
    .unwrap();
    (engine, browser, events, store)
}

#[derive(Clone)]
struct BlockingVault {
    value: Arc<Mutex<Option<String>>>,
    block: Arc<AtomicBool>,
    started: mpsc::Sender<()>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl SecretVault for BlockingVault {
    fn store(&self, _account: &str, secret: &str) -> Result<(), SecretError> {
        self.value.lock().unwrap().replace(secret.into());
        Ok(())
    }
    fn load(&self, _account: &str) -> Result<Option<String>, SecretError> {
        if self.block.load(Ordering::Acquire) {
            let _ = self.started.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        Ok(self.value.lock().unwrap().clone())
    }
    fn delete(&self, _account: &str) -> Result<(), SecretError> {
        self.value.lock().unwrap().take();
        Ok(())
    }
}

fn extraction_relay(script: &str, raw: serde_json::Value) -> Url {
    let relay = script
        .split("location.href='")
        .nth(1)
        .and_then(|value| value.split("'+btoa").next())
        .unwrap();
    let encoded_run = script
        .split("_isctExtractionRun=")
        .nth(1)
        .and_then(|value| value.split(';').next())
        .unwrap();
    let run: String = serde_json::from_str(encoded_run).unwrap();
    let mut raw = raw;
    raw.as_object_mut()
        .unwrap()
        .insert("_isctExtractionRun".into(), run.into());
    Url::parse(&format!(
        "{relay}{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&raw).unwrap())
    ))
    .unwrap()
}

fn login_config(script: &str) -> serde_json::Value {
    let raw = script
        .trim_end()
        .rsplit_once("})(")
        .map(|(_, value)| value)
        .and_then(|value| value.strip_suffix(");"))
        .unwrap_or_else(|| panic!("login configuration missing from script: {script}"));
    serde_json::from_str(raw).unwrap()
}

#[tokio::test]
async fn heartbeat_loads_the_vault_only_after_login_and_settles_missing_credentials() {
    let (engine, browser, _, _) = engine();
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let command = browser.first_navigation().await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: command.browser,
            sequence: 1,
        },
        cause: Some(command.action),
        url: Url::parse("https://isct.ex-tic.com/auth").unwrap(),
    });
    assert!(!check.await.unwrap().unwrap());
    assert_eq!(
        engine.session_snapshot().unwrap().states.science,
        SchoolSessionState::NotConfigured
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn stale_duplicate_load_is_ignored_and_same_site_popup_close_returns_to_opener() {
    let (engine, browser, events, _) = engine();
    engine
        .open(
            SchoolSite::Kyomu,
            Some("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx".into()),
        )
        .await
        .unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    let listener = browser.listener();
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx").unwrap(),
    });
    tokio::task::yield_now().await;
    assert!(matches!(
        listener.new_window(
            PageId {
                browser: id,
                sequence: 2,
            },
            Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/picker.aspx").unwrap(),
        ),
        NewWindowDecision::SameView(_)
    ));
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 3,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/picker.aspx").unwrap(),
    });
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/stale.aspx").unwrap(),
    });
    tokio::task::yield_now().await;
    assert!(browser.commands.lock().unwrap().iter().any(|command| {
        matches!(&command.command, BrowserAction::Eval(script) if script.contains("window.close"))
    }));
    let urls: Vec<_> = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::SchoolWebUrl { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect();
    assert!(!urls.iter().any(|url| url.ends_with("stale.aspx")));
    engine.shutdown().await;
}

#[tokio::test]
async fn shutdown_wakes_a_pending_session_without_waiting_for_its_timeout() {
    let (engine, browser, _, _) = engine();
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    browser.first_navigation().await;
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

#[tokio::test(start_paused = true)]
async fn heartbeat_timeout_publishes_failed_releases_the_lane_and_allows_retry() {
    let (engine, browser, _, _) = engine();
    let first = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::TokyoTech).await })
    };
    browser.navigation(0).await;
    tokio::time::advance(std::time::Duration::from_secs(91)).await;
    let error = first.await.unwrap().unwrap_err();
    assert!(matches!(error, SchoolBrowserError::Timeout(_)));
    assert_eq!(
        engine.session_snapshot().unwrap().states.tokyo_tech,
        SchoolSessionState::Failed
    );

    let retry = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::TokyoTech).await })
    };
    let navigation = browser.navigation(1).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://portal.nap.gsic.titech.ac.jp/portal.pl").unwrap(),
    });
    assert!(!retry.await.unwrap().unwrap());
    assert_eq!(
        engine.session_snapshot().unwrap().states.tokyo_tech,
        SchoolSessionState::NotConfigured
    );
    engine.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn manual_login_timeout_emits_error_and_failed_session() {
    let (engine, browser, events, _) = engine();
    let login = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .login(
                    SchoolAuthProfile::Science,
                    SchoolCredentials {
                        username: "student".into(),
                        password: "password".into(),
                        otp_secret: "JBSWY3DPEHPK3PXP".into(),
                        matrix: String::new(),
                    },
                )
                .await
        })
    };
    browser.navigation(0).await;
    tokio::time::advance(std::time::Duration::from_secs(91)).await;
    assert!(matches!(
        login.await.unwrap(),
        Err(SchoolBrowserError::Timeout(_))
    ));
    assert_eq!(
        engine.session_snapshot().unwrap().states.science,
        SchoolSessionState::Failed
    );
    assert!(events.0.lock().unwrap().iter().any(|event| matches!(
        event,
        CoreEvent::SchoolLogin(SchoolLoginEvent { profile: SchoolAuthProfile::Science, state, .. })
            if state == "error"
    )));
    engine.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn heartbeat_timeout_rejects_the_old_login_relay_and_retry_can_connect() {
    let (started_send, _started_receive) = mpsc::channel();
    let (_release_send, release_receive) = mpsc::channel();
    let vault = BlockingVault {
        value: Arc::new(Mutex::new(None)),
        block: Arc::new(AtomicBool::new(false)),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "student".into(),
                password: "password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
    let (engine, browser, events, _) = engine_with_auth(Arc::new(SchoolAuthService::new(vault)));
    let first = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let first_navigation = browser.navigation(0).await;
    let listener = browser.listener();
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: first_navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth?return=discarded").unwrap(),
    });
    let first_config = login_config(&browser.eval(0).await);
    tokio::time::advance(std::time::Duration::from_secs(91)).await;
    assert!(matches!(
        first.await.unwrap(),
        Err(SchoolBrowserError::Timeout(_))
    ));
    let late = serde_json::json!({
        "profile": "science",
        "run": first_config["run"],
        "state": "invalid-credentials",
        "message": "late relay"
    });
    listener.title_changed(
        PageId {
            browser: first_navigation.browser,
            sequence: 1,
        },
        format!(
            "{}{}",
            first_config["authRelay"].as_str().unwrap(),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&late).unwrap())
        ),
    );
    assert!(!events.0.lock().unwrap().iter().any(|event| matches!(
        event,
        CoreEvent::SchoolLogin(SchoolLoginEvent { state, .. }) if state == "invalid-credentials"
    )));

    let retry = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let retry_navigation = browser.navigation(1).await;
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: retry_navigation.browser,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth?return=discarded").unwrap(),
    });
    browser.eval(1).await;
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: retry_navigation.browser,
            sequence: 3,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/").unwrap(),
    });
    assert!(retry.await.unwrap().unwrap());
    assert_eq!(
        engine.session_snapshot().unwrap().states.science,
        SchoolSessionState::Active
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn visible_preemption_requeues_one_heartbeat_and_resumes_after_hide() {
    let (engine, browser, _, _) = engine();
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    browser.navigation(0).await;
    engine
        .user_browsing(Some(SchoolSite::SciencePortal))
        .unwrap();
    tokio::task::yield_now().await;
    assert!(!check.is_finished());
    engine.user_browsing(None).unwrap();
    let resumed = browser.navigation(1).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: resumed.browser,
            sequence: 1,
        },
        // Real native adapters cannot reliably identify the initiating action.
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth").unwrap(),
    });
    assert!(!check.await.unwrap().unwrap());
    assert_eq!(
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
            .count(),
        2
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn late_vault_result_cannot_start_login_after_another_site_preempts_the_heartbeat() {
    let (started_send, started_receive) = mpsc::channel();
    let (release_send, release_receive) = mpsc::channel();
    let value = Arc::new(Mutex::new(None));
    let block = Arc::new(AtomicBool::new(false));
    let vault = BlockingVault {
        value,
        block: block.clone(),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .save(
            SchoolAuthProfile::Science,
            SchoolCredentials {
                username: "student".into(),
                password: "password".into(),
                otp_secret: "JBSWY3DPEHPK3PXP".into(),
                matrix: String::new(),
            },
        )
        .unwrap();
    block.store(true, Ordering::Release);
    let (engine, browser, _, _) = engine_with_auth(Arc::new(SchoolAuthService::new(vault)));
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let navigation = browser.navigation(0).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth").unwrap(),
    });
    tokio::task::spawn_blocking(move || {
        started_receive.recv_timeout(std::time::Duration::from_secs(1))
    })
    .await
    .unwrap()
    .unwrap();
    engine.user_browsing(Some(SchoolSite::Kyomu)).unwrap();
    release_send.send(()).unwrap();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(!browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .any(|command| matches!(command.command, BrowserAction::Eval(_))));
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

#[tokio::test]
async fn tokyo_login_query_is_sanitized_consistently_before_saved_credentials_are_injected() {
    let (started_send, _started_receive) = mpsc::channel();
    let (_release_send, release_receive) = mpsc::channel();
    let vault = BlockingVault {
        value: Arc::new(Mutex::new(None)),
        block: Arc::new(AtomicBool::new(false)),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .save(
            SchoolAuthProfile::TokyoTech,
            SchoolCredentials {
                username: "student".into(),
                password: "password".into(),
                otp_secret: String::new(),
                matrix: "A".repeat(70),
            },
        )
        .unwrap();
    let (engine, browser, _, _) = engine_with_auth(Arc::new(SchoolAuthService::new(vault)));
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::TokyoTech).await })
    };
    let navigation = browser.navigation(0).await;
    let raw_login_url = Url::parse(
        "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?SAMLRequest=request&RelayState=state",
    )
    .unwrap();
    browser.set_current_url(navigation.browser, raw_login_url.clone());
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: raw_login_url,
    });
    let mut injected = false;
    for _ in 0..1_000 {
        if browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|command| matches!(command.command, BrowserAction::Eval(_)))
        {
            injected = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(injected, "Tokyo Tech credentials were not injected");
    let navigations_before = browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
        .count();
    engine.open(SchoolSite::TokyoPortal, None).await.unwrap();
    let navigations_after = browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
        .count();
    assert_eq!(navigations_after, navigations_before);
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

#[tokio::test]
async fn two_profile_requests_coalesce_per_profile_without_overwriting_the_active_heartbeat() {
    let (engine, browser, _, _) = engine();
    assert!(engine.request_session(SchoolAuthProfile::Science).unwrap());
    assert!(engine
        .request_session(SchoolAuthProfile::TokyoTech)
        .unwrap());
    assert!(!engine.request_session(SchoolAuthProfile::Science).unwrap());
    for index in 0..2 {
        let navigation = browser.navigation(index).await;
        let auth_url = match navigation.browser.site {
            SchoolSite::SciencePortal => "https://isct.ex-tic.com/auth",
            SchoolSite::TokyoPortal => "https://portal.nap.gsic.titech.ac.jp/portal.pl",
            site => panic!("unexpected session site {site:?}"),
        };
        browser.listener().page_loaded(PageLoaded {
            page: PageId {
                browser: navigation.browser,
                sequence: 1,
            },
            cause: None,
            url: Url::parse(auth_url).unwrap(),
        });
    }
    for _ in 0..1_000 {
        let snapshot = engine.session_snapshot().unwrap();
        if snapshot.states.science == SchoolSessionState::NotConfigured
            && snapshot.states.tokyo_tech == SchoolSessionState::NotConfigured
        {
            engine.shutdown().await;
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("both session requests did not settle");
}

#[tokio::test]
async fn coordinator_pull_accepts_native_cause_none_and_commits_lms_payload() {
    let (engine, browser, _, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = {
        let engine = engine.clone();
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    PullTarget::School {
                        site: SchoolSite::ScienceLms,
                        course_id: None,
                    },
                    PullOptions::manual(),
                    move |context| {
                        let engine = engine.clone();
                        async move {
                            engine
                                .pull(SchoolSite::ScienceLms, None, context, PullPriority::Manual)
                                .await
                                .map(|outcomes| serde_json::to_value(outcomes).unwrap())
                                .map_err(|error| {
                                    crate::pull_coordinator::PullError::Execution(error.to_string())
                                })
                        }
                    },
                )
                .await
        })
    };
    let navigation = browser.navigation(0).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    });
    let relay = extraction_relay(
        &browser.extraction_script().await,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
            "language": "en",
            "coursesComplete": true,
            "courses": [{
                "id": 123456789,
                "fullname": "Trace course",
                "shortname": "TRACE",
                "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=123456789"
            }]
        }),
    );
    assert_eq!(
        browser.listener().decide_navigation(
            PageId {
                browser: navigation.browser,
                sequence: 1,
            },
            relay,
            true,
        ),
        NavigationDecision::Cancel
    );
    request.await.unwrap().unwrap();
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM lms_courses WHERE id='lms-course:123456789'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    coordinator.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn full_lms_pull_rejects_the_preempted_relay_and_resumes_the_same_caller() {
    let (engine, browser, _, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = {
        let engine = engine.clone();
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    PullTarget::School {
                        site: SchoolSite::ScienceLms,
                        course_id: None,
                    },
                    PullOptions::manual(),
                    move |context| {
                        let engine = engine.clone();
                        async move {
                            engine
                                .pull(SchoolSite::ScienceLms, None, context, PullPriority::Manual)
                                .await
                                .map(|outcomes| serde_json::to_value(outcomes).unwrap())
                                .map_err(|error| {
                                    crate::pull_coordinator::PullError::Execution(error.to_string())
                                })
                        }
                    },
                )
                .await
        })
    };
    let first_navigation = browser.navigation(0).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: first_navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    });
    let stale_relay = extraction_relay(
        &browser.extraction_script_at(0).await,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
            "language": "en",
            "coursesComplete": true,
            "courses": [{
                "id": 111111111,
                "fullname": "Preempted course",
                "shortname": "STALE",
                "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=111111111"
            }]
        }),
    );
    let admitted = coordinator.snapshots().await[0].clone();

    engine.user_browsing(Some(SchoolSite::Kyomu)).unwrap();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(!request.is_finished());
    engine.user_browsing(None).unwrap();

    let resumed_navigation = browser.navigation(1).await;
    assert_eq!(resumed_navigation.browser, first_navigation.browser);
    assert_eq!(
        browser.listener().decide_navigation(
            PageId {
                browser: first_navigation.browser,
                sequence: 1,
            },
            stale_relay,
            true,
        ),
        NavigationDecision::Cancel
    );
    assert!(!request.is_finished());

    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: resumed_navigation.browser,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    });
    let fresh_relay = extraction_relay(
        &browser.extraction_script_at(1).await,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
            "language": "en",
            "coursesComplete": true,
            "courses": [{
                "id": 222222222,
                "fullname": "Resumed course",
                "shortname": "FRESH",
                "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=222222222"
            }]
        }),
    );
    assert_eq!(
        browser.listener().decide_navigation(
            PageId {
                browser: resumed_navigation.browser,
                sequence: 2,
            },
            fresh_relay,
            true,
        ),
        NavigationDecision::Cancel
    );
    let value = request.await.unwrap().unwrap();
    let outcomes: Vec<IngestOutcome> = serde_json::from_value(value).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].state, "ok");

    let completed = coordinator.snapshots().await[0].clone();
    assert_eq!(completed.id, admitted.id);
    assert_eq!(completed.attempt, 1);
    assert_eq!(completed.state, PullState::Succeeded);
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let stale: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM lms_courses WHERE id='lms-course:111111111'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let fresh: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM lms_courses WHERE id='lms-course:222222222'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!((stale, fresh), (0, 1));
    coordinator.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn kyomu_partial_commit_marks_coordinator_failed_when_the_next_page_fails() {
    let (engine, browser, _, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = {
        let engine = engine.clone();
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    PullTarget::School {
                        site: SchoolSite::Kyomu,
                        course_id: None,
                    },
                    PullOptions::manual(),
                    move |context| {
                        let engine = engine.clone();
                        async move {
                            let outcomes = engine
                                .pull(
                                    SchoolSite::Kyomu,
                                    None,
                                    context.clone(),
                                    PullPriority::Manual,
                                )
                                .await
                                .map_err(|error| {
                                    crate::pull_coordinator::PullError::Execution(error.to_string())
                                })?;
                            if let Some(problem) = outcomes.iter().find(|row| row.state != "ok") {
                                context.outcome_error(problem.message.clone()).await;
                            }
                            serde_json::to_value(outcomes).map_err(|error| {
                                crate::pull_coordinator::PullError::Execution(error.to_string())
                            })
                        }
                    },
                )
                .await
        })
    };

    let landing = browser.navigation(0).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: landing.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/").unwrap(),
    });

    let gpa = browser.navigation(1).await;
    let gpa_url = Url::parse(
        "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
    )
    .unwrap();
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: gpa.browser,
            sequence: 2,
        },
        cause: None,
        url: gpa_url.clone(),
    });
    let relay = extraction_relay(
        &browser.extraction_script().await,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": gpa_url.as_str(),
            "kind": "gpa",
            "data": {
                "overall": [[[], ["GPA", "GPT", "修得済単位", "合格率"], ["3.20", "100", "10", "90"]]],
                "terms": []
            }
        }),
    );
    assert_eq!(
        browser.listener().decide_navigation(
            PageId {
                browser: gpa.browser,
                sequence: 2,
            },
            relay,
            true,
        ),
        NavigationDecision::Cancel
    );

    let registrations = browser.navigation(2).await;
    browser.listener().load_failed(LoadFailed {
        page: PageId {
            browser: registrations.browser,
            sequence: 3,
        },
        cause: Some(registrations.action),
        url: match registrations.command {
            BrowserAction::Navigate(url) => Some(url),
            _ => None,
        },
        message: "registrations unavailable".into(),
    });

    let value = request.await.unwrap().unwrap();
    let outcomes: Vec<IngestOutcome> = serde_json::from_value(value).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        (outcomes[0].kind.as_str(), outcomes[0].state.as_str()),
        ("gpa", "ok")
    );
    assert_eq!(
        (outcomes[1].kind.as_str(), outcomes[1].state.as_str()),
        ("registrations", "error")
    );
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Failed);
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM gpa_summary WHERE id='kyomu-gpa:all'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    coordinator.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn cancellation_before_applying_keeps_the_lms_database_unchanged() {
    let (engine, browser, _, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = {
        let engine = engine.clone();
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    PullTarget::School {
                        site: SchoolSite::ScienceLms,
                        course_id: None,
                    },
                    PullOptions::manual(),
                    move |context| {
                        let engine = engine.clone();
                        async move {
                            engine
                                .pull(SchoolSite::ScienceLms, None, context, PullPriority::Manual)
                                .await
                                .map(|outcomes| serde_json::to_value(outcomes).unwrap())
                                .map_err(|error| {
                                    crate::pull_coordinator::PullError::Execution(error.to_string())
                                })
                        }
                    },
                )
                .await
        })
    };
    let navigation = browser.navigation(0).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    });
    let relay = extraction_relay(
        &browser.extraction_script().await,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
            "language": "en",
            "coursesComplete": true,
            "courses": [{"id": 987654321, "fullname": "Cancelled"}]
        }),
    );
    let id = coordinator.snapshots().await[0].id.clone();
    assert!(coordinator.cancel(&id).await);
    browser.listener().decide_navigation(
        PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        relay,
        true,
    );
    assert!(request.await.unwrap().is_err());
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM lms_courses WHERE id='lms-course:987654321'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    coordinator.shutdown().await;
    engine.shutdown().await;
}
