//! The walk into the campus VPN over a scripted portal proxy and APM: the
//! steps the real ones took when they were recorded, with their markup.
use super::*;
use crate::{
    net::{BoxFuture, NetRequest, Response},
    school_auth::{SchoolAuthService, SchoolCredentials},
    school_session::{tests::MemoryVault, Snapshot, Transport, WebCookie},
};
use std::sync::Arc;

const PASSWORD_FORM: &str = r#"<form method="post" action="/vpnaccess_apm/service/post;jsessionid=J1">
    <input type="password" id="password" name="password" autocomplete="current-password">
    <button type="submit">OK</button></form>"#;
const F5_ST: &str = "1793677622c900c100c1793677622c14400c600c";

/// The proxy and the APM as recorded: the APM admits browsers only and hands
/// over to the proxy through its policy page, the proxy knows the account
/// from the portal session and takes `password`, then posts back.
#[derive(Default)]
struct Campus {
    password: String,
    passwords: Mutex<usize>,
    /// The APM ends every session on its access policy.
    deny: bool,
}

/// The APM's policy page as it serves it: a form its script points at the
/// proxy and submits.
const POLICY_RELAY: &str = r#"<html><body>
    <FORM name=external_data_post_cls method=post action="">
        <input type=hidden name=client_data value="SecurityDevice">
        <input type=hidden name=post_url value="https://apm.nap.gsic.titech.ac.jp/my.policy">
    </FORM>
    <script>
    document.external_data_post_cls.action = unescape("https://rp.nap.gsic.titech.ac.jp/vpnaccess_apm/service/");
    document.external_data_post_cls.submit();
    </script></body></html>"#;
/// The proxy's way back after the password: a self-posting form to the APM.
const BACK_RELAY: &str = r#"<form name="back" method="post" action="https://apm.nap.gsic.titech.ac.jp/my.policy">
    <input type="hidden" name="token" value="t1"></form>"#;

impl Campus {
    fn answer(&self, request: &NetRequest) -> Response {
        let header = |name: reqwest::header::HeaderName| {
            request
                .headers
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let cookie = header(reqwest::header::COOKIE);
        let field = |name: &str| match &request.body {
            crate::net::Body::Form(fields) => fields
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone()),
            _ => None,
        };
        let url = request.url.clone();
        let post = request.method == reqwest::Method::POST;
        match (url.host_str().unwrap_or(""), post, url.path()) {
            // The access policy admits browsers only.
            ("apm.nap.gsic.titech.ac.jp", false, "/")
                if !header(reqwest::header::USER_AGENT).contains("Macintosh") =>
            {
                redirect(&url, "/vdesk/hangup.php3", None)
            }
            ("apm.nap.gsic.titech.ac.jp", false, "/") => redirect(
                &url,
                "/my.policy",
                Some("MRHSession=pending; path=/; secure"),
            ),
            ("apm.nap.gsic.titech.ac.jp", false, "/my.policy") => page(&url, POLICY_RELAY, None),
            ("apm.nap.gsic.titech.ac.jp", true, "/my.policy") if self.deny => {
                redirect(&url, "/my.logout.php3?errorcode=19", None)
            }
            ("apm.nap.gsic.titech.ac.jp", true, "/my.policy") if field("token").is_some() => {
                let mut answer = redirect(&url, "/vdesk/webtop.eui", None);
                for line in [
                    "MRHSession=s3cret; path=/; secure; HttpOnly".to_owned(),
                    format!("F5_ST={F5_ST}; path=/; secure"),
                ] {
                    answer
                        .headers
                        .append(reqwest::header::SET_COOKIE, line.parse().unwrap());
                }
                answer
            }
            ("apm.nap.gsic.titech.ac.jp", false, "/vdesk/webtop.eui") => {
                page(&url, "<p>webtop</p>", None)
            }
            // The proxy knows the account from the portal session.
            ("rp.nap.gsic.titech.ac.jp", true, "/vpnaccess_apm/service/")
                if cookie.contains("GA=portal")
                    && field("client_data").as_deref() == Some("SecurityDevice") =>
            {
                page(&url, PASSWORD_FORM, None)
            }
            ("rp.nap.gsic.titech.ac.jp", true, path)
                if path.starts_with("/vpnaccess_apm/service/post") =>
            {
                *self.passwords.lock().unwrap() += 1;
                if field("password").as_deref() == Some(self.password.as_str()) {
                    page(&url, BACK_RELAY, None)
                } else {
                    page(&url, PASSWORD_FORM, None)
                }
            }
            _ => page(&url, "", None),
        }
    }
}

impl Transport for Campus {
    fn send(&self, request: NetRequest) -> BoxFuture<'_, Result<Response, NetError>> {
        Box::pin(async move { Ok(self.answer(&request)) })
    }
    fn report_maintenance(&self, _host: Host) {}
    fn paused_until(&self, _host: Host) -> Option<i64> {
        None
    }
}

fn response(url: &Url, code: u16, cookie: Option<&str>) -> Response {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(cookie) = cookie {
        headers.append(reqwest::header::SET_COOKIE, cookie.parse().unwrap());
    }
    Response {
        status: code,
        url: url.clone(),
        headers,
        body: bytes::Bytes::new(),
    }
}

fn page(url: &Url, html: &str, cookie: Option<&str>) -> Response {
    let mut response = response(url, 200, cookie);
    response.headers.insert(
        reqwest::header::CONTENT_TYPE,
        "text/html; charset=utf-8".parse().unwrap(),
    );
    response.body = bytes::Bytes::from(html.to_owned());
    response
}

fn redirect(url: &Url, to: &str, cookie: Option<&str>) -> Response {
    let mut response = response(url, 302, cookie);
    response
        .headers
        .insert(reqwest::header::LOCATION, to.parse().unwrap());
    response
}

/// Sessions with the Tokyo Tech account saved and its portal identity
/// restored, so the walk starts from the portal session.
fn sessions(campus: Arc<Campus>) -> SchoolSessions {
    let auth = Arc::new(SchoolAuthService::new(MemoryVault::default()));
    auth.promote(
        SchoolAuthProfile::TokyoTech,
        &SchoolCredentials {
            username: "user".into(),
            password: "right".into(),
            otp_secret: String::new(),
            // GetAccess needs a full Matrix card; the walk never reads it.
            matrix: "A".repeat(70),
        },
    )
    .unwrap();
    let sessions = SchoolSessions::with_transport(campus, auth);
    sessions
        .0
        .account(SchoolAuthProfile::TokyoTech)
        .identity
        .restore(Snapshot {
            cookies: vec![WebCookie {
                url: "https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList".into(),
                set_cookie: "GA=portal; Domain=nap.gsic.titech.ac.jp; Path=/; Secure".into(),
            }],
        })
        .unwrap();
    sessions
}

/// From the portal session, through the APM's policy page and the proxy's
/// password page, back to the APM: the session cookies come back, and the
/// saved password was posted once. The scripted APM refuses any agent but a
/// browser's, as the real one does.
#[tokio::test]
async fn the_walk_signs_in_to_the_apm_with_the_saved_portal_password() {
    let campus = Arc::new(Campus {
        password: "right".into(),
        ..Campus::default()
    });
    let sessions = sessions(campus.clone());
    let login = sessions
        .walk_to_apm(SchoolAuthProfile::TokyoTech)
        .await
        .unwrap();
    assert_eq!(
        login.cookie.as_str(),
        format!("MRHSession=s3cret; F5_ST={F5_ST}")
    );
    assert_eq!(*campus.passwords.lock().unwrap(), 1);
}

/// The proxy showing its form again after the password is a refusal; the
/// walk must not post the password a second time.
#[tokio::test]
async fn a_refused_password_is_posted_once_and_reported() {
    let campus = Arc::new(Campus {
        password: "other".into(),
        ..Campus::default()
    });
    let sessions = sessions(campus.clone());
    assert_eq!(
        sessions
            .walk_to_apm(SchoolAuthProfile::TokyoTech)
            .await
            .unwrap_err(),
        CampusVpnError::Rejected
    );
    assert_eq!(*campus.passwords.lock().unwrap(), 1);
}

/// The APM's logout page after the policy is a denial, not a session.
#[tokio::test]
async fn the_apm_refusing_its_policy_is_a_denial() {
    let sessions = sessions(Arc::new(Campus {
        password: "right".into(),
        deny: true,
        ..Campus::default()
    }));
    assert_eq!(
        sessions
            .walk_to_apm(SchoolAuthProfile::TokyoTech)
            .await
            .unwrap_err(),
        CampusVpnError::Denied
    );
}

