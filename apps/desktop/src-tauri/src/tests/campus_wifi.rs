use super::is_science_tokyo_x_address;

#[test]
fn science_tokyo_x_ranges_match_the_noc_guide() {
    for ip in [
        "10.3.0.1",
        "10.3.143.254",
        "2403:4440:40:61::1",
        "2403:4440:50:63::1",
    ] {
        assert!(is_science_tokyo_x_address(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["10.2.0.1", "10.3.144.1", "192.0.0.2", "2403:4440:40:62::1"] {
        assert!(!is_science_tokyo_x_address(ip.parse().unwrap()), "{ip}");
    }
}
