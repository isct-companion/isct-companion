use super::*;
use crate::{
    data::DataStore,
    events::CoreEvent,
    lifecycle::SchoolSite,
    ports::{EventError, EventSink, SecretError, SecretVault},
    school_auth::{SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    sync_engine::SyncHints,
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
    /// Report the next dispatch as sent to a view that no longer exists.
    unavailable: AtomicBool,
    maintenance: Mutex<Option<String>>,
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
        for attempt in 0..6_000 {
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
            settle(attempt).await;
        }
        panic!("navigation {index} was not dispatched");
    }

    async fn eval(&self, index: usize) -> String {
        for attempt in 0..6_000 {
            let scripts: Vec<_> = self
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter_map(|command| match &command.command {
                    BrowserAction::Eval(script)
                        if !script.contains("isct-companion-availability:")
                            && !script.starts_with("document.title=") =>
                    {
                        Some(script.clone())
                    }
                    _ => None,
                })
                .collect();
            if let Some(script) = scripts.get(index) {
                return script.clone();
            }
            settle(attempt).await;
        }
        panic!("eval {index} was not dispatched");
    }

    fn set_current_url(&self, browser: BrowserId, url: Url) {
        self.current.lock().unwrap().insert(browser, url);
    }
}

/// Yields first, then sleeps briefly so blocking work (vault reads, SQLite
/// commits) can finish on a loaded machine.
async fn settle(attempt: usize) {
    if attempt < 1_000 {
        tokio::task::yield_now().await;
    } else {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
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
        if self.unavailable.swap(false, Ordering::AcqRel) {
            return Err(BrowserError::Unavailable(
                "School view is unavailable.".into(),
            ));
        }
        if let BrowserAction::Navigate(url) = &command.command {
            self.current
                .lock()
                .unwrap()
                .insert(command.browser, url.clone());
        }
        let availability = match &command.command {
            BrowserAction::Eval(script) if script.contains("isct-companion-availability:") => {
                let prefix = script
                    .split("document.title=")
                    .nth(1)
                    .unwrap()
                    .split('+')
                    .next()
                    .unwrap();
                let prefix: String = serde_json::from_str(prefix).unwrap();
                let sequence: u64 = script
                    .split("sequence:")
                    .nth(1)
                    .unwrap()
                    .split(',')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                Some((prefix, sequence))
            }
            _ => None,
        };
        let browser = command.browser;
        self.commands.lock().unwrap().push(command);
        if let Some((prefix, sequence)) = availability {
            let payload = serde_json::json!({"sequence":sequence,"message":self.maintenance.lock().unwrap().clone()});
            self.listener().title_changed(
                PageId { browser, sequence },
                format!("{prefix}{}", URL_SAFE_NO_PAD.encode(payload.to_string())),
            );
        }
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

#[tokio::test]
async fn otp_setup_uses_visible_manual_login_then_continues_to_settings_without_saved_otp() {
    for auth in [
        Arc::new(SchoolAuthService::new(EmptyVault)),
        Arc::new(SchoolAuthService::new(saved_science_vault())),
    ] {
        let (engine, browser, _, _) = engine_with_auth(auth);
        let operation = engine.begin_school_otp_setup().unwrap();
        let settings = "https://isct.ex-tic.com/idm/user/mfa_setting/";
        engine
            .navigate(SchoolSite::SciencePortal, settings)
            .unwrap();
        let opened = browser.navigation(0).await;
        let listener = browser.listener();
        listener.page_loaded(PageLoaded {
            page: PageId {
                browser: opened.browser,
                sequence: 1,
            },
            cause: None,
            url: Url::parse("https://isct.ex-tic.com/auth/session/second_factor").unwrap(),
        });
        tokio::task::yield_now().await;
        assert!(!browser.commands.lock().unwrap().iter().any(|command| {
            matches!(&command.command, BrowserAction::Eval(script) if script.contains("isct-companion-login:"))
        }));
        listener.page_loaded(PageLoaded {
            page: PageId {
                browser: opened.browser,
                sequence: 2,
            },
            cause: None,
            url: Url::parse("https://isct.ex-tic.com/idm/user/portal/").unwrap(),
        });
        let continued = browser.navigation(1).await;
        assert!(
            matches!(&continued.command, BrowserAction::Navigate(url) if url.as_str() == settings)
        );
        // Leaving cancels this enrollment, so a later portal load cannot
        // redirect a different browsing session.
        engine.user_browsing(None).unwrap();
        assert!(matches!(
            engine.school_otp_setup(operation).await,
            Err(crate::school_auth::OtpSetupError::Cancelled)
        ));
        listener.page_loaded(PageLoaded {
            page: PageId {
                browser: opened.browser,
                sequence: 3,
            },
            cause: None,
            url: Url::parse("https://isct.ex-tic.com/idm/user/portal/").unwrap(),
        });
        assert_eq!(
            browser
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter(|command| matches!(&command.command, BrowserAction::Navigate(_)))
                .count(),
            2
        );
        engine.shutdown().await;
    }
}

#[tokio::test(start_paused = true)]
async fn otp_setup_waits_have_correlated_cancellation_drop_timeout_and_shutdown() {
    use crate::school_auth::OtpSetupError;
    let (engine, _, _, _) = engine();
    let first = engine.begin_school_otp_setup().unwrap();
    let second = engine.begin_school_otp_setup().unwrap();
    engine.cancel_school_otp_setup(first);
    assert!(matches!(
        engine.school_otp_setup(first).await,
        Err(OtpSetupError::Cancelled)
    ));
    let mut wait = Box::pin(engine.school_otp_setup(second));
    assert!(futures_util::poll!(&mut wait).is_pending());
    drop(wait);
    assert!(matches!(
        engine.school_otp_setup(second).await,
        Err(OtpSetupError::Cancelled)
    ));
    let third = engine.begin_school_otp_setup().unwrap();
    assert!(matches!(
        engine.school_otp_setup(third).await,
        Err(OtpSetupError::TimedOut)
    ));
    let fourth = engine.begin_school_otp_setup().unwrap();
    let mut wait = Box::pin(engine.school_otp_setup(fourth));
    assert!(futures_util::poll!(&mut wait).is_pending());
    engine.shutdown().await;
    assert!(matches!(wait.await, Err(OtpSetupError::Cancelled)));
}

#[cfg(feature = "desktop-workspaces")]
#[tokio::test]
async fn assignment_upload_targets_the_edit_page_once_and_rejects_other_destinations() {
    use crate::workspaces::SubmissionFile;
    let (engine, browser, _, _) = engine();
    let files = || {
        vec![SubmissionFile {
            name: "report.txt".into(),
            data: "bXkgcmVwb3J0".into(),
        }]
    };
    let messages = || SubmissionMessages {
        waiting: "waiting".into(),
        queued: "queued".into(),
        failed: "failed".into(),
    };
    for invalid in [
        "https://example.com/mod/assign/view.php?id=100",
        "http://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100",
        "https://lms.s.isct.ac.jp/2025/course/view.php?id=100",
        "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&id=200",
    ] {
        assert!(engine
            .prepare_submission(invalid, files(), messages())
            .is_err());
    }
    let target = engine
        .prepare_submission(
            "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100",
            files(),
            messages(),
        )
        .unwrap();
    assert_eq!(
        target,
        "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission"
    );
    engine
        .open(SchoolSite::ScienceLms, Some(target.clone()))
        .await
        .unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    // Follow the URL actually sent to the native browser, not a synthetic target.
    assert_eq!(browser.current.lock().unwrap()[&id].as_str(), target);
    engine.navigate(SchoolSite::ScienceLms, &target).unwrap();
    assert_eq!(browser.current.lock().unwrap()[&id].as_str(), target);
    let count_uploads = || {
        browser.commands.lock().unwrap().iter().filter(|command| matches!(&command.command, BrowserAction::Eval(script) if script.contains("__isctAssignmentUpload"))).count()
    };
    let listener = browser.listener();
    for (sequence, url) in [
        (
            1,
            "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=200&action=editsubmission",
        ),
        (2, target.as_str()),
        (3, target.as_str()),
    ] {
        listener.page_loaded(PageLoaded {
            page: PageId {
                browser: id,
                sequence,
            },
            cause: None,
            url: Url::parse(url).unwrap(),
        });
        tokio::task::yield_now().await;
        assert_eq!(count_uploads(), if sequence == 1 { 0 } else { 1 });
    }
    engine.shutdown().await;
}

fn engine_with_auth(
    auth: Arc<SchoolAuthService>,
) -> (
    Arc<SchoolBrowserEngine>,
    Arc<FakeBrowser>,
    Arc<Events>,
    DataStore,
) {
    let path = std::env::temp_dir().join(format!("isct-browser-{}", Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    let browser = Arc::new(FakeBrowser::default());
    let events = Arc::new(Events::default());
    let engine =
        SchoolBrowserEngine::new(auth, events.clone(), browser.clone(), SyncHints::detached())
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

fn login_config(script: &str) -> serde_json::Value {
    let raw = script
        .trim_end()
        .rsplit_once("})(")
        .map(|(_, value)| value)
        .and_then(|value| value.strip_suffix(");"))
        .unwrap_or_else(|| panic!("login configuration missing from script: {script}"));
    serde_json::from_str(raw).unwrap()
}

// Protect real engine routing: rich editors need inherited-origin frames;
// interactive external pages must not inherit school automation authority.
#[tokio::test]
async fn interactive_browsing_supports_editor_frames_and_external_history_without_school_scripts() {
    let (engine, browser, events, _) = engine();
    let site = SchoolSite::ScienceLms;
    let official =
        "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission#editor";
    engine.open(site, Some(official.into())).await.unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    let listener = browser.listener();
    let page = PageId {
        browser: id,
        sequence: 1,
    };
    listener.page_loaded(PageLoaded {
        page,
        cause: None,
        url: Url::parse(official).unwrap(),
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if events
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, CoreEvent::SchoolWebUrl {url, ..} if url == official))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    engine
        .set_customization(
            site,
            SchoolPageCustomization {
                css: "body{color:red}".into(),
                javascript: "schoolCustomization()".into(),
            },
        )
        .unwrap();
    for main_frame in [false, true] {
        for raw in [
            "about:blank",
            "about:srcdoc",
            "https://docs.google.com/forms/d/test/viewform?entry.1=a%20b#section",
            "http://example.com/search?q=course",
        ] {
            assert_eq!(
                listener.decide_navigation(page, Url::parse(raw).unwrap(), main_frame),
                NavigationDecision::Allow,
                "{raw}"
            );
        }
    }
    for raw in [
        "javascript:alert(1)",
        "file:///tmp/private",
        "data:text/html,test",
        "https://user:secret@example.com/",
    ] {
        assert_eq!(
            listener.decide_navigation(page, Url::parse(raw).unwrap(), true),
            NavigationDecision::Cancel,
            "{raw}"
        );
    }
    let external = "https://www.google.com/search?q=linear%20algebra&hl=ja#results";
    let resolved = browsing_target(external, site).unwrap();
    assert_eq!(resolved.site, site);
    assert_eq!(resolved.url, external);
    assert!(sanitize_school_url(external).is_none());
    assert!(
        matches!(listener.new_window(page, Url::parse(external).unwrap()), NewWindowDecision::SameView(url) if url.as_str() == external)
    );
    let scripts_before = browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Eval(_)))
        .count();
    engine.navigate(site, external).unwrap();
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 2,
        },
        cause: None,
        url: Url::parse(external).unwrap(),
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if events
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, CoreEvent::SchoolWebUrl {url, ..} if url == external))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let scripts_after = browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Eval(_)))
        .count();
    assert_eq!(
        scripts_after, scripts_before,
        "external page received a school script"
    );
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|event| matches!(event, CoreEvent::SchoolWebUrl {url, ..} if url == external)));
    assert!(engine.back(site).unwrap());
    assert_eq!(browser.current.lock().unwrap()[&id].as_str(), official);
    assert!(engine.forward(site).unwrap());
    assert_eq!(browser.current.lock().unwrap()[&id].as_str(), external);
    assert_eq!(engine.open(site, None).await.unwrap().url, external);
    engine.user_browsing(None).unwrap();
    assert_eq!(
        listener.decide_navigation(page, Url::parse(external).unwrap(), true),
        NavigationDecision::Cancel
    );
    assert!(matches!(
        listener.new_window(page, Url::parse(external).unwrap()),
        NewWindowDecision::OpenExternal(_)
    ));
    engine.shutdown().await;
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

// Protect saved Tokyo Tech sign-in on GetAccess pages whose query the
// sanitizer drops: the hidden session check records sanitized URLs, a visible
// page records the full one, and both must still inject credentials. A visible
// 教務Web once stopped on the 情報基盤利用承諾 agreement page this way.
#[tokio::test]
async fn tokyo_login_query_is_sanitized_consistently_before_saved_credentials_are_injected() {
    tokyo_saved_credentials_reach(
        false,
        "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?SAMLRequest=request&RelayState=state",
    )
    .await;
    tokyo_saved_credentials_reach(
        true,
        "https://portal.nap.gsic.titech.ac.jp/portal.pl?GASF=CERTIFICATE,IG.GRID,IG.TOKENRO,IG.OTP&GAREASONCODE=-1&GARESOURCEID=kyomuweb2023&GAURI=https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx&Reason=-1&APPID=kyomuweb2023&URI=https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
    )
    .await;
}

async fn tokyo_saved_credentials_reach(visible: bool, login_url: &str) {
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
    let check = (!visible).then(|| {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::TokyoTech).await })
    });
    // A visible view is created at its start URL; the session check navigates.
    let id = if visible {
        engine.open(SchoolSite::Kyomu, None).await.unwrap();
        *browser.current.lock().unwrap().keys().next().unwrap()
    } else {
        browser.navigation(0).await.browser
    };
    let raw_login_url = Url::parse(login_url).unwrap();
    browser.set_current_url(id, raw_login_url.clone());
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 1,
        },
        cause: None,
        url: raw_login_url,
    });
    let mut injected = None;
    for attempt in 0..2_000 {
        injected =
            browser
                .commands
                .lock()
                .unwrap()
                .iter()
                .find_map(|command| match &command.command {
                    BrowserAction::Eval(script) if command.browser == id => script
                        .contains("isct-companion-login")
                        .then(|| script.clone()),
                    _ => None,
                });
        if injected.is_some() {
            break;
        }
        settle(attempt).await;
    }
    let script = injected
        .unwrap_or_else(|| panic!("Tokyo Tech credentials were not injected (visible: {visible})"));
    assert_eq!(login_config(&script)["profile"], "tokyo-tech");
    if !visible {
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
    }
    engine.shutdown().await;
    if let Some(check) = check {
        assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
    }
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

fn saved_science_vault() -> BlockingVault {
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
    vault
}

#[tokio::test]
async fn a_closed_view_fails_its_session_check_and_is_recreated_next_time() {
    let (engine, browser, _, _) = engine();
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let first = browser.navigation(0).await;
    browser.listener().closed(first.browser);
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), check)
        .await
        .expect("a closed view must not wait for the session timeout")
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error,
        SchoolBrowserError::Platform("The school view was closed.".into())
    );
    assert_eq!(
        engine.session_snapshot().unwrap().states.science,
        SchoolSessionState::Failed
    );
    let retry = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let second = browser.navigation(1).await;
    assert_eq!(second.browser.site, first.browser.site);
    assert!(second.browser.generation > first.browser.generation);
    engine.shutdown().await;
    assert_eq!(retry.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

#[tokio::test]
async fn a_view_reported_unavailable_is_replaced_instead_of_kept_as_a_zombie() {
    let (engine, browser, _, _) = engine();
    let target = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
    browser.unavailable.store(true, Ordering::Release);
    assert!(engine.navigate(SchoolSite::Kyomu, target).is_err());
    assert!(engine.navigate(SchoolSite::Kyomu, target).unwrap());
    let commands = browser.commands.lock().unwrap().clone();
    assert_eq!(commands.len(), 1);
    let ensured: Vec<_> = browser.current.lock().unwrap().keys().copied().collect();
    assert_eq!(
        ensured.len(),
        2,
        "the second navigation must create a new view"
    );
    assert!(ensured.contains(&commands[0].browser));
    engine.shutdown().await;
}

fn manual_credentials() -> SchoolCredentials {
    SchoolCredentials {
        username: "typed-in-settings".into(),
        password: "password".into(),
        otp_secret: "JBSWY3DPEHPK3PXP".into(),
        matrix: String::new(),
    }
}

#[tokio::test]
async fn sign_in_ended_by_a_credential_change_leaves_no_stale_run() {
    let (engine, browser, _, _) =
        engine_with_auth(Arc::new(SchoolAuthService::new(saved_science_vault())));
    let login = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .login(SchoolAuthProfile::Science, manual_credentials())
                .await
        })
    };
    browser.navigation(0).await;
    engine
        .credentials_changed(SchoolAuthProfile::Science, false)
        .unwrap();
    assert_eq!(
        login.await.unwrap(),
        Err(SchoolBrowserError::Auth("School sign-in failed.".into()))
    );
    // The next sign-in page must be answered by a fresh automatic sign-in with
    // the saved credentials, not by the abandoned manual run.
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    let navigation = browser.navigation(1).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth").unwrap(),
    });
    assert_eq!(login_config(&browser.eval(0).await)["username"], "student");
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

#[tokio::test]
async fn manual_sign_in_paused_by_a_visible_page_resumes_after_it_hides() {
    let (engine, browser, events, _) = engine();
    let login = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .login(SchoolAuthProfile::Science, manual_credentials())
                .await
        })
    };
    browser.navigation(0).await;
    engine.user_browsing(Some(SchoolSite::Kyomu)).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!login.is_finished());
    engine.user_browsing(None).unwrap();
    let resumed = browser.navigation(1).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: resumed.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/idm/user/portal/").unwrap(),
    });
    assert_eq!(login.await.unwrap().unwrap().phase, "sessionRestored");
    assert!(!events.0.lock().unwrap().iter().any(|event| matches!(
        event,
        CoreEvent::SchoolLogin(SchoolLoginEvent { state, .. }) if state == "error"
    )));
    engine.shutdown().await;
}

#[tokio::test]
async fn a_panic_while_state_was_locked_does_not_stop_the_engine() {
    let (engine, browser, _, _) = engine();
    let listener = browser.listener();
    // Poison the engine state from a panicking native callback thread.
    let poisoned = std::thread::spawn({
        let engine = engine.clone();
        move || engine.poison_state_for_test()
    })
    .join();
    assert!(poisoned.is_err());
    assert!(engine.session_snapshot().is_ok());
    assert!(engine
        .navigate(
            SchoolSite::Kyomu,
            "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx"
        )
        .unwrap());
    let navigation = browser.navigation(0).await;
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx").unwrap(),
    });
    engine.shutdown().await;
}
