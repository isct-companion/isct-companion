//! Sign-in against the real schools. Ignored by default: they need real
//! credentials and reach the schools, and each run is one full sign-in, so
//! keep to a few a day per account. They read only these variables and never
//! print or store what they hold:
//!
//! - `ISCT_SCIENCE_ID`, `ISCT_SCIENCE_PASSWORD`, `ISCT_SCIENCE_OTP`
//!   (the `otpauth://` setup link or its base32 secret);
//! - `ISCT_TOKYO_ID`, `ISCT_TOKYO_PASSWORD`, `ISCT_TOKYO_MATRIX`
//!   (70 letters, spaces and newlines allowed).
//!
//! `cargo test -p app-core --lib school_session::login -- --ignored --nocapture`
use super::*;
use crate::{
    net::Http,
    ports::{SecretError, SecretVault},
    school_auth::{SchoolAuthProfile, SchoolAuthService},
    school_session::{Operation, SchoolSessions, Site},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn science_credentials() -> SchoolCredentials {
    SchoolCredentials {
        username: required("ISCT_SCIENCE_ID"),
        password: required("ISCT_SCIENCE_PASSWORD"),
        otp_secret: required("ISCT_SCIENCE_OTP"),
        matrix: String::new(),
    }
}

fn tokyo_credentials() -> SchoolCredentials {
    let matrix: String = required("ISCT_TOKYO_MATRIX")
        .chars()
        .filter(char::is_ascii_alphabetic)
        .map(|letter| letter.to_ascii_uppercase())
        .collect();
    SchoolCredentials {
        username: required("ISCT_TOKYO_ID"),
        password: required("ISCT_TOKYO_PASSWORD"),
        otp_secret: String::new(),
        matrix,
    }
}

#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

impl SecretVault for MemoryVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(account.into(), secret.into());
        Ok(())
    }
    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        Ok(self.0.lock().unwrap().get(account).cloned())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.0.lock().unwrap().remove(account);
        Ok(())
    }
}

fn install_logger() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target().starts_with("app_core::school_session") {
                eprintln!("{}", record.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

/// One page read on a site, as a read operation.
struct Page(&'static str);

impl Operation for Page {
    type Output = String;
    type Error = NetError;

    fn attempt(
        &mut self,
        attempt: crate::school_session::Attempt,
    ) -> crate::net::BoxFuture<'_, Result<String, NetError>> {
        Box::pin(async move {
            let url = Url::parse(self.0).map_err(|_| NetError::Invalid("url".into()))?;
            let host = Host::of(&url).ok_or_else(|| NetError::Invalid("host".into()))?;
            let response = attempt.send(crate::net::NetRequest::get(host, url)).await?;
            match response.status {
                200 => response.text(),
                300..=399 => Err(NetError::SignInRequired),
                status => Err(NetError::Http(status)),
            }
        })
    }
}

/// Connects the account the way Settings does and reads one page of its
/// site from a site session built from the identity snapshot: the whole
/// path, one sign-in.
async fn connect_and_read(
    profile: SchoolAuthProfile,
    credentials: SchoolCredentials,
    site: Site,
    page: &'static str,
) -> String {
    install_logger();
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    let credentials = auth.resolve(profile, credentials).unwrap();
    let sessions = SchoolSessions::new(Some(Http::new().unwrap()), auth.clone());
    let outcome = sessions.connect(profile, credentials.clone()).await;
    assert_eq!(
        outcome,
        crate::school_session::SchoolConnectOutcome::Connected
    );
    assert_eq!(
        auth.load(profile).unwrap(),
        Some(credentials),
        "the school's acceptance saves the credentials"
    );
    let permit = sessions.permit(profile).unwrap();
    let deadline = tokio::time::Instant::now() + crate::sync_engine::KEY_DEADLINE;
    sessions
        .run(site, &permit, deadline, &mut Page(page))
        .await
        .unwrap_or_else(|error| panic!("{site:?} did not read: {error:?}"))
}

#[tokio::test]
#[ignore = "needs a real Science Tokyo account in ISCT_SCIENCE_*"]
async fn science_signs_in_over_http_and_its_lms_session_reads_the_dashboard() {
    let html = connect_and_read(
        SchoolAuthProfile::Science,
        science_credentials(),
        Site::Lms,
        LMS_HOME,
    )
    .await;
    assert!(
        html.contains("login/logout.php"),
        "the LMS answered as a guest"
    );
}

#[tokio::test]
#[ignore = "needs a real Tokyo Tech account in ISCT_TOKYO_*"]
async fn tokyo_tech_signs_in_over_http_and_its_kyomu_session_reads_the_student_top() {
    let html = connect_and_read(
        SchoolAuthProfile::TokyoTech,
        tokyo_credentials(),
        Site::Kyomu,
        "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/",
    )
    .await;
    assert!(
        !html.contains("Template=userpass_key"),
        "教務Web showed the agreement"
    );
}

#[tokio::test]
#[ignore = "submits one deliberately wrong password; needs ISCT_SCIENCE_*"]
async fn science_refuses_a_wrong_password_without_a_second_attempt() {
    install_logger();
    let mut credentials = science_credentials();
    credentials.password.push('x');
    let submitted = std::sync::atomic::AtomicBool::new(false);
    let end = science(&Http::new().unwrap(), &credentials, &submitted).await;
    assert!(
        matches!(end, LoginEnd::Rejected(Kind::InvalidCredentials, _)),
        "{end:?}"
    );
}

#[tokio::test]
#[ignore = "submits one deliberately wrong Matrix answer; needs ISCT_TOKYO_*"]
async fn tokyo_tech_refuses_a_wrong_matrix_without_a_second_attempt() {
    install_logger();
    let mut credentials = tokyo_credentials();
    // Every letter wrong, whichever cells are asked.
    credentials.matrix = credentials
        .matrix
        .chars()
        .map(|letter| if letter == 'A' { 'B' } else { 'A' })
        .collect();
    let submitted = std::sync::atomic::AtomicBool::new(false);
    let end = tokyo(&Http::new().unwrap(), &credentials, &submitted).await;
    assert!(
        matches!(end, LoginEnd::Rejected(Kind::InvalidMatrix, _)),
        "{end:?}"
    );
}
