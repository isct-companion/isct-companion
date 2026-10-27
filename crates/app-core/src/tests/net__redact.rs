use super::*;

#[test]
fn urls_lose_credential_query_values_and_keep_everything_else() {
    assert_eq!(
        redact_url(
            "https://lms.s.isct.ac.jp/2025/lib/ajax/service.php?sesskey=AbC123&info=core_course_get_enrolled"
        ),
        "https://lms.s.isct.ac.jp/2025/lib/ajax/service.php?sesskey=[redacted]&info=core_course_get_enrolled"
    );
    assert_eq!(
        redact_url(
            "https://lms.s.isct.ac.jp/2025/webservice/pluginfile.php/1/a.pdf?TOKEN=t1&forcedownload=1&wstoken=t2&privatetoken=t3#passport=p"
        ),
        "https://lms.s.isct.ac.jp/2025/webservice/pluginfile.php/1/a.pdf?TOKEN=[redacted]&forcedownload=1&wstoken=[redacted]&privatetoken=[redacted]#passport=[redacted]"
    );
    assert_eq!(
        redact_url(
            "https://lms.s.isct.ac.jp/2025/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport=8f1c&urlscheme=moodlemobile"
        ),
        "https://lms.s.isct.ac.jp/2025/admin/tool/mobile/launch.php?service=moodle_mobile_app&passport=[redacted]&urlscheme=moodlemobile"
    );
    let plain = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/PID1_0.aspx";
    assert_eq!(redact_url(plain), plain);
    // `tokens` and `mytoken` are not credential names.
    let near = "https://syllabus.s.isct.ac.jp/courses?tokens=1&mytoken=2";
    assert_eq!(redact_url(near), near);
}

#[test]
fn moodle_mobile_redirects_lose_the_whole_token() {
    for raw in [
        "moodlemobile://token=YWJjOjo6ZGVmOjo6Z2hp",
        "MoodleMobile://token=YWJj:::ZGVm",
        "moodlemobile://",
    ] {
        let redacted = redact_url(raw);
        assert!(redacted.ends_with("://token=[redacted]"), "{redacted}");
        assert!(!redacted.contains("YWJj"), "{redacted}");
    }
}

#[test]
fn text_loses_tokens_sesskeys_passports_and_cookies() {
    let text = concat!(
        r#"M.cfg = {"sesskey":"AbC123","wwwroot":"https://lms.s.isct.ac.jp/2025"}; "#,
        "POST wstoken=w1&wsfunction=core_webservice_get_site_info ",
        "launch passport=p1 ",
        "Location: moodlemobile://token=Zm9vOjo6YmFy\n",
        "Cookie: MoodleSession=m1; ASP.NET_SessionId=s1\n",
        "Set-Cookie: KYOMU-http=k1; path=/\n",
        "privatetoken: v1 Authorization: Bearer b1",
    );
    let redacted = redact_text(text);
    for secret in ["AbC123", "w1", "p1", "Zm9v", "m1", "s1", "k1", "v1", "b1"] {
        assert!(!redacted.contains(secret), "{secret} in {redacted}");
    }
    assert!(redacted.contains("wsfunction=core_webservice_get_site_info"));
    assert!(redacted.contains(r#""wwwroot":"https://lms.s.isct.ac.jp/2025""#));
    assert!(redacted.contains("moodlemobile://token=[redacted]"));
    assert!(redacted.contains("Cookie: [redacted]"));
}
