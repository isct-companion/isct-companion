//! On-demand connecting's side of the helper: while Core has it on, one
//! thread holds the helper's standby connection open and turns each demand
//! into a sign-in. The helper keeps the campus routes on standby only while
//! this connection lives, so the app quitting or crashing takes them away.
//! Whether a demand signs in is Core's (`VpnService::demanded`), and so is
//! whether standby is wanted at all: on the campus network it is not
//! (`VpnService::wants_standby`), so this thread rereads the network now and
//! then and lets standby go or takes it back.
use super::helper::{self, standby_stream};
use app_core::vpn::{VpnError, VpnService};
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use vpn_protocol::{Request, StandbyEvent};

/// How long a helper that cannot hold standby is left before trying again.
const RETRY: Duration = Duration::from_secs(30);
/// How often the network is reread, to notice joining or leaving campus.
const NETWORK_EVERY: Duration = Duration::from_secs(10);

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Starts the standby thread unless it runs already. It ends by itself once
/// Core turns on-demand off.
pub(crate) fn ensure(app: tauri::AppHandle) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let service = app.state::<Arc<VpnService>>().inner().clone();
        while service.on_demand() {
            let wait = if !reread(&app, &service) {
                // On campus: nothing to hold until the device leaves.
                NETWORK_EVERY
            } else {
                match hold(&app, &service) {
                    Ok(()) => Duration::ZERO,
                    Err(error) => {
                        log::warn!("[vpn] standby: {error}");
                        RETRY
                    }
                }
            };
            // Asleep in small steps, so turning it off ends the thread.
            for _ in 0..wait.as_secs() {
                if !service.on_demand() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
        // A turn-on that came while this thread was ending.
        if service.on_demand() {
            ensure(app);
        }
    });
}

/// Rereads the network; whether standby is wanted now. The card is told when
/// the device joins or leaves campus, so it says why nothing is held.
fn reread(app: &tauri::AppHandle, service: &VpnService) -> bool {
    let before = service.on_campus();
    service.network(super::addresses());
    if service.on_campus() != before {
        helper::tell(app, service, service.state(), None);
    }
    service.wants_standby()
}

/// The campus addresses of the hosts standby must leave alone, looked up
/// now: the routes are set from addresses, not names.
fn bypass() -> Vec<Ipv4Addr> {
    let mut addresses: Vec<Ipv4Addr> = VpnService::standby_bypass_hosts()
        .filter_map(|host| (host, 443).to_socket_addrs().ok())
        .flatten()
        .filter_map(|address| match address.ip() {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        })
        .collect();
    addresses.sort();
    addresses.dedup();
    addresses
}

/// One standby connection, until the helper or Core ends it. `Ok` when Core
/// turned on-demand off.
fn hold(app: &tauri::AppHandle, service: &Arc<VpnService>) -> Result<(), String> {
    let mut stream = standby_stream().map_err(|error| error.to_string())?;
    let request = Request::Standby { bypass: bypass() };
    let line = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    writeln!(stream, "{line}").map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut network_read = Instant::now();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Err("the helper ended standby".into()),
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
        // Dropping the connection is how standby is turned off, also on
        // reaching the campus network. The helper's heartbeat paces this.
        if !service.on_demand() {
            return Ok(());
        }
        if network_read.elapsed() >= NETWORK_EVERY {
            network_read = Instant::now();
            if !reread(app, service) {
                return Ok(());
            }
        }
        match serde_json::from_str::<StandbyEvent>(line.trim()) {
            Ok(StandbyEvent::Standby { .. }) => {}
            Ok(StandbyEvent::Demand) => {
                tauri::async_runtime::spawn(demand(app.clone(), service.clone()));
            }
            Ok(StandbyEvent::Failed { detail }) => return Err(detail),
            // An older helper answers with a reply it cannot keep up.
            Err(_) => return Err(format!("unexpected line: {}", line.trim())),
        }
    }
}

/// Something used the campus network with no tunnel up: Core decides
/// whether to sign in, and how the outcome counts.
async fn demand(app: tauri::AppHandle, service: Arc<VpnService>) {
    if !service.demanded(chrono::Utc::now()) {
        return;
    }
    // A press or a reconnect in progress is bringing the tunnel up already.
    let Some(_login) = service.begin_login() else {
        return;
    };
    let outcome = match crate::core_ports::core(&app)
        .sessions()
        .campus_vpn_login()
        .await
    {
        // Turned off while signing in: no tunnel behind its back.
        Ok(_) if !service.on_demand() => service.state(),
        Ok(login) => {
            let connecting = service.clone();
            tauri::async_runtime::spawn_blocking(move || connecting.connect(&login.cookie))
                .await
                .unwrap_or_else(|_| service.state())
        }
        Err(error) => service.failed(VpnError::Login(error)),
    };
    let notice = service.demand_finished(&outcome, chrono::Utc::now());
    helper::tell(&app, &service, outcome, notice);
}
