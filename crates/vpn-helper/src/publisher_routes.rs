//! The publisher sites' routes (`Request::Routes`): their papers are licensed
//! to campus addresses, so their IPv4 addresses go through the tunnel like
//! the campus network. The tunnel carries no IPv6, so their IPv6 addresses
//! are refused while a tunnel or standby is up; a browser racing both then
//! falls back to IPv4 at once instead of going around the tunnel. Standby
//! routes the IPv4 ones itself (`ondemand`), so reaching a publisher wakes it.
//!
//! What is installed is kept here and only the difference is applied, so
//! `sync` can run on every change: a tunnel up or down, a new list, standby.

use super::{log, platform, Config, State};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Whether standby holds the campus routes now; set by `ondemand::serve`.
pub(crate) static STANDBY: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Applied {
    /// The tunnel the IPv4 routes point at.
    via: Option<platform::Via>,
    v4: Vec<Ipv4Addr>,
    v6: Vec<Ipv6Addr>,
}

static APPLIED: Mutex<Applied> = Mutex::new(Applied {
    via: None,
    v4: Vec::new(),
    v6: Vec::new(),
});

/// Brings the installed routes in line with the state now.
pub(crate) fn sync(cfg: &Config, state: &State) {
    let (via, v4, v6) = {
        let st = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let via = st.routed.then(|| platform::tunnel_via(&st)).flatten();
        (via, st.publishers_v4.clone(), st.publishers_v6.clone())
    };
    let refuse_v6 = via.is_some() || STANDBY.load(Ordering::SeqCst);
    let mut applied = APPLIED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // A different tunnel (or none): the old one's routes go first. Most went
    // with its interface; removing them again is harmless.
    if applied.via != via {
        if let Some(old) = applied.via.take() {
            for ip in applied.v4.drain(..) {
                let _ = platform::delete_host_route(ip, &old);
            }
        }
        applied.v4.clear();
        applied.via.clone_from(&via);
    }
    let want4 = if via.is_some() { v4 } else { Vec::new() };
    if let Some(via) = &via {
        for ip in applied.v4.clone() {
            if !want4.contains(&ip) {
                if let Err(error) = platform::delete_host_route(ip, via) {
                    log(cfg, &format!("publisher route {ip}: {error}"));
                }
            }
        }
        for ip in &want4 {
            if !applied.v4.contains(ip) {
                if let Err(error) = platform::add_host_route(*ip, via) {
                    log(cfg, &format!("publisher route {ip}: {error}"));
                }
            }
        }
    }
    applied.v4 = want4;

    let want6 = if refuse_v6 { v6 } else { Vec::new() };
    for ip in applied.v6.clone() {
        if !want6.contains(&ip) {
            let _ = platform::delete_refused_route(ip);
        }
    }
    for ip in &want6 {
        if !applied.v6.contains(ip) {
            if let Err(error) = platform::add_refused_route(*ip) {
                log(cfg, &format!("publisher IPv6 {ip}: {error}"));
            }
        }
    }
    applied.v6 = want6;
}
