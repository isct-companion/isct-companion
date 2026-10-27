use super::*;
use std::sync::Mutex as StdMutex;

struct FakePort {
    cookies: StdMutex<String>,
    stored: StdMutex<Vec<String>>,
}

impl FakePort {
    fn new(cookies: &str) -> Self {
        Self {
            cookies: StdMutex::new(cookies.into()),
            stored: StdMutex::new(Vec::new()),
        }
    }
}

impl SchoolCookiePort for FakePort {
    fn cookies(&self, _url: &Url) -> Result<Vec<(String, String)>, NetError> {
        let cookies = self
            .cookies
            .lock()
            .map_err(|_| NetError::Invalid("poisoned".into()))?;
        Ok(cookies
            .split("; ")
            .filter_map(|pair| pair.split_once('='))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect())
    }

    fn set_cookies(&self, _url: &Url, set_cookie: &[String]) -> Result<(), NetError> {
        self.stored
            .lock()
            .map_err(|_| NetError::Invalid("poisoned".into()))?
            .extend_from_slice(set_cookie);
        Ok(())
    }
}

#[test]
fn rejects_non_school_urls_before_calling_the_port() {
    let port = FakePort::new("unused=1");
    for raw in [
        "https://example.test/file.pdf",
        "http://lms.s.isct.ac.jp/2026/",
        "https://syllabus.s.isct.ac.jp/",
    ] {
        let url = Url::parse(raw).unwrap();
        assert!(matches!(
            cookie_header(&port, &url),
            Err(NetError::Invalid(_))
        ));
    }
}

/// Every school origin is read straight from the store. LMS and 教務Web need
/// their own session cookie, not just any cookie: without it the read asks
/// for a sign-in instead of sending a request the school would bounce.
#[test]
fn school_origins_need_their_own_session_cookie() {
    let lms = Url::parse("https://lms.s.isct.ac.jp/2026/pluginfile.php/1/file.pdf").unwrap();
    let kyomu =
        Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/PID1_0.aspx").unwrap();
    let portal = Url::parse("https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList").unwrap();
    let account = Url::parse("https://isct.ex-tic.com/idm/user/mfa_setting/").unwrap();

    let port = FakePort::new("TitechForm=x");
    assert_eq!(cookie_header(&port, &lms), Err(NetError::SignInRequired));
    assert_eq!(cookie_header(&port, &kyomu), Err(NetError::SignInRequired));
    assert_eq!(cookie_header(&port, &portal).unwrap(), "TitechForm=x");
    assert_eq!(cookie_header(&port, &account).unwrap(), "TitechForm=x");
    let signed_out = FakePort::new("");
    assert_eq!(
        cookie_header(&signed_out, &account),
        Err(NetError::SignInRequired)
    );
    let empty_session = FakePort::new("MoodleSession=");
    assert_eq!(
        cookie_header(&empty_session, &lms),
        Err(NetError::SignInRequired)
    );

    let port = FakePort::new("MoodleSession=secret; ASP.NET_SessionId=s");
    assert_eq!(
        cookie_header(&port, &lms).unwrap(),
        "MoodleSession=secret; ASP.NET_SessionId=s"
    );
    assert_eq!(
        cookie_header(&port, &kyomu).unwrap(),
        "MoodleSession=secret; ASP.NET_SessionId=s"
    );
}

/// A Rust response may only write the cookies a browser would accept from
/// it: host-only, or a Domain covering the host that names a whole site.
#[test]
fn set_cookie_write_back_keeps_only_cookies_for_the_response_host() {
    let port = FakePort::new("");
    let kyomu = Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx").unwrap();
    let values = [
        "KYOMU-http=1; path=/; HttpOnly",
        "a=1; Domain=.titech.ac.jp; Path=/",
        "b=1; Domain=gakumu.titech.ac.jp",
        "c=1; Domain=ac.jp",
        "d=1; Domain=isct.ac.jp",
        "=nameless; Path=/",
        "no-pair",
    ]
    .map(String::from);
    store_set_cookies(&port, &kyomu, &values).unwrap();
    assert_eq!(
        *port.stored.lock().unwrap(),
        [
            "KYOMU-http=1; path=/; HttpOnly",
            "a=1; Domain=.titech.ac.jp; Path=/",
            "b=1; Domain=gakumu.titech.ac.jp",
        ]
    );

    let public = Url::parse("https://syllabus.s.isct.ac.jp/").unwrap();
    store_set_cookies(&port, &public, &values).unwrap();
    assert_eq!(port.stored.lock().unwrap().len(), 3);
    let account = Url::parse("https://isct.ex-tic.com/idm/user/mfa_setting/").unwrap();
    store_set_cookies(
        &port,
        &account,
        &[
            "session=school; Path=/; Secure".into(),
            "shared=school; Domain=ex-tic.com".into(),
            "foreign=x; Domain=isct.ac.jp".into(),
        ],
    )
    .unwrap();
    assert_eq!(
        &port.stored.lock().unwrap()[3..],
        &["session=school; Path=/; Secure"]
    );
}
