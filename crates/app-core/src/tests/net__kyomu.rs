use super::*;

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
// maintenance all the same: the host pauses with the notice, on a page read
// and a course search alike. Another error page stays the HTTP failure it is.
#[tokio::test]
async fn a_maintenance_page_with_an_error_status_pauses_the_host() {
    use crate::net::one_answer::OneAnswer;
    use crate::school_auth::{SchoolAuthProfile::TokyoTech, SchoolAuthService};
    use crate::school_session::SchoolSessions;
    use std::sync::Arc;

    struct NoVault;
    impl crate::ports::SecretVault for NoVault {
        fn store(&self, _: &str, _: &str) -> Result<(), crate::ports::SecretError> {
            Ok(())
        }
        fn load(&self, _: &str) -> Result<Option<String>, crate::ports::SecretError> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> Result<(), crate::ports::SecretError> {
            Ok(())
        }
    }
    let answering = |body| {
        let sessions = SchoolSessions::with_transport(
            Arc::new(OneAnswer {
                status: 503,
                content_type: "text/html; charset=utf-8",
                body,
            }),
            Arc::new(SchoolAuthService::new(NoVault)),
        );
        sessions.assume_configured(TokyoTech);
        let attempt = Attempt::detached(&sessions, Site::Kyomu, Default::default());
        (sessions, attempt)
    };
    let notice = |sessions: &SchoolSessions| {
        sessions
            .snapshot()
            .profile(TokyoTech)
            .maintenance
            .clone()
            .and_then(|maintenance| maintenance.notice)
    };

    const PAGE: &str = "<html><body><main><h1>ただいまメンテナンス中です</h1></main></body></html>";
    let (sessions, attempt) = answering(PAGE);
    let answer = page(&attempt, &MenuCache::default(), "grades").await;
    assert!(
        matches!(answer, Err(NetError::HostPaused { .. })),
        "{answer:?}"
    );
    assert!(notice(&sessions).is_some_and(|text| text.contains("メンテナンス中")));

    let (sessions, attempt) = answering("<html><body><p>Service Unavailable</p></body></html>");
    let answer = page(&attempt, &MenuCache::default(), "grades").await;
    assert!(matches!(answer, Err(NetError::Http(503))), "{answer:?}");
    assert_eq!(notice(&sessions), None);
}

/// Issue #64: a 修士 account reads its grades at the address its student
/// menu links (`PID2_2`), not the 学士 one 教務Web redirects it away from,
/// and the menu is read once per site instance.
#[tokio::test]
async fn a_page_is_read_at_the_address_the_student_menu_links() {
    use crate::net::Response;
    use crate::school_auth::{SchoolAuthProfile::TokyoTech, SchoolAuthService};
    use crate::school_session::{SchoolSessions, Transport};
    use std::sync::{Arc, Mutex};

    /// The 修士 student menu for every request, keeping each path asked.
    #[derive(Default)]
    struct MasterMenu(Mutex<Vec<String>>);
    impl Transport for MasterMenu {
        fn send(
            &self,
            request: NetRequest,
        ) -> crate::net::BoxFuture<'_, Result<Response, NetError>> {
            self.0.lock().unwrap().push(request.url.path().to_string());
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("text/html; charset=utf-8"),
            );
            let body = r#"<html><body><div id="ctl00_sideMenu1_pnlStudent">
                <a id="ctl00_sideMenu1_HyperLink8" href="%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_2.aspx">成績一覧</a>
                </div></body></html>"#;
            let response = Response {
                status: 200,
                url: request.url,
                headers,
                body: body.as_bytes().to_vec().into(),
            };
            Box::pin(async move { Ok(response) })
        }
        fn report_maintenance(&self, _host: crate::net::Host) {}
        fn paused_until(&self, _host: crate::net::Host) -> Option<i64> {
            None
        }
    }
    struct NoVault;
    impl crate::ports::SecretVault for NoVault {
        fn store(&self, _: &str, _: &str) -> Result<(), crate::ports::SecretError> {
            Ok(())
        }
        fn load(&self, _: &str) -> Result<Option<String>, crate::ports::SecretError> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> Result<(), crate::ports::SecretError> {
            Ok(())
        }
    }
    let transport = Arc::new(MasterMenu::default());
    let sessions = SchoolSessions::with_transport(
        transport.clone(),
        Arc::new(SchoolAuthService::new(NoVault)),
    );
    sessions.assume_configured(TokyoTech);
    let attempt = Attempt::detached(&sessions, Site::Kyomu, Default::default());
    let cache = MenuCache::default();
    // The answer to the page itself is the menu again, which no grade
    // reader accepts: only the address asked matters here.
    for _ in 0..2 {
        let answer = page(&attempt, &cache, "grades").await;
        assert!(matches!(answer, Err(NetError::Invalid(_))), "{answer:?}");
    }
    let grades = "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_2.aspx";
    assert_eq!(
        *transport.0.lock().unwrap(),
        ["/Titech/Student/", grades, grades]
    );
    // The menu links no 科目から page: an error, never another address.
    let answer = page(&attempt, &cache, "declared").await;
    assert!(
        matches!(&answer, Err(NetError::Invalid(reason)) if reason.contains("declared")),
        "{answer:?}"
    );
    assert_eq!(transport.0.lock().unwrap().len(), 3);
}
