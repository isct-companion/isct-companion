//! Where the campus printer listens: lpd://172.20.83.212/kyocera. It is
//! reached from ScienceTokyo Wi-Fi; ScienceTokyoX and the campus VPN do not
//! reach it, even with a route to it through the tunnel.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

#[cfg(any(test, target_os = "macos"))]
pub(crate) const HOST: &str = "172.20.83.212";
pub(crate) const QUEUE: &str = "kyocera";
/// RFC 1179's port.
pub(crate) const PORT: u16 = 515;
/// `HOST` and `PORT` as a socket address (`tests/student_id.rs` keeps them
/// in step).
pub const ADDRESS: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(172, 20, 83, 212), PORT));
