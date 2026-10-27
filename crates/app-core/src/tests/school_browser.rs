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
    /// Leave stops unconfirmed, as a native view that hangs would.
    hold_stop: AtomicBool,
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
        let stop = (command.command == BrowserAction::Stop).then_some(command.action);
        self.commands.lock().unwrap().push(command);
        if let Some(action) = stop.filter(|_| !self.hold_stop.load(Ordering::Acquire)) {
            self.listener().stopped(
                PageId {
                    browser,
                    sequence: 0,
                },
                action,
            );
        }
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

// Native WebViews have no headless mode; drive their real Core coordinator
// through page callbacks. A live portal must not finish HTTP session recovery
// before the target service has established its own session cookie.
#[tokio::test]
async fn transport_sign_in_waits_for_service_after_portal_and_rejects_login_forms() {
    for (profile, site, portal, entry, landing, hop) in [
        (
            SchoolAuthProfile::Science,
            SchoolSite::ScienceLms,
            "https://isct.ex-tic.com/idm/user/portal/",
            crate::lms_session::LMS_HOME,
            crate::lms_session::LMS_HOME,
            "https://lms.s.isct.ac.jp/2025/auth/saml2/login.php",
        ),
        (
            SchoolAuthProfile::TokyoTech,
            SchoolSite::Kyomu,
            SchoolSite::TokyoPortal.start_url(),
            SchoolSite::Kyomu.start_url(),
            "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/Index.aspx",
            SchoolSite::Kyomu.start_url(),
        ),
    ] {
        for accepted in [true, false] {
            let (engine, browser, _, _) = engine();
            let recovery = {
                let engine = engine.clone();
                tokio::spawn(async move { engine.ensure_transport_session(profile).await })
            };
            let portal_navigation = browser.navigation(0).await;
            browser.listener().page_loaded(PageLoaded {
                page: PageId {
                    browser: portal_navigation.browser,
                    sequence: 1,
                },
                cause: None,
                url: Url::parse(portal).unwrap(),
            });
            let service_navigation = browser.navigation(1).await;
            assert_eq!(service_navigation.browser.site, site);
            assert!(
                matches!(&service_navigation.command, BrowserAction::Navigate(url) if url.as_str() == entry)
            );
            assert!(
                !recovery.is_finished(),
                "portal authentication is not a service session"
            );
            browser.listener().page_loaded(PageLoaded {
                page: PageId {
                    browser: service_navigation.browser,
                    sequence: 1,
                },
                cause: None,
                url: Url::parse(hop).unwrap(),
            });
            tokio::task::yield_now().await;
            assert!(
                !recovery.is_finished(),
                "an SSO hop is not the service landing page"
            );
            browser.listener().page_loaded(PageLoaded {
                page: PageId {
                    browser: service_navigation.browser,
                    sequence: 2,
                },
                cause: None,
                url: Url::parse(if accepted {
                    landing
                } else {
                    match profile {
                        SchoolAuthProfile::Science => "https://isct.ex-tic.com/auth/session",
                        SchoolAuthProfile::TokyoTech => {
                            "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login"
                        }
                    }
                })
                .unwrap(),
            });
            assert_eq!(recovery.await.unwrap().unwrap(), accepted);
            engine.shutdown().await;
        }
    }
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

// Drives the real enrollment coordinator through native page/title events.
// A school rejection must leave the saved key untouched; success must retain
// the password and only promote the correlated candidate after registered.
#[tokio::test]
async fn otp_enrollment_submits_visible_form_and_promotes_only_after_school_success() {
    for (accepted, reset) in [(false, false), (true, false), (true, true)] {
        let auth = Arc::new(SchoolAuthService::new(saved_science_vault()));
        let (engine, browser, events, _) = engine_with_auth(auth.clone());
        let operation = engine.begin_school_otp_setup().unwrap();
        let settings = "https://isct.ex-tic.com/idm/user/mfa_setting/";
        engine
            .navigate(SchoolSite::SciencePortal, settings)
            .unwrap();
        let browser_id = browser.navigation(0).await.browser;
        let listener = browser.listener();
        let load = |sequence, url: &str| {
            listener.page_loaded(PageLoaded {
                page: PageId {
                    browser: browser_id,
                    sequence,
                },
                cause: None,
                url: Url::parse(url).unwrap(),
            });
        };
        let reply = |config: &serde_json::Value, kind: &str, extra: serde_json::Value| {
            let mut packet = serde_json::json!({"operation":config["operation"], "sequence":config["sequence"], "token":config["token"], "kind":kind});
            packet
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            listener.title_changed(
                PageId {
                    browser: browser_id,
                    sequence: config["sequence"].as_u64().unwrap(),
                },
                format!(
                    "{}{}",
                    config["relay"].as_str().unwrap(),
                    URL_SAFE_NO_PAD.encode(packet.to_string())
                ),
            );
        };
        load(1, settings);
        let wait = {
            let engine = engine.clone();
            tokio::spawn(async move { engine.complete_school_otp_setup(operation, reset).await })
        };
        let inspect = login_config(&browser.eval(0).await);
        reply(
            &inspect,
            "settings",
            serde_json::json!({"owner":"student", "registered":reset}),
        );
        let offset = if reset {
            let remove = login_config(&browser.eval(1).await);
            assert_eq!(remove["action"], "remove");
            assert_eq!(remove["owner"], "student");
            load(2, settings);
            let cleared = login_config(&browser.eval(2).await);
            reply(
                &cleared,
                "settings",
                serde_json::json!({"owner":"student", "registered":false}),
            );
            2
        } else {
            0
        };
        let register = browser.navigation(1).await;
        assert!(
            matches!(register.command, BrowserAction::Navigate(ref url) if url.path() == "/idm/user/totp_setting/add")
        );
        load(
            2 + u64::from(reset),
            "https://isct.ex-tic.com/idm/user/totp_setting/add",
        );
        let inspect = login_config(&browser.eval(1 + offset).await);
        let next = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        reply(
            &inspect,
            "secret",
            serde_json::json!({"owner":"student", "secret":next}),
        );
        let submit = login_config(&browser.eval(2 + offset).await);
        assert_eq!(submit["action"], "submit");
        assert_eq!(submit["owner"], "student");
        assert_eq!(submit["secret"], next);
        assert_eq!(submit["code"].as_str().unwrap().len(), 6);
        assert_eq!(
            auth.load(SchoolAuthProfile::Science)
                .unwrap()
                .unwrap()
                .otp_secret,
            "JBSWY3DPEHPK3PXP"
        );
        // Duplicate read callback cannot complete or resubmit this operation.
        reply(
            &inspect,
            "secret",
            serde_json::json!({"owner":"student", "secret":next}),
        );
        assert!(!wait.is_finished());
        load(3 + u64::from(reset), settings);
        let confirm = login_config(&browser.eval(3 + offset).await);
        reply(
            &confirm,
            "settings",
            serde_json::json!({"owner":"student", "registered":accepted}),
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.is_ok(), accepted);
        let saved = auth.load(SchoolAuthProfile::Science).unwrap().unwrap();
        assert_eq!(
            saved.otp_secret,
            if accepted { next } else { "JBSWY3DPEHPK3PXP" }
        );
        assert_eq!(saved.password, "password");
        assert_eq!(events.0.lock().unwrap().iter().any(|event| matches!(event, CoreEvent::SchoolLogin(event) if event.message == "OTP setup complete")), accepted);
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

#[tokio::test]
async fn assignment_upload_targets_the_edit_page_once_and_rejects_other_destinations() {
    use crate::school_browser::SubmissionFile;
    let (engine, browser, _, _) = engine();
    // Shells hand over bytes; Core encodes them as the page's uploader takes them.
    let files = || {
        vec![SubmissionFile::from_bytes(
            "report.txt".into(),
            b"my report",
        )]
    };
    assert_eq!(files()[0].data, "bXkgcmVwb3J0");
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
    value: Arc<Mutex<HashMap<String, String>>>,
    block: Arc<AtomicBool>,
    started: mpsc::Sender<()>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}

impl SecretVault for BlockingVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.value
            .lock()
            .unwrap()
            .insert(account.into(), secret.into());
        Ok(())
    }
    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        // One vault read holds: a sign-in reads several items.
        if self.block.swap(false, Ordering::AcqRel) {
            let _ = self.started.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        Ok(self.value.lock().unwrap().get(account).cloned())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.value.lock().unwrap().remove(account);
        Ok(())
    }
    /// As on an Apple device: iCloud Keychain may replace the item.
    fn shared(&self, account: &str) -> bool {
        account == "school-accounts"
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
        engine.session_snapshot().unwrap().profiles.science.state,
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
        engine.session_snapshot().unwrap().profiles.tokyo_tech.state,
        SchoolSessionState::Failed
    );

    // A shell wake alone must recover it; no click, focus or restart.
    engine.retry_sessions();
    // The timed-out check's navigation and the stop of its view.
    assert_eq!(browser.commands.lock().unwrap().len(), 2);
    tokio::time::advance(std::time::Duration::from_secs(90)).await;
    engine.retry_sessions();
    engine.retry_sessions(); // Repeated wakes cannot queue duplicate checks.
    let navigation = browser.navigation(1).await;
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: navigation.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://portal.nap.gsic.titech.ac.jp/portal.pl").unwrap(),
    });
    for attempt in 0..6_000 {
        if engine.session_snapshot().unwrap().profiles.tokyo_tech.state
            == SchoolSessionState::NotConfigured
        {
            break;
        }
        settle(attempt).await;
    }
    assert_eq!(
        engine.session_snapshot().unwrap().profiles.tokyo_tech.state,
        SchoolSessionState::NotConfigured
    );
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    engine.retry_sessions();
    assert_eq!(browser.commands.lock().unwrap().len(), 3);
    engine.shutdown().await;
}

// A job's time runs from admission: a check queued behind another spends its
// wait for the lane, so a stuck lane cannot stretch it to twice the timeout.
// A visible school page holding it back is not its time.
#[tokio::test(start_paused = true)]
async fn a_session_job_has_one_deadline_from_admission_that_visible_pages_do_not_spend() {
    let (engine, browser, _, _) = engine();
    let check = |profile| {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(profile).await })
    };
    let science = check(SchoolAuthProfile::Science);
    browser.navigation(0).await;
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    let tokyo = check(SchoolAuthProfile::TokyoTech);
    // Admitted now: `advance` moves the clock before the job first runs.
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    assert!(matches!(
        science.await.unwrap(),
        Err(SchoolBrowserError::Timeout(_))
    ));
    assert_eq!(
        browser.navigation(1).await.browser.site,
        SchoolSite::TokyoPortal
    );
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    let tokyo = tokio::time::timeout(std::time::Duration::from_secs(1), tokyo)
        .await
        .expect("the wait for the lane counts toward the job's deadline");
    assert!(matches!(
        tokyo.unwrap(),
        Err(SchoolBrowserError::Timeout(_))
    ));

    engine.user_browsing(Some(SchoolSite::Kyomu)).unwrap();
    let held = check(SchoolAuthProfile::Science);
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(200)).await;
    assert!(!held.is_finished());
    engine.user_browsing(None).unwrap();
    assert_eq!(
        browser.navigation(2).await.browser.site,
        SchoolSite::SciencePortal
    );
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    assert!(
        !held.is_finished(),
        "the time behind a visible page did not count"
    );
    engine.shutdown().await;
    assert_eq!(held.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

// An attempt that ran out of time leaves its page mid-way. Its view is
// stopped, and the next check reuses it only once the view confirms: a page
// of the abandoned attempt reported late is not the new check's. A view that
// never confirms is replaced instead of waited on.
#[tokio::test(start_paused = true)]
async fn an_abandoned_attempt_stops_its_view_before_the_view_is_used_again() {
    let (engine, browser, _, _) = engine();
    browser.hold_stop.store(true, Ordering::Release);
    let check = || {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::TokyoTech).await })
    };
    let last_stop = || {
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .rfind(|command| command.command == BrowserAction::Stop)
            .cloned()
            .expect("the timed-out check's view was stopped")
    };
    let navigations = || {
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
            .count()
    };
    let first = check();
    let abandoned = browser.navigation(0).await.browser;
    tokio::time::advance(std::time::Duration::from_secs(91)).await;
    assert!(matches!(
        first.await.unwrap(),
        Err(SchoolBrowserError::Timeout(_))
    ));
    let stop = last_stop();
    assert_eq!(stop.browser, abandoned);

    let second = check();
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert_eq!(navigations(), 1, "the view has not confirmed its stop");
    let page = |sequence| PageId {
        browser: abandoned,
        sequence,
    };
    browser.listener().stopped(page(3), stop.action);
    assert_eq!(browser.navigation(1).await.browser, abandoned);
    browser.listener().page_loaded(PageLoaded {
        page: page(3),
        cause: None,
        url: Url::parse("https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList").unwrap(),
    });
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(!second.is_finished(), "a page from before the stop is late");
    browser.listener().page_loaded(PageLoaded {
        page: page(4),
        cause: None,
        url: Url::parse("https://portal.nap.gsic.titech.ac.jp/portal.pl").unwrap(),
    });
    assert!(!second.await.unwrap().unwrap());

    let third = check();
    browser.navigation(2).await;
    tokio::time::advance(std::time::Duration::from_secs(91)).await;
    assert!(matches!(
        third.await.unwrap(),
        Err(SchoolBrowserError::Timeout(_))
    ));
    let fourth = check();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_secs(6)).await;
    let replaced = browser.navigation(3).await.browser;
    assert_eq!(replaced.site, abandoned.site);
    assert!(replaced.generation > abandoned.generation);
    engine.shutdown().await;
    assert_eq!(fourth.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

#[tokio::test(start_paused = true)]
async fn connect_timeout_emits_error_and_failed_session() {
    let (engine, browser, events, _) = engine();
    let connect = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .connect(
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
    let result = connect.await.unwrap().unwrap();
    assert_eq!(result.state, SchoolSessionState::Failed);
    assert_eq!(
        result.problem.map(|problem| problem.kind),
        Some(SchoolSignInProblemKind::Unavailable)
    );
    assert!(!result.verified);
    assert!(events.0.lock().unwrap().iter().any(|event| matches!(
        event,
        CoreEvent::SchoolLogin(SchoolLoginEvent { profile: SchoolAuthProfile::Science, state, .. })
            if state == "error"
    )));
    engine.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn heartbeat_timeout_rejects_the_old_login_relay_and_revealing_the_page_can_connect() {
    let (started_send, _started_receive) = mpsc::channel();
    let (_release_send, release_receive) = mpsc::channel();
    let vault = BlockingVault {
        value: Arc::default(),
        block: Arc::new(AtomicBool::new(false)),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .stage(
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

    // The native view already shows the login page: revealing it generates
    // no new load callback, so ownership transfer must arm saved login.
    engine.open(SchoolSite::SciencePortal, None).await.unwrap();
    let next_config = login_config(&browser.eval(1).await);
    assert_ne!(next_config["run"], first_config["run"]);
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: first_navigation.browser,
            sequence: 3,
        },
        cause: None,
        url: Url::parse("https://lms.s.isct.ac.jp/2025/").unwrap(),
    });
    for attempt in 0..6_000 {
        if engine.session_snapshot().unwrap().profiles.science.state == SchoolSessionState::Active {
            break;
        }
        settle(attempt).await;
    }
    assert_eq!(
        engine.session_snapshot().unwrap().profiles.science.state,
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
    let block = Arc::new(AtomicBool::new(false));
    let vault = BlockingVault {
        value: Arc::default(),
        block: block.clone(),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .stage(
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
        value: Arc::default(),
        block: Arc::new(AtomicBool::new(false)),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .stage(
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
        if snapshot.profiles.science.state == SchoolSessionState::NotConfigured
            && snapshot.profiles.tokyo_tech.state == SchoolSessionState::NotConfigured
        {
            engine.shutdown().await;
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("both session requests did not settle");
}

// A caller that stops waiting (a download's timeout, a dropped request) must
// not stop the sign-in it started. The job goes on with nobody waiting, and
// the next caller joins it instead of queueing behind a paused one.
#[tokio::test]
async fn a_session_job_keeps_running_after_its_only_caller_stops_waiting() {
    let (engine, browser, _, _) = engine();
    let first = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .ensure_transport_session(SchoolAuthProfile::Science)
                .await
        })
    };
    let portal = browser.navigation(0).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: portal.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse("https://isct.ex-tic.com/idm/user/portal/").unwrap(),
    });
    let service = browser.navigation(1).await;
    assert_eq!(service.browser.site, SchoolSite::ScienceLms);
    let second = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .ensure_transport_session(SchoolAuthProfile::Science)
                .await
        })
    };
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: service.browser,
            sequence: 1,
        },
        cause: None,
        url: Url::parse(crate::lms_session::LMS_HOME).unwrap(),
    });
    assert!(second.await.unwrap().unwrap());
    assert_eq!(
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
            .count(),
        2,
        "the second caller joined the running job"
    );
    engine.shutdown().await;
}

fn saved_science_vault() -> BlockingVault {
    let (started_send, _started_receive) = mpsc::channel();
    let (_release_send, release_receive) = mpsc::channel();
    let vault = BlockingVault {
        value: Arc::default(),
        block: Arc::new(AtomicBool::new(false)),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    SchoolAuthService::new(vault.clone())
        .stage(
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
        engine.session_snapshot().unwrap().profiles.science.state,
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
    let connect = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .connect(SchoolAuthProfile::Science, manual_credentials())
                .await
        })
    };
    browser.navigation(0).await;
    engine
        .credentials_changed(SchoolAuthProfile::Science, false)
        .unwrap();
    let result = connect.await.unwrap().unwrap();
    assert_eq!(result.state, SchoolSessionState::NotConfigured);
    assert!(!result.verified);
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
    let connect = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .connect(SchoolAuthProfile::Science, manual_credentials())
                .await
        })
    };
    browser.navigation(0).await;
    engine.user_browsing(Some(SchoolSite::Kyomu)).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!connect.is_finished());
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
    assert_eq!(
        connect.await.unwrap().unwrap().state,
        SchoolSessionState::Active
    );
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

/// Drives hidden session checks to the Science sign-in page, one after
/// another on the same view, as launch and focus do.
struct Checks {
    engine: Arc<SchoolBrowserEngine>,
    browser: Arc<FakeBrowser>,
    navigations: usize,
    sequence: u64,
}

impl Checks {
    fn new(engine: &Arc<SchoolBrowserEngine>, browser: &Arc<FakeBrowser>) -> Self {
        Self {
            engine: engine.clone(),
            browser: browser.clone(),
            navigations: 0,
            sequence: 0,
        }
    }

    async fn load(&mut self, url: &str) -> PageId {
        let navigation = self.browser.navigation(self.navigations).await;
        self.navigations += 1;
        self.sequence += 1;
        let page = PageId {
            browser: navigation.browser,
            sequence: self.sequence,
        };
        self.browser.listener().page_loaded(PageLoaded {
            page,
            cause: None,
            url: Url::parse(url).unwrap(),
        });
        page
    }

    /// The page the school opens by itself after a form, in the same view.
    fn follow(&mut self, page: PageId, url: &str) {
        self.sequence += 1;
        self.browser.listener().page_loaded(PageLoaded {
            page: PageId {
                browser: page.browser,
                sequence: self.sequence,
            },
            cause: None,
            url: Url::parse(url).unwrap(),
        });
    }

    /// A session check whose page is the sign-in form.
    async fn reach_login(
        &mut self,
    ) -> (
        tokio::task::JoinHandle<Result<bool, SchoolBrowserError>>,
        PageId,
    ) {
        let check = {
            let engine = self.engine.clone();
            tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
        };
        let page = self.load("https://isct.ex-tic.com/auth/session").await;
        (check, page)
    }

    fn scripts(&self) -> Vec<serde_json::Value> {
        self.browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter_map(|command| match &command.command {
                BrowserAction::Eval(script) if script.contains("isct-companion-login") => {
                    Some(login_config(script))
                }
                _ => None,
            })
            .collect()
    }

    /// What the injected script reports through the page title.
    fn relay(&self, page: PageId, state: &str, message: &str, submitted: &[&str]) {
        let config = self.scripts().pop().unwrap();
        let payload = serde_json::json!({
            "profile": config["profile"],
            "run": config["run"],
            "seq": 1,
            "state": state,
            "message": message,
            "submitted": submitted,
        });
        self.browser.listener().title_changed(
            page,
            format!(
                "{}{}",
                config["authRelay"].as_str().unwrap(),
                URL_SAFE_NO_PAD.encode(payload.to_string())
            ),
        );
    }

    fn science(&self) -> SchoolProfileSession {
        self.engine.session_snapshot().unwrap().profiles.science
    }
}

fn vault_item(vault: &BlockingVault, account: &str) -> Option<String> {
    vault.value.lock().unwrap().get(account).cloned()
}

// A wrong password must never loop until the school locks the account: not on
// the next check, not after a relaunch. New credentials, typed here or synced
// from another device, are what lifts the pause.
#[tokio::test]
async fn a_wrong_password_pauses_automatic_sign_in_until_the_credentials_change() {
    let vault = saved_science_vault();
    let (engine, browser, _, _) = engine_with_auth(Arc::new(SchoolAuthService::new(vault.clone())));
    let mut checks = Checks::new(&engine, &browser);
    let (check, page) = checks.reach_login().await;
    browser.eval(0).await;
    checks.relay(
        page,
        "invalid-credentials",
        "Login failed.",
        &["identifier", "password"],
    );
    assert!(!check.await.unwrap().unwrap());
    let science = checks.science();
    assert_eq!(science.state, SchoolSessionState::Failed);
    assert_eq!(science.auto, SchoolAutoSignIn::Paused);
    assert_eq!(
        science.problem,
        Some(SchoolSignInProblem {
            kind: SchoolSignInProblemKind::InvalidCredentials,
            message: "Login failed.".into(),
        })
    );

    let (check, _) = checks.reach_login().await;
    assert!(!check.await.unwrap().unwrap());
    assert_eq!(checks.scripts().len(), 1);
    for attempt in 0..6_000 {
        if vault_item(&vault, "school-candidates")
            .is_some_and(|item| item.contains("invalid-credentials"))
        {
            break;
        }
        settle(attempt).await;
    }
    engine.shutdown().await;

    // Relaunched: the candidate remembers why the school turned it down.
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    let (engine, browser, _, _) = engine_with_auth(auth.clone());
    let mut checks = Checks::new(&engine, &browser);
    let (check, _) = checks.reach_login().await;
    assert!(!check.await.unwrap().unwrap());
    assert!(checks.scripts().is_empty());
    assert_eq!(checks.science().auto, SchoolAutoSignIn::Paused);

    // Another device signs in with the right password: iCloud Keychain
    // replaces the synchronized item, and the next check uses it at once.
    let other = SchoolAuthService::new(vault.clone());
    let corrected = SchoolCredentials {
        username: "student".into(),
        password: "corrected".into(),
        otp_secret: "JBSWY3DPEHPK3PXP".into(),
        matrix: String::new(),
    };
    other
        .import_sync_payload(
            &serde_json::json!({
                "version": 1,
                "accounts": { "science": corrected },
            })
            .to_string(),
        )
        .unwrap();
    let (check, page) = checks.reach_login().await;
    let config = login_config(&browser.eval(0).await);
    assert_eq!(config["password"], "corrected");
    assert_eq!(checks.science().auto, SchoolAutoSignIn::Allowed);

    // The same verdict on new credentials typed here pauses again, and the
    // person's next change in Settings lifts it again.
    checks.relay(page, "invalid-credentials", "Login failed.", &["password"]);
    assert!(!check.await.unwrap().unwrap());
    auth.stage(SchoolAuthProfile::Science, manual_credentials())
        .unwrap();
    engine
        .credentials_changed(SchoolAuthProfile::Science, true)
        .unwrap();
    assert_eq!(checks.science().problem, None);
    let (check, _) = checks.reach_login().await;
    assert_eq!(
        login_config(&browser.eval(1).await)["username"],
        "typed-in-settings"
    );
    engine.shutdown().await;
    assert_eq!(check.await.unwrap(), Err(SchoolBrowserError::Stopped));
}

// A turned-down one-time password is often a clock matter: Core tries once
// a later window has started, a few times, then waits for the person.
#[tokio::test(start_paused = true)]
async fn a_rejected_code_is_retried_in_a_later_window_a_few_times() {
    let (engine, browser, _, _) =
        engine_with_auth(Arc::new(SchoolAuthService::new(saved_science_vault())));
    let mut checks = Checks::new(&engine, &browser);
    for round in 1..=3usize {
        let (check, page) = checks.reach_login().await;
        browser.eval(round - 1).await;
        checks.relay(
            page,
            "invalid-otp",
            "Invalid code.",
            &["identifier", "password", "otp"],
        );
        assert!(!check.await.unwrap().unwrap());
        let science = checks.science();
        assert_eq!(
            science.problem.unwrap().kind,
            SchoolSignInProblemKind::InvalidOtp
        );
        if round == 3 {
            assert_eq!(science.auto, SchoolAutoSignIn::Paused);
            break;
        }
        assert_eq!(science.auto, SchoolAutoSignIn::RetryLater);
        // Within the window the code was turned down in, nothing is sent.
        let (check, _) = checks.reach_login().await;
        assert!(!check.await.unwrap().unwrap());
        assert_eq!(checks.scripts().len(), round);
        tokio::time::advance(std::time::Duration::from_secs(30 * round as u64 + 1)).await;
    }
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    engine.retry_sessions();
    let commands = browser.commands.lock().unwrap().len();
    tokio::task::yield_now().await;
    assert_eq!(browser.commands.lock().unwrap().len(), commands);
    let (check, _) = checks.reach_login().await;
    assert!(!check.await.unwrap().unwrap());
    assert_eq!(checks.scripts().len(), 3);
    engine.shutdown().await;
}

// Only a sign-in that submitted every saved secret and reached the school
// proves them; a portal session that was still valid proves nothing. Until
// then nothing reaches the synchronized item other devices read.
#[tokio::test]
async fn connect_promotes_only_credentials_the_school_saw_in_full() {
    let vault = saved_science_vault();
    let auth = Arc::new(SchoolAuthService::new(vault.clone()));
    let (engine, browser, _, _) = engine_with_auth(auth.clone());
    let credentials = auth.load(SchoolAuthProfile::Science).unwrap().unwrap();
    let mut checks = Checks::new(&engine, &browser);

    let connect = {
        let engine = engine.clone();
        let credentials = credentials.clone();
        tokio::spawn(async move {
            engine
                .connect(SchoolAuthProfile::Science, credentials)
                .await
        })
    };
    checks.load("https://lms.s.isct.ac.jp/2025/").await;
    let result = connect.await.unwrap().unwrap();
    assert_eq!(result.state, SchoolSessionState::Active);
    assert!(!result.verified);
    assert!(vault_item(&vault, "school-accounts").is_none());

    let connect = {
        let engine = engine.clone();
        let credentials = credentials.clone();
        tokio::spawn(async move {
            engine
                .connect(SchoolAuthProfile::Science, credentials)
                .await
        })
    };
    let page = checks.load("https://isct.ex-tic.com/auth/session").await;
    browser.eval(0).await;
    checks.relay(page, "progress", "password", &["identifier", "password"]);
    checks.relay(page, "progress", "otp", &[]);
    checks.follow(page, "https://isct.ex-tic.com/idm/user/portal");
    let result = connect.await.unwrap().unwrap();
    assert_eq!(
        result,
        SchoolConnectResult {
            state: SchoolSessionState::Active,
            problem: None,
            verified: true,
        }
    );
    assert!(vault_item(&vault, "school-accounts").is_some_and(|item| item.contains("student")));
    assert!(vault_item(&vault, "school-candidates").is_none());
    engine.shutdown().await;
}

// Credentials typed in Settings are the person's answer: a check already
// running gives way to the connect instead of holding it back with the old
// saved sign-in, and everyone waiting on the profile, before or during the
// connect, gets the connect's verdict rather than a second sign-in.
#[tokio::test]
async fn a_connect_takes_the_profile_over_and_answers_everyone_waiting_on_it() {
    let auth = Arc::new(SchoolAuthService::new(saved_science_vault()));
    auth.stage(SchoolAuthProfile::Science, manual_credentials())
        .unwrap();
    let (engine, browser, _, _) = engine_with_auth(auth);
    let mut checks = Checks::new(&engine, &browser);
    let connect = || {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .connect(SchoolAuthProfile::Science, manual_credentials())
                .await
        })
    };
    let check = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    browser.navigation(0).await;
    checks.navigations = 1;
    let first = connect();
    let page = checks.load("https://isct.ex-tic.com/auth/session").await;
    assert!(
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .any(|command| command.command == BrowserAction::Stop),
        "the check that gave way was stopped"
    );
    assert_eq!(
        login_config(&browser.eval(0).await)["username"],
        "typed-in-settings"
    );
    checks.relay(
        page,
        "invalid-credentials",
        "Login failed.",
        &["identifier", "password"],
    );
    let result = first.await.unwrap().unwrap();
    assert_eq!(result.state, SchoolSessionState::Failed);
    assert_eq!(
        result.problem.map(|problem| problem.kind),
        Some(SchoolSignInProblemKind::InvalidCredentials)
    );
    assert!(!check.await.unwrap().unwrap());

    let second = connect();
    let page = checks.load("https://isct.ex-tic.com/auth/session").await;
    browser.eval(1).await;
    let joined = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.ensure_session(SchoolAuthProfile::Science).await })
    };
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    checks.relay(page, "progress", "password", &["identifier", "password"]);
    checks.relay(page, "progress", "otp", &[]);
    checks.follow(page, "https://isct.ex-tic.com/idm/user/portal");
    assert!(second.await.unwrap().unwrap().verified);
    assert!(joined.await.unwrap().unwrap());
    assert_eq!(
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
            .count(),
        3,
        "the check asked for during the connect joined it"
    );
    engine.shutdown().await;
}

// Saving proved credentials can outlast the sign-in that proved them. When
// new credentials arrive meanwhile, the late save must not publish the old
// sign-in as the profile's current session.
#[tokio::test]
async fn a_slow_credential_save_cannot_publish_a_superseded_sign_in() {
    let (started_send, started_receive) = mpsc::channel();
    let (release_send, release_receive) = mpsc::channel();
    let block = Arc::new(AtomicBool::new(false));
    let vault = BlockingVault {
        value: Arc::default(),
        block: block.clone(),
        started: started_send,
        release: Arc::new(Mutex::new(release_receive)),
    };
    let auth = Arc::new(SchoolAuthService::new(vault));
    let (engine, browser, _, _) = engine_with_auth(auth);
    let mut checks = Checks::new(&engine, &browser);
    let connect = {
        let engine = engine.clone();
        tokio::spawn(async move {
            engine
                .connect(SchoolAuthProfile::Science, manual_credentials())
                .await
        })
    };
    let page = checks.load("https://isct.ex-tic.com/auth/session").await;
    browser.eval(0).await;
    checks.relay(page, "progress", "password", &["identifier", "password"]);
    checks.relay(page, "progress", "otp", &[]);
    block.store(true, Ordering::Release);
    checks.follow(page, "https://isct.ex-tic.com/idm/user/portal");
    tokio::task::spawn_blocking(move || {
        started_receive.recv_timeout(std::time::Duration::from_secs(5))
    })
    .await
    .unwrap()
    .unwrap();
    engine
        .credentials_changed(SchoolAuthProfile::Science, true)
        .unwrap();
    release_send.send(()).unwrap();
    for attempt in 0..2_000 {
        settle(attempt).await;
    }
    assert_eq!(checks.science().state, SchoolSessionState::Pending);
    engine.shutdown().await;
    let _ = connect.await;
}
