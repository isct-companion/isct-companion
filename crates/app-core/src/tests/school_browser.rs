use super::*;
use crate::{
    data::DataStore,
    events::CoreEvent,
    lifecycle::SchoolSite,
    ports::{EventError, EventSink, SecretError, SecretVault},
    school_auth::{SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    school_session::{
        tests::{MemoryVault, School, WebStore},
        SchoolSessions, WebClear,
    },
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
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
    disposed: Mutex<Vec<BrowserId>>,
    /// Every `cover` Core sent.
    covers: Mutex<Vec<(BrowserId, bool)>>,
    /// A host like the Windows interim: no resubmission facts, popups as
    /// separate windows.
    plain_host: AtomicBool,
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
    fn capabilities(&self) -> HostCapabilities {
        let full = !self.plain_host.load(Ordering::Acquire);
        HostCapabilities {
            form_resubmission: full,
            inline_popups: full,
        }
    }

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
        self.commands.lock().unwrap().push(command);
        Ok(())
    }

    fn current_url(&self, browser: BrowserId) -> Result<Option<Url>, BrowserError> {
        Ok(self.current.lock().unwrap().get(&browser).cloned())
    }

    fn dispose(&self, browser: BrowserId) -> Result<(), BrowserError> {
        self.current.lock().unwrap().remove(&browser);
        self.disposed.lock().unwrap().push(browser);
        Ok(())
    }

    fn cover(&self, browser: BrowserId, covered: bool) -> Result<(), BrowserError> {
        self.covers.lock().unwrap().push((browser, covered));
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
// through page callbacks.
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
        .open(
            SchoolSite::ScienceLms,
            "web",
            SchoolWebEntry::Go {
                url: target.clone(),
            },
        )
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
    engine_on(auth, FakeBrowser::default())
}

fn engine_on(
    auth: Arc<SchoolAuthService>,
    browser: FakeBrowser,
) -> (
    Arc<SchoolBrowserEngine>,
    Arc<FakeBrowser>,
    Arc<Events>,
    DataStore,
) {
    let path = std::env::temp_dir().join(format!("isct-browser-{}", Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    let browser = Arc::new(browser);
    let events = Arc::new(Events::default());
    // No web cookie port: these protocol cases never seed an identity.
    let sessions = SchoolSessions::new(None, auth);
    let engine = SchoolBrowserEngine::new(sessions, events.clone(), browser.clone()).unwrap();
    (engine, browser, events, store)
}

fn request(
    page: PageId,
    raw: &str,
    main_frame: bool,
    kind: NavigationKind,
    user_gesture: bool,
) -> NavigationRequest {
    NavigationRequest {
        page,
        url: Url::parse(raw).unwrap(),
        method: Some("GET".into()),
        main_frame,
        kind,
        user_gesture,
    }
}

fn window(opener: PageId, raw: &str) -> NewWindowRequest {
    NewWindowRequest {
        opener,
        url: Url::parse(raw).unwrap(),
        method: Some("GET".into()),
        kind: NavigationKind::Other,
        user_gesture: true,
        name: Some("picker".into()),
        features: WindowFeatures::default(),
    }
}

fn notices(events: &Events) -> Vec<SchoolWebNotice> {
    events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::SchoolWebNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .collect()
}

/// Every `school-web-page` the app was told, in order.
fn pages(events: &Events) -> Vec<SchoolWebPage> {
    events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::SchoolWebPage(page) => Some(page.clone()),
            _ => None,
        })
        .collect()
}

async fn url_shown(events: &Events, expected: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if pages(events)
                .iter()
                .any(|page| page.url.as_deref() == Some(expected))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expected} was not shown"));
}

fn evals(browser: &FakeBrowser) -> usize {
    browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Eval(_)))
        .count()
}

// Native WebViews have no headless mode; drive their real Core coordinator
// through page callbacks. The rule table is keyed by who the view serves:
// the person browses anything over HTTPS (rich editors need inherited-origin
// frames), plain HTTP they follow goes to the system browser, and a hidden

#[tokio::test]
async fn the_rule_table_lets_the_person_browse_and_sends_cleartext_to_the_system_browser() {
    let (engine, browser, events, _) = engine();
    let site = SchoolSite::ScienceLms;
    let official =
        "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission#editor";
    engine
        .open(
            site,
            "web",
            SchoolWebEntry::Go {
                url: official.into(),
            },
        )
        .await
        .unwrap();
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
    url_shown(&events, official).await;
    engine
        .set_customization(
            site,
            SchoolPageCustomization {
                css: "body{color:red}".into(),
                javascript: "schoolCustomization()".into(),
            },
        )
        .unwrap();
    let decide = |raw: &str, main_frame: bool, kind: NavigationKind, gesture: bool| {
        listener.decide_navigation(request(page, raw, main_frame, kind, gesture))
    };
    for main_frame in [false, true] {
        for raw in [
            "about:blank",
            "about:srcdoc",
            "https://docs.google.com/forms/d/test/viewform?entry.1=a%20b#section",
        ] {
            assert_eq!(
                decide(raw, main_frame, NavigationKind::Other, false),
                NavigationDecision::Allow,
                "{raw}"
            );
        }
    }
    assert_eq!(
        decide("data:text/html,frame", false, NavigationKind::Other, false),
        NavigationDecision::Allow
    );
    for (raw, reason) in [
        ("data:text/html,test", DenyReason::Scheme),
        ("javascript:alert(1)", DenyReason::Scheme),
        ("file:///tmp/private", DenyReason::Scheme),
        ("https://user:secret@example.com/", DenyReason::Credentials),
    ] {
        assert_eq!(
            decide(raw, true, NavigationKind::Link, true),
            NavigationDecision::Deny(reason),
            "{raw}"
        );
    }
    // 「緊急時避難場所」: a cleartext PDF link opens in the system browser;
    // nothing relaxes ATS for it.
    let cleartext = "http://www2.gakumu.titech.ac.jp/hinan.pdf";
    assert_eq!(
        decide(cleartext, true, NavigationKind::Link, false),
        NavigationDecision::External(Url::parse(cleartext).unwrap())
    );
    assert_eq!(
        decide(cleartext, false, NavigationKind::Link, true),
        NavigationDecision::Deny(DenyReason::Cleartext)
    );
    assert_eq!(
        listener.new_window(window(page, cleartext)),
        NewWindowDecision::External(Url::parse(cleartext).unwrap())
    );
    // A script redirect nobody asked for is refused and shown on the page
    // with the address, for Open in browser.
    assert_eq!(
        decide(cleartext, true, NavigationKind::Other, false),
        NavigationDecision::Deny(DenyReason::Cleartext)
    );
    let refused = Some(SchoolWebFailure {
        url: Some(cleartext.into()),
        reason: LoadFailure::Cleartext,
    });
    assert_eq!(engine.page(site).notice, refused);

    let external = "https://www.google.com/search?q=linear%20algebra&hl=ja#results";
    let resolved = browsing_target(external, site).unwrap();
    assert_eq!(resolved.site, site);
    assert_eq!(resolved.url, external);
    assert!(automation_target(external).is_none());
    let scripts_before = evals(&browser);
    engine.navigate(site, external).unwrap();
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 2,
        },
        cause: None,
        url: Url::parse(external).unwrap(),
    });
    url_shown(&events, external).await;
    assert_eq!(
        evals(&browser),
        scripts_before,
        "external page received a school script"
    );
    assert_eq!(
        engine.page(site).notice,
        None,
        "Core's navigation clears it"
    );
    // Back and Forward are the engine's own history. This host never says
    // where it can go, so the app asks the engine anyway.
    let shown = engine.page(site);
    assert_eq!(
        (shown.back, shown.forward),
        (SchoolWebStep::Unknown, SchoolWebStep::Unknown)
    );
    assert!(engine.back(site).unwrap());
    assert!(engine.forward(site).unwrap());
    let steps: Vec<_> = browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| {
            command.browser == id
                && matches!(
                    command.command,
                    BrowserAction::GoBack | BrowserAction::GoForward
                )
        })
        .map(|command| command.command.clone())
        .collect();
    assert_eq!(steps, [BrowserAction::GoBack, BrowserAction::GoForward]);
    assert_eq!(
        engine
            .open(site, "web", SchoolWebEntry::Resume)
            .await
            .unwrap()
            .url,
        external
    );

    engine.shutdown().await;
}

async fn kyomu_open() -> (
    Arc<SchoolBrowserEngine>,
    Arc<FakeBrowser>,
    Arc<Events>,
    BrowserId,
) {
    let (engine, browser, events, _) = engine();
    engine
        .open(
            SchoolSite::Kyomu,
            "web",
            SchoolWebEntry::Go {
                url: KYOMU_HOME.into(),
            },
        )
        .await
        .unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    browser.listener().page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 2,
        },
        cause: None,
        url: Url::parse(KYOMU_HOME).unwrap(),
    });
    url_shown(&events, KYOMU_HOME).await;
    (engine, browser, events, id)
}

const KYOMU_HOME: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";

/// How many popups the page on screen had, each time that changed.
fn popups_open(events: &Events) -> Vec<usize> {
    let mut open: Vec<usize> = Vec::new();
    for page in pages(events) {
        if open.last() != Some(&page.popups) {
            open.push(page.popups);
        }
    }
    open
}

fn urls(events: &Events) -> Vec<String> {
    pages(events)
        .into_iter()
        .filter_map(|page| page.url)
        .collect()
}

// 教務Web's showDialog opens a named window whose page posts back through
// `opener` and closes itself: the popup must be a real child of its opener,
// front of the toolbar while open, and gone with everything it opened.
#[tokio::test]
async fn a_popup_is_a_child_of_its_opener_and_closes_with_what_it_opened() {
    let (engine, browser, events, id) = kyomu_open().await;
    let site = SchoolSite::Kyomu;
    let listener = browser.listener();
    let opener = PageId {
        browser: id,
        sequence: 2,
    };
    let picker = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/picker.aspx";
    let NewWindowDecision::Popup(popup) = listener.new_window(window(opener, picker)) else {
        panic!("the picker did not get a window");
    };
    assert_eq!(popup.site, site);
    assert_ne!(popup, id);
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: popup,
            sequence: 1,
        },
        cause: None,
        url: Url::parse(picker).unwrap(),
    });
    url_shown(&events, picker).await;
    // A late report of an older page, and the opener's own postback behind
    // the popup, leave the address on the popup.
    for (sequence, path) in [(1, "stale.aspx"), (3, "postback.aspx")] {
        listener.page_loaded(PageLoaded {
            page: PageId {
                browser: id,
                sequence,
            },
            cause: None,
            url: Url::parse(&format!(
                "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/{path}"
            ))
            .unwrap(),
        });
    }
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(urls(&events).last().map(String::as_str), Some(picker));
    // A popup opened from the popup nests under it; the toolbar acts on it.
    let page = PageId {
        browser: popup,
        sequence: 1,
    };
    let NewWindowDecision::Popup(nested) = listener.new_window(window(page, "about:blank")) else {
        panic!("window.open('') did not get a window");
    };
    assert_eq!(popups_open(&events), [0, 1, 2]);
    assert!(engine.reload(site).unwrap());
    assert!(browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .any(|command| command.browser == nested && command.command == BrowserAction::Reload));
    // window.close() in the picker closes it and the window it opened.
    listener.closed(popup);
    assert_eq!(*browser.disposed.lock().unwrap(), [nested]);
    assert_eq!(popups_open(&events), [0, 1, 2, 0]);
    assert!(urls(&events).last().unwrap().ends_with("postback.aspx"));
    assert_eq!(
        listener.decide_navigation(request(page, picker, true, NavigationKind::Link, true)),
        NavigationDecision::Deny(DenyReason::Closed)
    );
    // A page cannot open windows without bound; the app's close button
    // closes the frontmost.
    let mut opened = Vec::new();
    while let NewWindowDecision::Popup(popup) = listener.new_window(window(opener, picker)) {
        opened.push(popup);
    }
    assert_eq!(opened.len(), 8);
    assert_eq!(
        listener.new_window(window(opener, picker)),
        NewWindowDecision::Deny(DenyReason::TooManyPopups)
    );
    assert!(engine.close_popup(site).unwrap());
    assert_eq!(browser.disposed.lock().unwrap().last(), opened.last());
    // Closing the view takes its popups with it.
    listener.closed(id);
    assert!(!engine.close_popup(site).unwrap());
    assert_eq!(
        listener.new_window(window(opener, picker)),
        NewWindowDecision::Deny(DenyReason::Closed)
    );
    engine.shutdown().await;
}

// An LMS file link opens a popup whose only load becomes a download. A file
// that is saved leaves the popup empty forever, so Core closes it; a file
// shown in place, or a popup that already showed a page, stays.
#[tokio::test]
async fn a_popup_left_empty_by_a_saved_download_closes() {
    let (engine, browser, events, id) = kyomu_open().await;
    let listener = browser.listener();
    let opener = PageId {
        browser: id,
        sequence: 2,
    };
    let file = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/file.aspx";
    let popup = || match listener.new_window(window(opener, file)) {
        NewWindowDecision::Popup(popup) => popup,
        other => panic!("no popup: {other:?}"),
    };
    let became = |popup: BrowserId, had_page: bool, disposition: DownloadDisposition| {
        listener.became_download(BecameDownload {
            page: PageId {
                browser: popup,
                sequence: u64::from(had_page),
            },
            had_page,
            disposition,
        })
    };
    let empty_saved = popup();
    became(empty_saved, false, DownloadDisposition::Saved);
    assert_eq!(*browser.disposed.lock().unwrap(), [empty_saved]);
    assert_eq!(popups_open(&events), [0, 1, 0]);
    assert_eq!(
        listener.decide_navigation(request(
            PageId {
                browser: empty_saved,
                sequence: 0,
            },
            file,
            true,
            NavigationKind::Other,
            false,
        )),
        NavigationDecision::Deny(DenyReason::Closed)
    );

    let empty_shown = popup();
    became(empty_shown, false, DownloadDisposition::Shown);
    let with_page = popup();
    became(with_page, true, DownloadDisposition::Saved);
    // The opener's own downloads never close it.
    became(id, false, DownloadDisposition::Saved);
    assert_eq!(*browser.disposed.lock().unwrap(), [empty_saved]);
    assert_eq!(popups_open(&events), [0, 1, 0, 1, 2]);
    assert!(engine.close_popup(SchoolSite::Kyomu).unwrap());
    assert!(engine.close_popup(SchoolSite::Kyomu).unwrap());
    engine.shutdown().await;
}

// Moodle opens a submitted file in a new window (`target=_blank`), and the
// file is shown there with no history behind it. The window covers the LMS
// view on every surface that shows it, so Back has to close it and an app
// link has to land in the view itself. A surface coming back finds the view
// as it left it, window included, unless another surface showed it meanwhile.
#[tokio::test]
async fn a_file_window_gives_way_to_back_and_to_app_links() {
    let (engine, browser, events, _) = engine();
    let site = SchoolSite::ScienceLms;
    let reader = "reader-1";
    let activity = "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=131802";
    engine
        .open(
            site,
            reader,
            SchoolWebEntry::Go {
                url: activity.into(),
            },
        )
        .await
        .unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    let listener = browser.listener();
    let opener = PageId {
        browser: id,
        sequence: 1,
    };
    listener.page_loaded(PageLoaded {
        page: opener,
        cause: None,
        url: Url::parse(activity).unwrap(),
    });
    url_shown(&events, activity).await;
    let steps = || {
        browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| {
                matches!(
                    command.command,
                    BrowserAction::GoBack | BrowserAction::GoForward
                )
            })
            .count()
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
    // A host that reports history: the view's first page has nowhere to go,
    // and the app sends nothing.
    listener.history_changed(id, History::default());
    let page = engine.page(site);
    assert_eq!(
        (page.back, page.forward),
        (SchoolWebStep::No, SchoolWebStep::No)
    );
    assert!(!engine.back(site).unwrap());
    assert!(!engine.forward(site).unwrap());
    assert_eq!(steps(), 0);

    let file = "https://lms.s.isct.ac.jp/2025/pluginfile.php/9/assignsubmission_file/submission_files/4/report.pdf?forcedownload=1";
    let open_file = || {
        let NewWindowDecision::Popup(popup) = listener.new_window(window(opener, file)) else {
            panic!("the file link did not get a window");
        };
        listener.history_changed(popup, History::default());
        listener.became_download(BecameDownload {
            page: PageId {
                browser: popup,
                sequence: 0,
            },
            had_page: false,
            disposition: DownloadDisposition::Shown,
        });
        listener.page_loaded(PageLoaded {
            page: PageId {
                browser: popup,
                sequence: 1,
            },
            cause: None,
            url: Url::parse(file).unwrap(),
        });
        popup
    };
    let popup = open_file();
    url_shown(&events, file).await;
    let page = engine.page(site);
    assert_eq!(
        (page.popups, page.back, page.forward),
        (1, SchoolWebStep::Yes, SchoolWebStep::No),
        "Back closes the file window"
    );
    assert!(engine.back(site).unwrap());
    assert_eq!(steps(), 0);
    assert_eq!(*browser.disposed.lock().unwrap(), [popup]);
    let page = pages(&events).pop().unwrap();
    assert_eq!(
        (page.url.as_deref(), page.popups),
        (Some(activity), 0),
        "the activity is in front again"
    );

    // Back on a popup with history steps through it instead.
    let popup = open_file();
    listener.history_changed(
        popup,
        History {
            back: true,
            forward: false,
        },
    );
    assert!(engine.back(site).unwrap());
    assert_eq!(
        browser.commands.lock().unwrap().last().unwrap().browser,
        popup
    );
    assert_eq!(browser.disposed.lock().unwrap().len(), 1);

    // The reader coming back keeps the window, even though the view under
    // it has moved on (a confirmation covered the reader, or the person
    // looked at Today).
    let elsewhere = "https://lms.s.isct.ac.jp/2025/my/";
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 2,
        },
        cause: None,
        url: Url::parse(elsewhere).unwrap(),
    });
    browser.set_current_url(id, Url::parse(elsewhere).unwrap());
    engine.user_browsing(None).unwrap();
    let before = navigations();
    engine
        .open(
            site,
            reader,
            SchoolWebEntry::Return {
                url: activity.into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(engine.page(site).popups, 1);
    assert_eq!(navigations(), before);

    // Web showed the view in between, as it was; the reader then goes back
    // to its own activity.
    engine.user_browsing(None).unwrap();
    engine
        .open(site, "web", SchoolWebEntry::Resume)
        .await
        .unwrap();
    assert_eq!(engine.page(site).popups, 1);
    engine.user_browsing(None).unwrap();
    engine
        .open(
            site,
            reader,
            SchoolWebEntry::Return {
                url: activity.into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(engine.page(site).popups, 0);
    assert_eq!(browser.disposed.lock().unwrap().last(), Some(&popup));
    assert_eq!(
        browser.commands.lock().unwrap().last().unwrap().command,
        BrowserAction::Navigate(Url::parse(activity).unwrap())
    );

    // An app link, even to the page already under the window, closes it.
    let popup = open_file();
    let before = navigations();
    engine
        .open(
            site,
            "web",
            SchoolWebEntry::Go {
                url: activity.into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(engine.page(site).popups, 0);
    assert_eq!(browser.disposed.lock().unwrap().last(), Some(&popup));
    assert_eq!(navigations(), before, "the view was already there");
    engine.shutdown().await;
}

// R2: a person's new POST always goes through; only the engine sending an
// old form again is stopped. 教務Web pages, paging included, are postbacks.
#[tokio::test]
async fn a_form_the_engine_would_send_again_is_never_resubmitted() {
    let (engine, browser, events, id) = kyomu_open().await;
    let listener = browser.listener();
    let page = PageId {
        browser: id,
        sequence: 2,
    };
    let result = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/result.aspx";
    let post = |kind: NavigationKind, main_frame: bool| {
        listener.decide_navigation(NavigationRequest {
            method: Some("POST".into()),
            ..request(page, &format!("{result}#top"), main_frame, kind, false)
        })
    };
    assert_eq!(
        post(NavigationKind::FormSubmit, true),
        NavigationDecision::Allow
    );
    // Reloading the postback result (the app's Reload, or the picker's
    // `opener.location.reload()`) opens the page again with GET.
    assert_eq!(
        post(
            NavigationKind::FormResubmit {
                from_history: false
            },
            true
        ),
        NavigationDecision::Deny(DenyReason::Resubmission)
    );
    assert_eq!(
        browser.commands.lock().unwrap().last().unwrap().command,
        BrowserAction::Navigate(Url::parse(result).unwrap())
    );
    assert_eq!(
        notices(&events),
        [SchoolWebNotice::Reopened { url: result.into() }]
    );
    // Back or Forward onto it steps past the entry instead.
    assert_eq!(
        post(NavigationKind::FormResubmit { from_history: true }, true),
        NavigationDecision::SkipEntry
    );
    assert_eq!(
        post(NavigationKind::FormResubmit { from_history: true }, false),
        NavigationDecision::Deny(DenyReason::Resubmission)
    );
    // The app's Reload is the engine's, so the rule above applies to it.
    assert!(engine.reload(SchoolSite::Kyomu).unwrap());
    assert_eq!(
        browser.commands.lock().unwrap().last().unwrap().command,
        BrowserAction::Reload
    );
    engine.shutdown().await;

    // A host that cannot report resubmission gets a GET of the current URL.
    let plain = FakeBrowser::default();
    plain.plain_host.store(true, Ordering::Release);
    let (engine, browser, _, _) = engine_on(Arc::new(SchoolAuthService::new(EmptyVault)), plain);
    engine
        .open(
            SchoolSite::Kyomu,
            "web",
            SchoolWebEntry::Go {
                url: KYOMU_HOME.into(),
            },
        )
        .await
        .unwrap();
    let id = *browser.current.lock().unwrap().keys().next().unwrap();
    browser.set_current_url(id, Url::parse(result).unwrap());
    assert!(engine.reload(SchoolSite::Kyomu).unwrap());
    assert_eq!(
        browser.commands.lock().unwrap().last().unwrap().command,
        BrowserAction::Navigate(Url::parse(result).unwrap())
    );
    // Its popups are separate windows: the toolbar stays on the view.
    let opener = PageId {
        browser: id,
        sequence: 0,
    };
    let NewWindowDecision::Popup(_) = browser.listener().new_window(window(opener, result)) else {
        panic!("no window");
    };
    assert!(engine.back(SchoolSite::Kyomu).unwrap());
    assert_eq!(browser.commands.lock().unwrap().last().unwrap().browser, id);
    engine.shutdown().await;
}

// 「教育計算機システム」 off VPN: WebKit stays on the old page and says
// nothing, so the failure has to reach the app as a fact it can show.
#[tokio::test]
async fn a_failed_load_the_person_sees_becomes_a_notice() {
    let (engine, browser, events, id) = kyomu_open().await;
    let site = SchoolSite::Kyomu;
    let listener = browser.listener();
    let campus = "https://vlmgr-42.edu2024.gsic.titech.ac.jp/idm/upc/";
    let failed = |browser: BrowserId, sequence: u64, reason: LoadFailure| {
        listener.load_failed(LoadFailed {
            page: PageId { browser, sequence },
            cause: None,
            url: Some(Url::parse(campus).unwrap()),
            reason,
            message: "The request timed out.".into(),
        })
    };
    let notice = || engine.page(site).notice;
    let shown = |reason: LoadFailure| {
        Some(SchoolWebFailure {
            url: Some(campus.into()),
            reason,
        })
    };
    failed(id, 3, LoadFailure::Cancelled);
    assert_eq!(notice(), None);
    failed(id, 4, LoadFailure::Unreachable);
    assert_eq!(notice(), shown(LoadFailure::Unreachable));
    assert_eq!(
        pages(&events).last().unwrap().notice,
        shown(LoadFailure::Unreachable),
        "the surface on screen is told"
    );
    // Failing again replaces it; a late report of an older failure does not.
    failed(id, 5, LoadFailure::HostNotFound);
    failed(id, 4, LoadFailure::Offline);
    assert_eq!(notice(), shown(LoadFailure::HostNotFound));

    // It belongs to the page that failed: a popup over the view shows its
    // own, and the view's is in front again once the popup closes.
    let opener = PageId {
        browser: id,
        sequence: 2,
    };
    let NewWindowDecision::Popup(popup) = listener.new_window(window(opener, campus)) else {
        panic!("no popup");
    };
    assert_eq!(notice(), None);
    failed(popup, 1, LoadFailure::Offline);
    assert_eq!(notice(), shown(LoadFailure::Offline));
    listener.closed(popup);
    assert_eq!(notice(), shown(LoadFailure::HostNotFound));

    // Another surface showing the view finds it too.
    engine.user_browsing(None).unwrap();
    engine
        .open(site, "reader-1", SchoolWebEntry::Resume)
        .await
        .unwrap();
    assert_eq!(notice(), shown(LoadFailure::HostNotFound));

    // A later page of the view clears it.
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: id,
            sequence: 6,
        },
        cause: None,
        url: Url::parse(KYOMU_HOME).unwrap(),
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while notice().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the later page did not clear the failure");
    // So does Core sending the view somewhere (Retry), and the view closing.
    failed(id, 7, LoadFailure::Unreachable);
    assert!(engine.navigate(site, campus).unwrap());
    assert_eq!(notice(), None);
    failed(id, 8, LoadFailure::Offline);
    assert_eq!(notice(), shown(LoadFailure::Offline));
    listener.closed(id);
    assert_eq!(notice(), None);

    engine.shutdown().await;
}

#[tokio::test]
async fn a_closed_view_is_recreated_as_a_new_generation_next_time() {
    let (engine, browser, _, _) = engine();
    let target = "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx";
    assert!(engine.navigate(SchoolSite::Kyomu, target).unwrap());
    let first = browser.navigation(0).await;
    browser.listener().closed(first.browser);
    assert!(engine.navigate(SchoolSite::Kyomu, target).unwrap());
    let second = browser.navigation(1).await;
    assert_eq!(second.browser.site, first.browser.site);
    assert!(second.browser.generation > first.browser.generation);
    engine.shutdown().await;
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
    assert_eq!(engine.page(SchoolSite::Kyomu).url, None);
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

// ---------------------------------------------------------------------------
// Web operations (`engine/operation.rs`), over the scripted Science Tokyo of
// the session suite and a recording web cookie store.

const LMS_PAGE: &str = "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=7";
const SIGN_IN_PAGE: &str = "https://isct.ex-tic.com/auth/session";

/// A Science Tokyo account saved with `password` (the school accepts
/// "right"), its sessions over `school`, and an engine whose views write
/// into `store`.
fn signed_engine(
    school: &Arc<School>,
    password: &str,
    store: &Arc<WebStore>,
) -> (
    Arc<SchoolBrowserEngine>,
    Arc<FakeBrowser>,
    Arc<Events>,
    SchoolSessions,
) {
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    auth.stage(
        SchoolAuthProfile::Science,
        SchoolCredentials {
            username: "user".into(),
            password: password.into(),
            otp_secret: "JBSWY3DPEHPK3PXP".into(),
            matrix: String::new(),
        },
    )
    .unwrap();
    let sessions = SchoolSessions::with_transport(school.clone(), auth);
    assert!(sessions.attach_web(store.clone()));
    let browser = Arc::new(FakeBrowser::default());
    let events = Arc::new(Events::default());
    let engine =
        SchoolBrowserEngine::new(sessions.clone(), events.clone(), browser.clone()).unwrap();
    (engine, browser, events, sessions)
}

/// Waits (real time) until `done` holds.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        for attempt in 0.. {
            if done() {
                return;
            }
            settle(attempt).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

/// The page `command`'s navigation arrived at `url`, after the engine asked
/// to make it with a GET.
fn arrive(browser: &FakeBrowser, command: &BrowserCommand, sequence: u64, url: &str) {
    let page = PageId {
        browser: command.browser,
        sequence: sequence - 1,
    };
    let listener = browser.listener();
    listener.decide_navigation(request(page, url, true, NavigationKind::Other, false));
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: command.browser,
            sequence,
        },
        cause: Some(command.action),
        url: Url::parse(url).unwrap(),
    });
}

/// The person's own navigation of `view` arrived at `url` (no cause).
fn wander(browser: &FakeBrowser, view: BrowserId, sequence: u64, url: &str) {
    let listener = browser.listener();
    listener.decide_navigation(request(
        PageId {
            browser: view,
            sequence: sequence - 1,
        },
        url,
        true,
        NavigationKind::Link,
        true,
    ));
    listener.page_loaded(PageLoaded {
        page: PageId {
            browser: view,
            sequence,
        },
        cause: None,
        url: Url::parse(url).unwrap(),
    });
}

/// Navigation `index`, once dispatched. A sign-in may wait up to one
/// one-time-password window first (`OTP_MARGIN`).
async fn navigated(browser: &FakeBrowser, index: usize) -> BrowserCommand {
    let mut found = None;
    until("the navigation was dispatched", || {
        found = browser
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
            .nth(index)
            .cloned();
        found.is_some()
    })
    .await;
    found.unwrap()
}

fn navigations(browser: &FakeBrowser) -> usize {
    browser
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter(|command| matches!(command.command, BrowserAction::Navigate(_)))
        .count()
}

fn access(engine: &SchoolBrowserEngine) -> Option<SchoolWebAccess> {
    engine.page(SchoolSite::ScienceLms).access
}

fn target(command: &BrowserCommand) -> String {
    match &command.command {
        BrowserAction::Navigate(url) => url.to_string(),
        other => panic!("not a navigation: {other:?}"),
    }
}

// Only an App link (or a new view) starts an operation. It signs in once
// when there is no identity, writes the verified identity, and only then
// opens the target. Resuming the view, or its page moving to the sign-in
// page by itself, starts nothing: no write, no sign-in, no navigation.
#[tokio::test]
async fn an_app_link_writes_a_verified_identity_before_it_opens_and_nothing_else_starts_one() {
    let school = School::accepting("right");
    let store = WebStore::open(true);
    let (engine, browser, _, _) = signed_engine(&school, "right", &store);
    engine
        .open(
            SchoolSite::ScienceLms,
            "assignment",
            SchoolWebEntry::Go {
                url: LMS_PAGE.into(),
            },
        )
        .await
        .unwrap();
    let opened = navigated(&browser, 0).await;
    assert_eq!(target(&opened), LMS_PAGE);
    assert_eq!(school.passwords(), 1, "no identity yet: one sign-in");
    let writes = store.writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 1, "written before the target opened");
    assert!(writes[0]
        .iter()
        .any(|cookie| cookie.set_cookie.starts_with("_idp=t1")));
    arrive(&browser, &opened, 1, LMS_PAGE);
    until("the operation ended", || {
        access(&engine).is_none() && navigations(&browser) == 1
    })
    .await;

    engine
        .open(SchoolSite::ScienceLms, "assignment", SchoolWebEntry::Resume)
        .await
        .unwrap();
    // The page goes to the sign-in page on its own (the school ended the
    // session): Core does not watch it.
    wander(&browser, opened.browser, 2, SIGN_IN_PAGE);
    for attempt in 0..200 {
        settle(attempt).await;
    }
    assert_eq!(navigations(&browser), 1);
    assert_eq!(store.writes.lock().unwrap().len(), 1);
    assert_eq!(school.passwords(), 1);
    assert_eq!(access(&engine), None);
    engine.shutdown().await;
}

// The person's Reload on the sign-in page writes the verified identity and
// opens the visit's safe page again. When the page still refuses it, the
// operation signs in once, writes and opens again; refused again, it stops
// and says so. One password per operation, never more.
#[tokio::test]
async fn a_reload_on_the_sign_in_page_sends_the_password_at_most_once() {
    let school = School::accepting("right");
    let store = WebStore::open(true);
    let (engine, browser, _, _) = signed_engine(&school, "right", &store);
    engine
        .open(
            SchoolSite::ScienceLms,
            "assignment",
            SchoolWebEntry::Go {
                url: LMS_PAGE.into(),
            },
        )
        .await
        .unwrap();
    let opened = navigated(&browser, 0).await;
    arrive(&browser, &opened, 1, LMS_PAGE);
    until("the link's operation ended", || access(&engine).is_none()).await;
    // Later the session runs out and the person's own link lands on the
    // sign-in page.
    wander(&browser, opened.browser, 2, SIGN_IN_PAGE);
    until("the sign-in page recorded", || {
        engine.page(SchoolSite::ScienceLms).url.as_deref() == Some(SIGN_IN_PAGE)
    })
    .await;
    assert_eq!(school.passwords(), 1);

    assert!(engine.reload(SchoolSite::ScienceLms).unwrap());
    // Repeated Reloads join the operation.
    assert!(engine.reload(SchoolSite::ScienceLms).unwrap());
    let reopened = navigated(&browser, 1).await;
    assert_eq!(target(&reopened), LMS_PAGE, "the visit's safe page");
    assert_eq!(
        school.passwords(),
        1,
        "a fresh verified identity is used as it is"
    );
    assert_eq!(store.writes.lock().unwrap().len(), 2);
    assert_eq!(access(&engine), Some(SchoolWebAccess::SigningIn));
    arrive(&browser, &reopened, 3, SIGN_IN_PAGE);
    let again = navigated(&browser, 2).await;
    assert_eq!(
        school.passwords(),
        2,
        "the page refused it: the one sign-in"
    );
    assert_eq!(store.writes.lock().unwrap().len(), 3);
    arrive(&browser, &again, 4, SIGN_IN_PAGE);
    until("the operation stopped", || {
        access(&engine).is_some_and(|access| access != SchoolWebAccess::SigningIn)
    })
    .await;
    assert_eq!(
        access(&engine),
        Some(SchoolWebAccess::Problem {
            problem: SchoolWebProblem::StillSignIn
        })
    );
    assert_eq!(school.passwords(), 2);
    assert_eq!(navigations(&browser), 3);
    let covers = browser.covers.lock().unwrap().clone();
    assert_eq!(covers.first().map(|cover| cover.1), Some(true));
    assert_eq!(
        covers.last().map(|cover| cover.1),
        Some(false),
        "the view is given back"
    );
    // Another page clears what the operation said.
    wander(&browser, again.browser, 5, LMS_PAGE);
    until("the problem cleared", || access(&engine).is_none()).await;
    engine.shutdown().await;
}

// A refused password is never sent again by a web operation, not even on
// Reload; the person goes to Settings. A school that cannot be reached gets
// no password while the pause after the failure lasts, except through the
// person's Reload on the sign-in page.
#[tokio::test]
async fn a_refusal_and_an_unreachable_school_send_no_password() {
    let school = School::accepting("right");
    let store = WebStore::open(true);
    let (engine, browser, _, _) = signed_engine(&school, "wrong", &store);
    engine
        .open(
            SchoolSite::ScienceLms,
            "assignment",
            SchoolWebEntry::Go {
                url: LMS_PAGE.into(),
            },
        )
        .await
        .unwrap();
    let opened = navigated(&browser, 0).await;
    assert_eq!(school.passwords(), 1);
    assert!(
        store.writes.lock().unwrap().is_empty(),
        "nothing verified to write"
    );
    arrive(&browser, &opened, 1, SIGN_IN_PAGE);
    until("the refusal shown", || {
        access(&engine)
            == Some(SchoolWebAccess::Problem {
                problem: SchoolWebProblem::Rejected,
            })
    })
    .await;
    assert!(engine.reload(SchoolSite::ScienceLms).unwrap());
    until("the reload stopped", || {
        access(&engine)
            == Some(SchoolWebAccess::Problem {
                problem: SchoolWebProblem::Rejected,
            })
    })
    .await;
    assert_eq!(school.passwords(), 1, "a refusal holds through Reload");
    assert_eq!(
        navigations(&browser),
        1,
        "nothing to open without a sign-in"
    );
    engine.shutdown().await;

    let down = School::accepting("right");
    down.state.lock().unwrap().down = true;
    let store = WebStore::open(true);
    let (engine, browser, _, _) = signed_engine(&down, "right", &store);
    engine
        .open(
            SchoolSite::ScienceLms,
            "assignment",
            SchoolWebEntry::Go {
                url: LMS_PAGE.into(),
            },
        )
        .await
        .unwrap();
    let opened = navigated(&browser, 0).await;
    arrive(&browser, &opened, 1, SIGN_IN_PAGE);
    until("unreachable shown", || {
        access(&engine)
            == Some(SchoolWebAccess::Problem {
                problem: SchoolWebProblem::Unreachable,
            })
    })
    .await;
    // An App link during the pause asks the school nothing.
    let asked = down.requests();
    engine
        .open(
            SchoolSite::ScienceLms,
            "course",
            SchoolWebEntry::Go {
                url: "https://lms.s.isct.ac.jp/2025/course/view.php?id=3".into(),
            },
        )
        .await
        .unwrap();
    let linked = navigated(&browser, 1).await;
    assert_eq!(down.requests(), asked);
    arrive(&browser, &linked, 2, SIGN_IN_PAGE);
    until("unreachable again", || access(&engine).is_some()).await;
    // The person's Reload on the sign-in page goes through the pause.
    down.state.lock().unwrap().down = false;
    assert!(engine.reload(SchoolSite::ScienceLms).unwrap());
    let reopened = navigated(&browser, 2).await;
    assert_eq!(down.passwords(), 1);
    assert_eq!(store.writes.lock().unwrap().len(), 1);
    arrive(&browser, &reopened, 3, LMS_PAGE);
    until("the reload ended", || access(&engine).is_none()).await;
    assert_eq!(down.passwords(), 1);
    engine.shutdown().await;
}

// Signing an account out closes its views and clears its websites only,
// after the view stopped; another account's views stay. A platform that
// cannot clear one account's websites is not asked to: the account says its
// web sign-in was kept until the person chooses to clear every school site.
#[tokio::test]
async fn signing_an_account_out_closes_its_views_and_clears_its_websites_only() {
    for by_site in [true, false] {
        let school = School::accepting("right");
        let store = WebStore::open(by_site);
        let (engine, browser, _, sessions) = signed_engine(&school, "right", &store);
        engine
            .open(
                SchoolSite::ScienceLms,
                "assignment",
                SchoolWebEntry::Go {
                    url: LMS_PAGE.into(),
                },
            )
            .await
            .unwrap();
        let lms = navigated(&browser, 0).await.browser;
        engine
            .open(SchoolSite::Syllabus, "syllabus", SchoolWebEntry::Resume)
            .await
            .unwrap();
        sessions.sign_out(SchoolAuthProfile::Science).await;
        // The syllabus has no account: its view stays.
        assert_eq!(*browser.disposed.lock().unwrap(), [lms]);
        let clears = store.clears.lock().unwrap().clone();
        let kept = sessions.snapshot().profiles.science.web_kept;
        if by_site {
            assert_eq!(clears, [WebClear::account(SchoolAuthProfile::Science)]);
            assert!(!kept);
        } else {
            assert!(
                clears.is_empty(),
                "never a silent clear of every school site"
            );
            assert!(kept);
            sessions.clear_web().await;
            assert_eq!(*store.clears.lock().unwrap(), [WebClear::AllSchool]);
            assert!(!sessions.snapshot().profiles.science.web_kept);
        }
        engine.shutdown().await;
    }
}
