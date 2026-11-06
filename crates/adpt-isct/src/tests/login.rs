use super::*;

const PROXY: &str = "https://rp.nap.gsic.titech.ac.jp/vpnaccess_apm/service/";

/// The proxy page as GetAccess serves it in front of the campus VPN: one
/// password field, no account field, the session id in the action.
#[test]
fn the_vpn_proxy_password_form_is_recognised_by_its_markup() {
    let html = r#"<html><body><form method="post" action="/vpnaccess_apm/service/post;jsessionid=ABC">
        <input type="password" id="password" name="password" autocomplete="current-password">
        <button type="submit">OK</button></form></body></html>"#;
    let VpnPage::Password(form) = vpn(html, PROXY) else {
        panic!("not read as the password step");
    };
    assert_eq!(
        form.action,
        "https://rp.nap.gsic.titech.ac.jp/vpnaccess_apm/service/post;jsessionid=ABC"
    );
    assert_eq!(form.get("password"), Some(""));
}

/// The GetAccess account form also has a password field; filling it with
/// only the password would send a sign-in without the account.
#[test]
fn the_portal_account_form_is_not_the_vpn_password_step() {
    let html = r#"<form name="login" method="post" action="/GetAccess/Login">
        <input name="usr_name"><input type="password" name="usr_password"></form>"#;
    assert_eq!(vpn(html, PROXY), VpnPage::Other);
}

/// A hand-off page posts hidden fields onward on load; a form with a visible
/// field is something for a person and is not submitted on their behalf.
#[test]
fn only_a_hidden_only_post_form_is_a_relay() {
    let relay = r#"<form method="POST" action="https://apm.nap.gsic.titech.ac.jp/my.policy">
        <input type="hidden" name="token" value="t1"></form>"#;
    let VpnPage::Relay(form) = vpn(relay, PROXY) else {
        panic!("not read as a relay");
    };
    assert_eq!(form.action, "https://apm.nap.gsic.titech.ac.jp/my.policy");
    assert_eq!(form.get("token"), Some("t1"));

    let visible = r#"<form method="post" action="/x"><input type="hidden" name="a" value="1">
        <input type="text" name="b"></form>"#;
    assert_eq!(vpn(visible, PROXY), VpnPage::Other);
}

/// The APM's own hand-off, as it serves it: an empty action that its script
/// replaces with the proxy's address before submitting.
#[test]
fn the_apm_hand_off_posts_to_the_address_its_script_sets() {
    let html = r#"<html><body>
        <FORM name=external_data_post_cls method=post action="">
            <input type=hidden name=client_data value="SecurityDevice">
            <input type=hidden name=post_url value="https://apm.nap.gsic.titech.ac.jp/my.policy">
        </FORM>
        <script>
        document.external_data_post_cls.action = unescape("https://rp.nap.gsic.titech.ac.jp/vpnaccess_apm/service/");
        document.external_data_post_cls.submit();
        </script></body></html>"#;
    let VpnPage::Relay(form) = vpn(html, "https://apm.nap.gsic.titech.ac.jp/my.policy") else {
        panic!("not read as a relay");
    };
    assert_eq!(form.action, PROXY);
    assert_eq!(form.get("client_data"), Some("SecurityDevice"));
    assert_eq!(
        form.get("post_url"),
        Some("https://apm.nap.gsic.titech.ac.jp/my.policy")
    );
}
