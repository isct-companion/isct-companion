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
