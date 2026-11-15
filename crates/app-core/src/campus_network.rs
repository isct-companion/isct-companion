//! Whether this device is on the university's own network, told by its
//! addresses alone: reading the Wi-Fi name would need location permission.
//! The campus Wi-Fi sign-in and the campus VPN both rest on this one answer.

use ipnet::IpNet;
use std::net::IpAddr;

/// ScienceTokyo (WPA2-PSK, then the web sign-in) hands out 10.2.0.0/16.
const SCIENCE_TOKYO_NET: &str = "10.2.0.0/16";
/// ScienceTokyoX (802.1X, NOC ID) client ranges, Ookayama/Tamachi then
/// Yokohama, published on
/// https://www.noc.cii.isct.ac.jp/service/nocid/sciencetokyox/. IPv4 is often
/// not leased at all (IPv6 only, macOS adds a 192.0.0.2 CLAT address), so the
/// IPv6 prefixes matter as much.
const SCIENCE_TOKYO_X_NETS: [&str; 4] = [
    "10.3.0.0/17",
    "10.3.128.0/20",
    "2403:4440:40:61::/64",
    "2403:4440:50:63::/64",
];
/// The campus's own IPv4 block, as a wired campus port assigns it. The VPN
/// tunnel's address comes from it too, and is left out by the caller.
const CAMPUS_NET: &str = "131.112.0.0/16";

fn within(net: &str, ip: IpAddr) -> bool {
    net.parse::<IpNet>().is_ok_and(|net| net.contains(&ip))
}

/// An address ScienceTokyo leases.
pub fn is_science_tokyo_address(ip: IpAddr) -> bool {
    within(SCIENCE_TOKYO_NET, ip)
}

/// An address ScienceTokyoX leases.
pub fn is_science_tokyo_x_address(ip: IpAddr) -> bool {
    SCIENCE_TOKYO_X_NETS.iter().any(|net| within(net, ip))
}

/// Whether any of the device's `addresses`, other than the VPN `tunnel`'s,
/// puts it on the campus network, where campus hosts are reached directly.
pub fn on_campus(addresses: impl IntoIterator<Item = IpAddr>, tunnel: Option<IpAddr>) -> bool {
    addresses.into_iter().any(|ip| {
        Some(ip) != tunnel
            && (is_science_tokyo_address(ip)
                || is_science_tokyo_x_address(ip)
                || within(CAMPUS_NET, ip))
    })
}

#[cfg(test)]
#[path = "tests/campus_network.rs"]
mod tests;
