use super::*;

#[test]
fn sign_in_pages_are_recognized_for_identity_providers_and_services() {
    for raw in [
        "https://isct.ex-tic.com/auth/session",
        "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?Template=userpass_key",
        "https://lms.s.isct.ac.jp/2025/login/index.php",
        "https://lms.s.isct.ac.jp/2025/auth/saml2/login.php",
    ] {
        assert!(is_login_url(&Url::parse(raw).unwrap()), "{raw}");
    }
    for raw in [
        "https://isct.ex-tic.com/idm/user/portal/",
        "https://lms.s.isct.ac.jp/2025/my/",
        "https://lms.s.isct.ac.jp/2025/pluginfile.php/1/mod_resource/content/1/a.pdf",
    ] {
        assert!(!is_login_url(&Url::parse(raw).unwrap()), "{raw}");
    }
    assert!(!is_identity_login(
        &Url::parse("https://lms.s.isct.ac.jp/2025/login/index.php").unwrap()
    ));
}

#[test]
fn school_urls_keep_only_bounded_domain_parameters() {
    assert_eq!(
        sanitize_school_url("https://syllabus.s.isct.ac.jp/courses?jwc=202602431&x=1").as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses?jwc=202602431")
    );
    assert!(sanitize_school_url("https://user:secret@lms.s.isct.ac.jp/2025/").is_none());
    assert!(sanitize_school_url("https://example.com/").is_none());
}
