use super::*;
use std::collections::HashMap;

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

/// openconnect's variables are the hook's only input. Without the tunnel's
/// index and address there is nothing to route, which must fail rather than
/// route the campus somewhere else.
#[test]
fn the_tunnel_is_read_from_openconnects_variables() {
    let tunnel = Tunnel::from_env(env(&[
        ("TUNIDX", "23"),
        ("INTERNAL_IP4_ADDRESS", "10.5.6.7"),
        ("INTERNAL_IP4_MTU", "1356"),
        ("VPNGATEWAY", "131.112.12.180"),
    ]))
    .unwrap();
    assert_eq!(
        tunnel,
        Tunnel {
            index: 23,
            address: Ipv4Addr::new(10, 5, 6, 7),
            mtu: Some(1356),
            gateway: Some(Ipv4Addr::new(131, 112, 12, 180)),
        }
    );
    // An IPv6 server is not pinned; a missing MTU keeps the adapter's.
    let tunnel = Tunnel::from_env(env(&[
        ("TUNIDX", "23"),
        ("INTERNAL_IP4_ADDRESS", "10.5.6.7"),
        ("VPNGATEWAY", "2001:db8::1"),
    ]))
    .unwrap();
    assert_eq!((tunnel.mtu, tunnel.gateway), (None, None));
    assert!(Tunnel::from_env(env(&[("INTERNAL_IP4_ADDRESS", "10.5.6.7")])).is_err());
    assert!(Tunnel::from_env(env(&[("TUNIDX", "23")])).is_err());
}

/// IP Helper takes addresses in network byte order; a swapped order would
/// route 0.0.112.131/16 and leave the campus unreachable.
#[test]
fn addresses_reach_ip_helper_in_network_order() {
    let raw = sockaddr(CAMPUS);
    let bytes = unsafe { raw.Ipv4.sin_addr.S_un.S_un_b };
    assert_eq!(
        [bytes.s_b1, bytes.s_b2, bytes.s_b3, bytes.s_b4],
        [131, 112, 0, 0]
    );
    assert_eq!(ipv4(&raw), Some(CAMPUS));
}

/// The live table can be read without privileges; no interface index this
/// high exists, so nothing claims the campus route for it.
#[test]
fn the_route_check_reads_the_live_table() {
    assert!(!routes().is_empty());
    assert!(!campus_route_on(u32::MAX));
}
