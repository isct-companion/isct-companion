//! The pages on the way into the campus VPN follow a sign-in, so none is
//! committed. This test reads pages saved off campus from a browser signed in
//! to the portal, from the directory in `VPN_CAPTURES`, kept outside the
//! repository, and is compiled only when that directory exists, and a file
//! missing from it fails its test
//! (`vpn_captures`, `build.rs`). Open <https://apm.nap.gsic.titech.ac.jp/>
//! from the portal's resource menu with the developer tools' network log kept
//! across pages, and save these responses' bodies:
//!
//! - `apm-handoff.html`: the APM's `GET https://apm.nap.gsic.titech.ac.jp/my.policy`,
//!   the form its script points at the proxy;
//! - `proxy-password.html`: the proxy's answer to that form's post to
//!   `https://rp.nap.gsic.titech.ac.jp/vpnaccess_apm/service/`, the
//!   password page;
//! - `getaccess-login.html`: the portal's account and password form (the
//!   page with `usr_name`), reached from
//!   <https://portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList> in a
//!   browser not signed in, through the agreement page's button.
//!
//! `VPN_CAPTURES=/path/to/dir cargo test -p adpt-isct --features campus-vpn --lib login::tests`
use super::*;

const APM_POLICY: &str = "https://apm.nap.gsic.titech.ac.jp/my.policy";
const PROXY: &str = "https://rp.nap.gsic.titech.ac.jp/vpnaccess_apm/service/";
const PORTAL_LOGIN: &str = "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login";

fn capture(name: &str) -> String {
    let path = std::path::Path::new(env!("VPN_CAPTURES")).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// The walk's three kinds of page as the school serves them: the APM's
/// hand-off posts on to the address its script sets (the form's own action
/// is empty), the proxy asks for the password alone, and the portal's
/// account form, which also has a password field, is not that step: filling
/// only its password would send a sign-in without the account.
#[test]
fn captured_vpn_pages_are_the_hand_off_the_password_step_and_not_the_portal() {
    let VpnPage::Relay(relay) = vpn(&capture("apm-handoff.html"), APM_POLICY) else {
        panic!("the APM's hand-off was not read as a relay");
    };
    assert_eq!(relay.action, PROXY);
    assert!(!relay.fields().is_empty());

    let VpnPage::Password(form) = vpn(&capture("proxy-password.html"), PROXY) else {
        panic!("the proxy's page was not read as the password step");
    };
    assert!(form.action.starts_with(PROXY), "{}", form.action);
    assert_eq!(form.get("password"), Some(""));

    let portal = capture("getaccess-login.html");
    assert_eq!(vpn(&portal, PORTAL_LOGIN), VpnPage::Other);
    assert!(matches!(
        tokyo(&portal, PORTAL_LOGIN),
        TokyoPage::Credentials(_)
    ));
}
