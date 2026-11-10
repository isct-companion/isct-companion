use super::*;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, LOCATION, SET_COOKIE};

fn response(headers: &[(reqwest::header::HeaderName, &str)], body: &[u8]) -> Response {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.append(name.clone(), HeaderValue::from_str(value).unwrap());
    }
    Response {
        status: 200,
        url: Url::parse("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/PID1_0.aspx").unwrap(),
        headers: map,
        body: bytes::Bytes::copy_from_slice(body),
    }
}

/// 教務Web declares UTF-8 today; if it ever switches, text must follow the
/// declaration instead of producing mojibake, and an unknown one must fail.
#[test]
fn text_decodes_by_the_declared_charset() {
    let (shift_jis, _, _) = encoding_rs::SHIFT_JIS.encode("履修申告 教務Web");
    let page = response(
        &[(CONTENT_TYPE, "text/html; charset=Shift_JIS")],
        &shift_jis,
    );
    assert_eq!(page.text().unwrap(), "履修申告 教務Web");

    let quoted = response(
        &[(CONTENT_TYPE, "text/html; Charset=\"euc-jp\"")],
        &encoding_rs::EUC_JP.encode("成績").0,
    );
    assert_eq!(quoted.text().unwrap(), "成績");

    let undeclared = response(&[(CONTENT_TYPE, "text/html")], "時間割".as_bytes());
    assert_eq!(undeclared.text().unwrap(), "時間割");
    let bare = response(&[], "時間割".as_bytes());
    assert_eq!(bare.text().unwrap(), "時間割");

    let unknown = response(&[(CONTENT_TYPE, "text/html; charset=x-made-up")], b"x");
    assert!(matches!(unknown.text(), Err(NetError::Invalid(_))));
}

/// The token redirect must reach the caller byte for byte: a URL parser
/// would read `moodlemobile://token=…` as a host and mangle it.
#[test]
fn location_and_set_cookie_values_pass_through_raw() {
    let token = "moodlemobile://token=YWJjZGVm:::MTIzNDU2Nzg5MA==:::cHJpdmF0ZQ==";
    let redirect = response(
        &[
            (LOCATION, token),
            (SET_COOKIE, "KYOMU-http=1; path=/; HttpOnly"),
            (SET_COOKIE, "MoodleSession=abc; path=/2025/; secure"),
        ],
        b"",
    );
    assert_eq!(redirect.location().as_deref(), Some(token));
    assert_eq!(
        redirect.set_cookies(),
        [
            "KYOMU-http=1; path=/; HttpOnly",
            "MoodleSession=abc; path=/2025/; secure"
        ]
    );
    assert_eq!(response(&[], b"").location(), None);
}

/// A request is only sent to its own host over https, so one host's cookies
/// can never go to another; a paused host fails without sending.
#[tokio::test]
async fn send_refuses_foreign_urls_and_paused_hosts_before_the_network() {
    let http = Http::new().unwrap();
    for raw in [
        "https://lms.s.isct.ac.jp/2025/my/",
        "http://kyomu0.gakumu.titech.ac.jp/Titech/",
        "https://kyomu0.gakumu.titech.ac.jp:8443/Titech/",
        "https://user@kyomu0.gakumu.titech.ac.jp/Titech/",
    ] {
        let request = NetRequest::get(Host::Kyomu, Url::parse(raw).unwrap());
        assert!(
            matches!(http.send(request).await, Err(NetError::Invalid(_))),
            "{raw}"
        );
    }

    http.report_maintenance(Host::Moodle);
    let request = NetRequest::get(
        Host::Moodle,
        Url::parse("https://lms.s.isct.ac.jp/2025/my/").unwrap(),
    );
    assert!(matches!(
        http.send(request).await,
        Err(NetError::HostPaused {
            host: Host::Moodle,
            ..
        })
    ));
}
