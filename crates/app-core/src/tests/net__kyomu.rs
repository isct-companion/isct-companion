//! The capture tests read 教務Web pages from `KYOMU_CAPTURES`, the directory
//! `crates/adpt-isct/src/tests/kyomu.rs` describes (`student-top.html` and
//! `<kind>.html` for each kind in `adpt_isct::kyomu::PAGES`), and from it
//! `maintenance.html`: the page 教務Web answers during a maintenance window,
//! which carries no account data, saved as the server sent it. They are
//! compiled only when `KYOMU_CAPTURES` names a directory (`kyomu_captures`,
//! `build.rs`); a page missing from it fails its test.
//!
//! `KYOMU_CAPTURES=/path/to/dir cargo test -p app-core --lib net::kyomu::tests`
use super::*;
#[cfg(kyomu_captures)]
use crate::net::Response;
#[cfg(kyomu_captures)]
use crate::school_auth::{SchoolAuthProfile::TokyoTech, SchoolAuthService};
#[cfg(kyomu_captures)]
use crate::school_session::{tests::NoVault, SchoolSessions, Transport};
#[cfg(kyomu_captures)]
use std::collections::HashMap;

#[cfg(kyomu_captures)]
fn offline_attempt(transport: Arc<dyn Transport>) -> (SchoolSessions, Attempt) {
    let sessions =
        SchoolSessions::with_transport(transport, Arc::new(SchoolAuthService::new(NoVault)));
    sessions.assume_configured(TokyoTech);
    let attempt = Attempt::detached(&sessions, Site::Kyomu, Default::default());
    (sessions, attempt)
}

#[cfg(kyomu_captures)]
fn capture(name: &str) -> String {
    let path = std::path::Path::new(env!("KYOMU_CAPTURES")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// GetAccess's sign-in and 教務Web's own entry both mean the session is
/// gone, so the run rebuilds it through the SSO instead of failing as a
/// refusal (1.3.1 reported `Default.aspx` as "refused (302)").
#[test]
fn a_redirect_to_getaccess_or_the_kyomu_entry_means_the_session_expired() {
    assert_eq!(classify(200, None), Reply::Page);
    for (status, location) in [
        (
            302,
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList",
        ),
        (
            302,
            "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList?x=1",
        ),
        (
            301,
            " https://PORTAL.nap.gsic.titech.ac.jp/getaccess/Login ",
        ),
        (302, "/Titech/Default.aspx"),
        (302, "/titech/default.aspx?ReturnUrl=%2fTitech%2fStudent%2f"),
        (
            302,
            "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
        ),
    ] {
        assert_eq!(
            classify(status, Some(location)),
            Reply::SessionExpired,
            "{location}"
        );
    }
    // Another page of 教務Web, the GetAccess menu, another host (with the
    // entry's path too), a missing target or a server error is not a session
    // signal: rebuilding the session would not change it.
    for (status, location) in [
        (302, Some("/Titech/Student/")),
        (302, Some("https://other.example/Titech/Default.aspx")),
        (302, Some("https://portal.nap.gsic.titech.ac.jp/portal.pl")),
        (302, Some("https://other.example/GetAccess/ResourceList")),
        (302, None),
        (
            500,
            Some("https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList"),
        ),
    ] {
        assert_eq!(classify(status, location), Reply::Unexpected(status));
    }
}

#[test]
fn a_capped_catalog_search_narrows_to_one_quarter() {
    let query = |quarter| CatalogSearch {
        code: "LA".into(),
        quarter,
        ..CatalogSearch::default()
    };
    let pairs = |quarter| -> Vec<(String, String)> {
        search_url(2026, &query(quarter))
            .unwrap()
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect()
    };
    let whole = pairs(0);
    assert!(whole.contains(&("ShinkokuNoS".into(), "LA".into())));
    assert!(!whole.iter().any(|(key, _)| key == "Quarters"));
    assert!(pairs(3).contains(&("Quarters".into(), "3".into())));
    // Out-of-range quarters search the whole year, never an invalid value.
    assert!(!pairs(5).iter().any(|(key, _)| key == "Quarters"));
}

// 教務Web's maintenance page may come with an error status. It is the site's
// maintenance all the same: the host pauses with the notice. A real page of
// the site answered with an error status stays the HTTP failure it is.
#[tokio::test]
#[cfg(kyomu_captures)]
async fn a_captured_maintenance_page_with_an_error_status_pauses_the_host() {
    use crate::net::one_answer::OneAnswer;
    let answering = |name: &str| {
        // `OneAnswer` serves a body that lives as long as the test.
        let body: &'static str = Box::leak(capture(name).into_boxed_str());
        offline_attempt(Arc::new(OneAnswer {
            status: 503,
            content_type: "text/html; charset=utf-8",
            body,
        }))
    };
    let notice = |sessions: &SchoolSessions| {
        sessions
            .snapshot()
            .profile(TokyoTech)
            .maintenance
            .clone()
            .and_then(|maintenance| maintenance.notice)
    };

    let (sessions, attempt) = answering("maintenance.html");
    let answer = page(&attempt, &MenuCache::default(), "grades").await;
    assert!(
        matches!(answer, Err(NetError::HostPaused { .. })),
        "{answer:?}"
    );
    assert!(notice(&sessions).is_some_and(|text| !text.trim().is_empty()));

    let (sessions, attempt) = answering("student-top.html");
    let answer = page(&attempt, &MenuCache::default(), "grades").await;
    assert!(matches!(answer, Err(NetError::Http(503))), "{answer:?}");
    assert_eq!(notice(&sessions), None);
}

/// Each path answered with the page captured for it; any other with 404.
#[cfg(kyomu_captures)]
struct Captured(HashMap<String, String>);

#[cfg(kyomu_captures)]
impl Transport for Captured {
    fn send(&self, request: NetRequest) -> crate::net::BoxFuture<'_, Result<Response, NetError>> {
        let (status, body) = match self.0.get(request.url.path()) {
            Some(html) => (200, html.clone()),
            None => (404, String::new()),
        };
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("text/html; charset=utf-8"),
        );
        let response = Response {
            status,
            url: request.url,
            headers,
            body: body.into_bytes().into(),
        };
        Box::pin(async move { Ok(response) })
    }
    fn report_maintenance(&self, _host: Host) {}
    fn paused_until(&self, _host: Host) -> Option<i64> {
        None
    }
}

/// Issue #64: every page is read at the address the account's own student
/// menu links (a 修士 account's 成績一覧 is `PID2_2`, not the 学士 `PID2_0`
/// 教務Web sends it away from), and what it answers passes the reader and the
/// trust boundary as that page. An address the menu does not link answers
/// 404 here, so a guessed one fails the read.
#[tokio::test]
#[cfg(kyomu_captures)]
async fn each_captured_page_is_read_at_the_address_its_student_menu_links() {
    let top = capture("student-top.html");
    let menu = kyomu::Menu::read(kyomu::STUDENT_TOP, &top).unwrap();
    let mut pages = HashMap::from([(
        Url::parse(kyomu::STUDENT_TOP).unwrap().path().to_string(),
        top,
    )]);
    for listed in &kyomu::PAGES {
        let url = Url::parse(&menu.url(listed.kind).unwrap()).unwrap();
        pages.insert(
            url.path().to_string(),
            capture(&format!("{}.html", listed.kind)),
        );
    }
    let (_sessions, attempt) = offline_attempt(Arc::new(Captured(pages)));
    let cache = MenuCache::default();
    for kind in kyomu::PAGES.iter().map(|listed| listed.kind) {
        let payload = page(&attempt, &cache, kind)
            .await
            .unwrap_or_else(|error| panic!("{kind}: {error:?}"));
        assert_eq!(payload.kind, kind);
    }
}
