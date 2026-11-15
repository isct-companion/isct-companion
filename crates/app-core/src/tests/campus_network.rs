use super::*;

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

/// The NOC's published ranges: an address just outside one must not count,
/// or the Wi-Fi card would call another network ScienceTokyoX.
#[test]
fn science_tokyo_x_ranges_match_the_noc_guide() {
    for text in [
        "10.3.0.1",
        "10.3.143.254",
        "2403:4440:40:61::1",
        "2403:4440:50:63::1",
    ] {
        assert!(is_science_tokyo_x_address(ip(text)), "{text}");
    }
    for text in ["10.2.0.1", "10.3.144.1", "192.0.0.2", "2403:4440:40:62::1"] {
        assert!(!is_science_tokyo_x_address(ip(text)), "{text}");
    }
    assert!(is_science_tokyo_address(ip("10.2.40.7")));
    assert!(!is_science_tokyo_address(ip("10.3.0.1")));
}

/// On campus the VPN is not needed, so automatic management stands down and
/// a press asks first. The tunnel's own campus address must not count, or a
/// connected VPN would call every network the campus one.
#[test]
fn campus_wifi_and_wired_ports_are_on_campus_but_the_tunnel_is_not() {
    // ScienceTokyoX, IPv6 only with the CLAT address beside it.
    assert!(on_campus(
        [ip("192.0.0.2"), ip("2403:4440:40:61:a8:ecdb:2c6a:71bb")],
        None
    ));
    assert!(on_campus([ip("10.2.3.4")], None));
    assert!(on_campus([ip("131.112.200.9")], None));
    // At home with the tunnel up.
    let tunnel = ip("131.112.10.19");
    assert!(!on_campus(
        [ip("192.168.1.20"), tunnel, ip("192.0.0.8")],
        Some(tunnel)
    ));
    assert!(!on_campus([ip("192.168.1.20"), ip("2001:db8::1")], None));
}
