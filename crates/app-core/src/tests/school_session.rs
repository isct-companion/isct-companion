//! The session rules that need no school: the jar, the identity snapshot
//! and the account barrier. Sign-in flows run against the real schools
//! (`school_session__login.rs`, ignored by default).
use super::*;
use std::time::Duration;

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

// The barrier waits for every permit, and no permit is issued while it runs.
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
    assert!(!closing.is_finished(), "it waits for the permit");
    drop(permit);
    assert!(closing.await.unwrap());
    assert_eq!(owner.permit().unwrap().generation(), 2);
}
