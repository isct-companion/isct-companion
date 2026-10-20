use super::*;
use crate::{
    data::DataStore,
    events::CoreEvent,
    lifecycle::SchoolSite,
    ports::{EventError, EventSink, SecretError, SecretVault},
    pull_coordinator::{PullCoordinator, PullNetwork, PullOptions, PullState, PullTarget},
    school_auth::{SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    school_refresh::SchoolRefresh,
    school_service::{IngestOutcome, IngestState, SchoolService},
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

    async fn extraction_script(&self) -> String {
        self.extraction_script_at(0).await
    }

    async fn extraction_script_at(&self, index: usize) -> String {
        for attempt in 0..6_000 {
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
            settle(attempt).await;
        }
        panic!("extraction {index} was not dispatched");
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
    let mut parts = extraction_parts(script, raw);
    assert_eq!(parts.len(), 1);
    parts.remove(0)
}

/// The navigations the page-side relay sends for `raw`, part by part.
fn extraction_parts(script: &str, raw: serde_json::Value) -> Vec<Url> {
    let quoted = |key: &str| {
        script
            .split(key)
            .nth(1)
            .and_then(|value| value.split(';').next())
            .unwrap()
    };
    let relay: String = serde_json::from_str(quoted("const relay = ")).unwrap();
    let run: String = serde_json::from_str(quoted("const run = ")).unwrap();
    let part: usize = quoted("const part = ").parse().unwrap();
    let mut raw = raw;
    raw.as_object_mut()
        .unwrap()
        .insert("_isctExtractionRun".into(), run.clone().into());
    let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&raw).unwrap());
    let chunks: Vec<_> = encoded.as_bytes().chunks(part).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            Url::parse(&format!(
                "{relay}{run}/{}/{index}/{}",
                chunks.len(),
                std::str::from_utf8(chunk).unwrap()
            ))
            .unwrap()
        })
        .collect()
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
                                .pull(SchoolSite::ScienceLms, None, None, context)
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
    assert_eq!(outcomes[0].state, IngestState::Ok);

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
    let service = SchoolService::new(
        coordinator.clone(),
        Arc::new(SchoolAuthService::new(EmptyVault)),
        SchoolRefresh::new(PullNetwork::Unknown),
    );
    let request = {
        let engine = engine.clone();
        tokio::spawn(async move {
            service
                .pull(
                    engine,
                    SchoolSite::Kyomu,
                    None,
                    PullOptions::manual(),
                    false,
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
                "complete": true,
                "shape": "gpa-summary",
                "empty": false,
                "categories": [],
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

    let outcomes = request.await.unwrap().unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(
        (outcomes[0].kind.as_str(), outcomes[0].state),
        ("gpa", IngestState::Ok)
    );
    assert_eq!(
        (outcomes[1].kind.as_str(), outcomes[1].state),
        ("registrations", IngestState::Error)
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
                                .pull(SchoolSite::ScienceLms, None, None, context)
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

#[tokio::test]
async fn an_lms_result_over_chromiums_url_cap_arrives_in_requested_parts() {
    let (engine, browser, _, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::manual(),
    );
    let navigation = browser.navigation(0).await;
    let page = PageId {
        browser: navigation.browser,
        sequence: 1,
    };
    browser.listener().page_loaded(PageLoaded {
        page,
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    });
    // Over 2 MiB encoded: WebView2 would drop it as one navigation.
    let parts = extraction_parts(
        &browser.extraction_script().await,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
            "language": "en",
            "coursesComplete": true,
            "courses": [{
                "id": 444444444,
                "fullname": "Large course",
                "shortname": "LARGE",
                "summary": "x".repeat(2 * 1024 * 1024),
                "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=444444444"
            }]
        }),
    );
    assert!(parts.len() >= 3);
    assert!(parts.iter().all(|url| url.as_str().len() < 2 * 1024 * 1024));
    let requests = || {
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter_map(|command| match &command.command {
                BrowserAction::Eval(script) if script.contains("__isctRelayPart(") => {
                    Some(script.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // A part sent before Core asked for it is refused and asks for nothing.
    browser
        .listener()
        .decide_navigation(page, parts[1].clone(), true);
    assert!(requests().is_empty());
    for (index, part) in parts.iter().enumerate() {
        assert_eq!(
            browser
                .listener()
                .decide_navigation(page, part.clone(), true),
            NavigationDecision::Cancel
        );
        let asked = requests();
        if index + 1 < parts.len() {
            assert_eq!(asked.len(), index + 1);
            assert!(asked[index].ends_with(&format!(",{});", index + 1)));
        } else {
            assert_eq!(asked.len(), index);
        }
    }
    let outcomes: Vec<IngestOutcome> =
        serde_json::from_value(request.await.unwrap().unwrap()).unwrap();
    assert_eq!(outcomes[0].state, IngestState::Ok);
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM lms_courses WHERE id='lms-course:444444444'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    coordinator.shutdown().await;
    engine.shutdown().await;
}

fn spawn_pull(
    engine: &Arc<SchoolBrowserEngine>,
    coordinator: &PullCoordinator,
    site: SchoolSite,
    options: PullOptions,
) -> tokio::task::JoinHandle<Result<serde_json::Value, crate::pull_coordinator::PullError>> {
    let engine = engine.clone();
    let coordinator = coordinator.clone();
    tokio::spawn(async move {
        coordinator
            .run(
                PullTarget::School {
                    site,
                    course_id: None,
                },
                options,
                move |context| {
                    let engine = engine.clone();
                    async move {
                        engine
                            .pull(site, None, None, context)
                            .await
                            .map(|outcomes| serde_json::to_value(outcomes).unwrap())
                            .map_err(|error| match error {
                                SchoolBrowserError::Cancelled => {
                                    crate::pull_coordinator::PullError::Cancelled
                                }
                                error => {
                                    crate::pull_coordinator::PullError::Execution(error.to_string())
                                }
                            })
                    }
                },
            )
            .await
    })
}

fn kyomu_gpa_relay(script: &str) -> Url {
    extraction_relay(
        script,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
            "kind": "gpa",
            "data": {
                "complete": true,
                "shape": "gpa-summary",
                "empty": false,
                "categories": [],
                "overall": [[[], ["GPA", "GPT", "修得済単位", "合格率"], ["3.20", "100", "10", "90"]]],
                "terms": []
            }
        }),
    )
}

#[tokio::test]
async fn kyomu_pull_can_be_cancelled_between_committed_pages() {
    let (engine, browser, _, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::Kyomu,
        PullOptions::manual(),
    );
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
    let BrowserAction::Navigate(gpa_url) = gpa.command.clone() else {
        panic!("GPA page was not navigated");
    };
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: gpa.browser,
            sequence: 2,
        },
        cause: None,
        url: gpa_url,
    });
    let relay = kyomu_gpa_relay(&browser.extraction_script().await);
    browser.listener().decide_navigation(
        PageId {
            browser: gpa.browser,
            sequence: 2,
        },
        relay,
        true,
    );
    // The second page starts only after the first commit left the write gate.
    browser.navigation(2).await;
    let id = coordinator.snapshots().await[0].id.clone();
    assert!(coordinator.cancel(&id).await);
    assert_eq!(
        request.await.unwrap(),
        Err(crate::pull_coordinator::PullError::Cancelled)
    );
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Cancelled);
    let navigations = browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
        .count();
    assert_eq!(navigations, 3);
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let committed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM gpa_summary WHERE id='kyomu-gpa:all'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(committed, 1);
    coordinator.shutdown().await;
    engine.shutdown().await;
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

async fn wait_for_snapshot(
    coordinator: &PullCoordinator,
    accept: impl Fn(&crate::pull_coordinator::PullSnapshot) -> bool,
) {
    for _ in 0..10_000 {
        if coordinator.snapshots().await.iter().any(&accept) {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("pull snapshot did not reach the expected state");
}

#[tokio::test]
async fn running_pull_signs_in_first_when_a_join_asks_for_login() {
    let (engine, browser, _, _) =
        engine_with_auth(Arc::new(SchoolAuthService::new(saved_science_vault())));
    let coordinator = PullCoordinator::default();
    engine.user_browsing(Some(SchoolSite::Kyomu)).unwrap();
    let automatic = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::automatic(),
    );
    wait_for_snapshot(&coordinator, |row| row.state == PullState::Running).await;
    let forced = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions {
            login_first: true,
            ..PullOptions::manual()
        },
    );
    wait_for_snapshot(&coordinator, |row| row.waiter_count == 2).await;
    engine.user_browsing(None).unwrap();

    let login = browser.navigation(0).await;
    assert_eq!(login.browser.site, SchoolSite::SciencePortal);
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: login.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth").unwrap(),
    });
    assert!(login_config(&browser.eval(0).await)["username"] == "student");
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: login.browser,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/idm/user/portal/").unwrap(),
    });
    let pull = browser.navigation(1).await;
    assert_eq!(pull.browser.site, SchoolSite::ScienceLms);
    assert_eq!(
        engine.session_snapshot().unwrap().states.science,
        SchoolSessionState::Active
    );
    coordinator.shutdown().await;
    assert!(automatic.await.unwrap().is_err());
    assert!(forced.await.unwrap().is_err());
    engine.shutdown().await;
}

#[tokio::test]
async fn manual_join_while_waiting_for_the_lane_registers_a_manual_waiter() {
    let (engine, browser, _, _) = engine();
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    browser.navigation(0).await;
    let coordinator = PullCoordinator::default();
    let automatic = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::automatic(),
    );
    wait_for_snapshot(&coordinator, |row| row.state == PullState::Running).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(engine.manual_waiters(), 0);
    let manual = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::manual(),
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while engine.manual_waiters() == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    coordinator.shutdown().await;
    assert!(automatic.await.unwrap().is_err());
    assert!(manual.await.unwrap().is_err());
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

fn navigated_to(browser: &FakeBrowser, prefix: &str) -> usize {
    browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| {
            matches!(&command.command, BrowserAction::Navigate(url) if url.as_str().starts_with(prefix))
        })
        .count()
}

#[tokio::test]
async fn pull_waiting_for_the_lane_does_not_navigate_a_page_the_user_just_opened() {
    let (engine, browser, _, _) = engine();
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    browser.navigation(0).await;
    let coordinator = PullCoordinator::default();
    let pull = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::manual(),
    );
    wait_for_snapshot(&coordinator, |row| row.state == PullState::Running).await;
    // The session check yields its lane to the visible page; the waiting pull
    // must not take that lane and load its target into the shown view.
    engine.user_browsing(Some(SchoolSite::ScienceLms)).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(navigated_to(&browser, "https://lms.s.isct.ac.jp/"), 0);
    assert!(!pull.is_finished());
    assert!(!check.is_finished());

    engine.user_browsing(None).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while navigated_to(&browser, "https://lms.s.isct.ac.jp/") == 0
            && navigated_to(&browser, "https://isct.ex-tic.com/") < 2
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    coordinator.shutdown().await;
    assert!(pull.await.unwrap().is_err());
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

fn lms_courses_relay(script: &str, id: u64) -> Url {
    extraction_relay(
        script,
        serde_json::json!({
            "status": "ok",
            "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
            "language": "en",
            "coursesComplete": true,
            "courses": [{
                "id": id,
                "fullname": "Recovered course",
                "shortname": "RECOVER",
                "viewurl": format!("https://lms.s.isct.ac.jp/2025/course/view.php?id={id}")
            }]
        }),
    )
}

#[tokio::test]
async fn hidden_pull_signs_in_with_saved_credentials_and_resumes_at_its_target() {
    let (engine, browser, _, store) =
        engine_with_auth(Arc::new(SchoolAuthService::new(saved_science_vault())));
    let coordinator = PullCoordinator::default();
    let request = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::automatic(),
    );
    let first = browser.navigation(0).await;
    let listener = browser.listener();
    // The expired LMS session redirects the hidden view to the identity provider.
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: first.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/auth").unwrap(),
    });
    let config = login_config(&browser.eval(0).await);
    assert_eq!(config["username"], "student");
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: first.browser,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/").unwrap(),
    });
    let resumed = browser.navigation(1).await;
    assert_eq!(resumed.browser, first.browser);
    assert_eq!(
        resumed.command,
        BrowserAction::Navigate(Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap())
    );
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: first.browser,
            sequence: 3,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    });
    let relay = lms_courses_relay(&browser.extraction_script().await, 333333333);
    listener.decide_navigation(
        PageId {
            browser: first.browser,
            sequence: 3,
        },
        relay,
        true,
    );
    let outcomes: Vec<IngestOutcome> =
        serde_json::from_value(request.await.unwrap().unwrap()).unwrap();
    assert_eq!(outcomes[0].state, IngestState::Ok);
    assert_eq!(
        engine.session_snapshot().unwrap().states.science,
        SchoolSessionState::Active
    );
    let connection = rusqlite::Connection::open(store.path()).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM lms_courses WHERE id='lms-course:333333333'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    coordinator.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn hidden_pulls_without_saved_credentials_report_sign_in_required_promptly() {
    for (site, sign_in, kind) in [
        (
            SchoolSite::ScienceLms,
            "https://isct.ex-tic.com/auth",
            "lms",
        ),
        (
            SchoolSite::Kyomu,
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?Template=userpass_key",
            "session",
        ),
    ] {
        let (engine, browser, _, _) = engine();
        let coordinator = PullCoordinator::default();
        let request = spawn_pull(&engine, &coordinator, site, PullOptions::manual());
        let navigation = browser.navigation(0).await;
        browser.listener().page_loaded(PageLoaded {
            page: PageId {
                browser: navigation.browser,
                sequence: 1,
            },
            cause: None,
            url: Url::parse(sign_in).unwrap(),
        });
        let value = tokio::time::timeout(std::time::Duration::from_secs(5), request)
            .await
            .expect("a sign-in landing must not wait for the page timeout")
            .unwrap()
            .unwrap();
        let outcomes: Vec<IngestOutcome> = serde_json::from_value(value).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            (outcomes[0].kind.as_str(), outcomes[0].state),
            (kind, IngestState::ReconnectRequired)
        );
        coordinator.shutdown().await;
        engine.shutdown().await;
    }
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

#[tokio::test]
async fn full_lms_pull_projects_fallback_without_writing_user_timetable() {
    let (engine, browser, events, store) = engine();
    let coordinator = PullCoordinator::default();
    let request = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::ScienceLms,
        PullOptions::automatic(),
    );
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
            "status":"ok","sourceUrl":"https://lms.s.isct.ac.jp/2025/my/","language":"en",
            "fullScan":true,"coursesComplete":true,
            "courses":[{"id":99,"fullname":"Algorithms","shortname":"CSC.T271[2026]","coursecategory":3,
                "viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=99"}],
            "timetable":{"year":2026,"quarter":"3","slots":[
                {"courseId":99,"day":5,"period":5},{"courseId":99,"day":5,"period":6}
            ]}
        }),
    );
    browser.listener().decide_navigation(
        PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        relay,
        true,
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), request)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let workspace = store.load_workspace().unwrap();
    assert!(workspace["importStatus"]["timetableReviewedAt"].is_null());
    let courses = crate::schedule::scheduled_courses(&workspace, "2026-3Q");
    assert_eq!(courses.len(), 1);
    assert_eq!(courses[0]["meetings"][0]["days"], serde_json::json!([5]));
    assert_eq!(
        courses[0]["meetings"][0]["periods"],
        serde_json::json!([5, 6])
    );
    assert!(store.user_rows("manual_courses").unwrap().is_empty());
    assert!(store.user_rows("manual_memberships").unwrap().is_empty());
    assert_eq!(store.table_rows("lms_courses").unwrap().len(), 1);
    assert!(store.last_import_runs().unwrap()["lms:all"].is_string());
    assert!(!events.0.lock().unwrap().iter().any(|event| matches!(event,
        CoreEvent::SchoolPullProgress(progress) if progress.phase == "timetable")));
    engine.shutdown().await;
}

#[tokio::test]
async fn hidden_catalogue_search_preserves_the_foreground_web_location() {
    let (engine, browser, events, _) = engine();
    let original = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/Default.aspx";
    engine
        .open(SchoolSite::Kyomu, Some(original.into()))
        .await
        .unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 1,
        },
        cause: None,
        url: Url::parse(original).unwrap(),
    });
    for attempt in 0..10_000 {
        if events
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, CoreEvent::SchoolWebUrl { url, .. } if url == original))
        {
            break;
        }
        settle(attempt).await;
    }
    engine.user_browsing(None).unwrap();
    let search = format!(
        "https://kyomu0.gakumu.titech.ac.jp{}?_search=True",
        crate::catalog::KYOMU_SEARCH_PATH
    );
    let coordinator = PullCoordinator::default();
    let request = {
        let engine = engine.clone();
        let search = search.clone();
        tokio::spawn(async move {
            coordinator
                .run(
                    PullTarget::Catalog {
                        year: 2026,
                        query: search.clone(),
                    },
                    PullOptions::manual(),
                    move |context| {
                        let engine = engine.clone();
                        let search = search.clone();
                        async move {
                            engine
                                .catalog_search(&search, None, Default::default(), &context)
                                .await
                                .map_err(Into::into)
                        }
                    },
                )
                .await
        })
    };
    // A direct search URL without a 教務Web session lands on the GetAccess
    // resource list, so the search enters through the entry page first.
    let entry = browser.navigation(0).await;
    assert!(matches!(&entry.command, BrowserAction::Navigate(url)
        if url.as_str() == "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx"));
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/").unwrap(),
    });
    let opened = browser.navigation(1).await;
    assert!(matches!(&opened.command, BrowserAction::Navigate(url) if url.as_str() == search));
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 3,
        },
        cause: None,
        url: Url::parse(&search).unwrap(),
    });
    let relay = extraction_relay(
        &browser.extraction_script().await,
        serde_json::json!({
            "status":"ok", "kind":"catalog", "sourceUrl":search,
            "data":{"complete":true,"shape":"course-search","empty":true,"courses":[],"nextCursor":null,"truncated":false}
        }),
    );
    browser.listener().decide_navigation(
        PageId {
            browser: id,
            sequence: 3,
        },
        relay,
        true,
    );
    request.await.unwrap().unwrap();
    assert!(!events.0.lock().unwrap().iter().any(
        |event| matches!(event, CoreEvent::SchoolWebUrl { url, .. } if url.contains("_search"))
    ));
    let reopened = engine.open(SchoolSite::Kyomu, None).await.unwrap();
    assert_eq!(reopened.url, original);
    assert_eq!(browser.current.lock().unwrap()[&id].as_str(), original);
    engine.shutdown().await;
}

#[tokio::test]
async fn maintenance_redirect_finishes_the_pull_and_recovery_releases_the_browser_lane() {
    let (engine, browser, events, _) = engine();
    *browser.maintenance.lock().unwrap() =
        Some("ただいまメンテナンス中です 2031/01/02 10:00–12:30".into());
    let coordinator = PullCoordinator::default();
    let pull = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::Kyomu,
        PullOptions::manual(),
    );
    let landing = browser.navigation(0).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: landing.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx").unwrap(),
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), pull)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().to_string().contains("2031/01/02"));
    assert!(events.0.lock().unwrap().iter().any(|event| matches!(event, CoreEvent::SchoolIngest(outcome) if outcome.state == IngestState::Maintenance)));
    assert_eq!(coordinator.snapshots().await[0].state, PullState::Failed);
    assert!(engine
        .session_snapshot()
        .unwrap()
        .kyomu_maintenance
        .unwrap()
        .contains("2031/01/02"));

    *browser.maintenance.lock().unwrap() = None;
    let next = spawn_pull(
        &engine,
        &coordinator,
        SchoolSite::Kyomu,
        PullOptions::manual(),
    );
    let landing = browser.navigation(1).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: landing.browser,
            sequence: 2,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/").unwrap(),
    });
    let _ = browser.navigation(2).await;
    assert!(engine
        .session_snapshot()
        .unwrap()
        .kyomu_maintenance
        .is_none());
    assert!(events.0.lock().unwrap().iter().any(|event| matches!(event, CoreEvent::SchoolIngest(outcome) if outcome.kind == "service" && outcome.state == IngestState::Ok)));
    coordinator.shutdown().await;
    assert!(next.await.unwrap().is_err());

    // Signing in during the same outage is a temporary service state, not a
    // credential rejection. It must settle promptly and remain retryable.
    *browser.maintenance.lock().unwrap() = Some("ただいまメンテナンス中です".into());
    let login = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .login(
                    SchoolAuthProfile::TokyoTech,
                    SchoolCredentials {
                        username: "test-student".into(),
                        password: "test-password".into(),
                        otp_secret: String::new(),
                        matrix: String::new(),
                    },
                )
                .await
        })
    };
    let navigation = browser.navigation(3).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx").unwrap(),
    });
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), login)
            .await
            .unwrap()
            .unwrap(),
        Err(SchoolBrowserError::Maintenance(_))
    ));
    assert_eq!(
        engine.session_snapshot().unwrap().states.tokyo_tech,
        SchoolSessionState::Maintenance
    );
    assert!(events.0.lock().unwrap().iter().any(
        |event| matches!(event, CoreEvent::SchoolLogin(outcome) if outcome.state == "maintenance")
    ));
    engine.shutdown().await;
}
