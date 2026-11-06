use super::*;

#[test]
fn only_a_redirect_to_getaccess_means_the_session_expired() {
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
    ] {
        assert_eq!(
            classify(status, Some(location)),
            Reply::SessionExpired,
            "{location}"
        );
    }
    // A redirect that stays on 教務Web (Default.aspx must never be followed),
    // the GetAccess menu, another host, a missing target or a server error is
    // not a sign-in signal: retrying the SSO would not change it.
    for (status, location) in [
        (302, Some("/Titech/Default.aspx")),
        (
            302,
            Some("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx"),
        ),
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
    let answer = page(&attempt, "grades").await;
    assert!(
        matches!(answer, Err(NetError::HostPaused { .. })),
        "{answer:?}"
    );
    assert!(notice(&sessions).is_some_and(|text| text.contains("メンテナンス中")));

    let (sessions, attempt) = answering("<html><body><p>Service Unavailable</p></body></html>");
    let answer = page(&attempt, "grades").await;
    assert!(matches!(answer, Err(NetError::Http(503))), "{answer:?}");
    assert_eq!(notice(&sessions), None);
}
