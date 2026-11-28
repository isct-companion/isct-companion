//! The walk into the campus VPN against the real portal proxy and APM.
//! Ignored by default: it needs a Tokyo Tech account with campus VPN access
//! in the variables `school_session::login`'s live tests read
//! (`ISCT_TOKYO_ID`, `ISCT_TOKYO_PASSWORD`, `ISCT_TOKYO_MATRIX`), reaches
//! the school, and spends one sign-in and one APM session per run. It never
//! prints or stores what they hold.
//!
//! `cargo test -p app-core --features campus-vpn --lib school_session::vpn -- --ignored --nocapture`
use super::*;
use crate::{
    net::Http,
    school_auth::{SchoolAuthService, SchoolCredentials},
    school_session::{tests::MemoryVault, SchoolConnectOutcome},
};
use std::sync::Arc;

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

/// From a fresh sign-in, through the APM's policy page and the proxy's
/// password page, back to the APM holding the session cookie openconnect is
/// given.
#[tokio::test]
#[ignore = "needs a real Tokyo Tech account with campus VPN access in ISCT_TOKYO_*"]
async fn the_walk_signs_in_to_the_apm_with_the_saved_portal_password() {
    let profile = SchoolAuthProfile::TokyoTech;
    let typed = SchoolCredentials {
        username: required("ISCT_TOKYO_ID"),
        password: required("ISCT_TOKYO_PASSWORD"),
        otp_secret: String::new(),
        matrix: required("ISCT_TOKYO_MATRIX")
            .chars()
            .filter(char::is_ascii_alphabetic)
            .map(|letter| letter.to_ascii_uppercase())
            .collect(),
    };
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    let credentials = auth.resolve(profile, typed).unwrap();
    let sessions = SchoolSessions::new(Some(Http::new().unwrap()), auth);
    assert!(matches!(
        sessions.connect(profile, credentials).await.unwrap(),
        SchoolConnectOutcome::Connected { .. }
    ));
    let login = sessions
        .campus_vpn_login()
        .await
        .unwrap_or_else(|error| panic!("the walk did not reach the APM: {error}"));
    assert!(login.cookie.as_str().starts_with("MRHSession="));
}
