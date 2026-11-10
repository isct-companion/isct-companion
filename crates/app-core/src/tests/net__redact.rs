use super::*;

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
