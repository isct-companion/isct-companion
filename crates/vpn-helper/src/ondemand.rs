//! On-demand standby: while the app holds a `Request::Standby` connection,
//! the campus routes point at a standby interface this helper owns, so the
//! first connection to campus needs no tunnel yet. A userspace TCP/IP stack
//! (ipstack) accepts it, the app is told (`StandbyEvent::Demand`) and signs
//! in, and once openconnect's own tunnel is up each held connection is dialled
//! again from a socket bound to that tunnel. The original packets carry the
//! standby address, not the one F5 assigns later, so they are never forwarded
//! as they are. openconnect keeps its own interface on both systems.
//!
//! While a tunnel is up the standby routes are withdrawn and new connections
//! use the tunnel directly. When it ends they come back. Any tunnel, pressed
//! or demanded, is ended here once it is no longer used (`Idleness`), whether
//! or not the app is still running: the school asks for connecting on demand
//! and disconnecting as soon as possible, its VPN ports being few.

use super::{current_tunnel, log, platform, stop, Config, State};
use ipstack::{IpStack, IpStackConfig, IpStackStream, IpStackTcpStream, IpStackUdpStream};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;
use vpn_protocol::{Ending, StandbyEvent, STANDBY_HEARTBEAT_SECS};

/// The campus network the tunnel carries, as `CAMPUS_NET` names it.
const CAMPUS: (Ipv4Addr, u8) = (Ipv4Addr::new(131, 112, 0, 0), 16);
/// The standby interface's own address: the IPv4 dummy address (RFC 7600),
/// which no network assigns, unlike 198.18/15 (Clash's fake IPs) or
/// 100.64/10 (Tailscale).
pub(crate) const STANDBY_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 0, 0, 8);
pub(crate) const STANDBY_MTU: u16 = 1400;
/// How long a held connection waits for the sign-in and the tunnel.
const FLOW_WAIT: Duration = Duration::from_secs(45);
const DIAL_WAIT: Duration = Duration::from_secs(15);
/// A demand repeats at most this often while connections keep arriving; the
/// app's back-off decides whether each one signs in.
const DEMAND_EVERY: Duration = Duration::from_secs(15);
const TICK: Duration = Duration::from_millis(500);
/// How long a tunnel with no use and no open connection is kept, growing a
/// step each time one is wanted again soon after being ended for it, so a
/// person who keeps coming back is not signed in over and over. Core's count
/// of tunnels that did not last restarts after 3 minutes up, which must stay
/// shorter than the first step.
const IDLE_STEPS: [Duration; 3] = [
    Duration::from_secs(5 * 60),
    Duration::from_secs(10 * 60),
    Duration::from_secs(15 * 60),
];
/// A tunnel started within this of an idle ending was ended too early.
const SOON_AGAIN: Duration = Duration::from_secs(10 * 60);
/// A tunnel without use for this long is ended even with connections open:
/// they are forgotten ones, kept up by keepalives alone.
const UNUSED: Duration = Duration::from_secs(30 * 60);
/// Use is at least this much traffic within `USE_WINDOW`; less is
/// keepalives (an SSH `ServerAliveInterval` sends some tens of bytes).
const USE_BYTES: u64 = 2 * 1024;
const USE_WINDOW: Duration = Duration::from_secs(60);
const IDLE_CHECK: Duration = Duration::from_secs(15);

/// The standby interface, as the route calls name it.
pub(crate) struct StandbyIf {
    pub(crate) name: String,
    /// Windows routes by index; macOS's `route` takes the name.
    #[cfg(windows)]
    pub(crate) index: u32,
}

/// One standby at a time. A newer connection takes over: the older one sees
/// the generation move on and tears down before the newer one sets up.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ACTIVE: Mutex<()> = Mutex::new(());

/// The campus network without `bypass`, as the fewest prefixes. Addresses
/// outside it are ignored: no standby route could capture them.
pub(crate) fn standby_prefixes(bypass: &[Ipv4Addr]) -> Vec<(Ipv4Addr, u8)> {
    fn cover(net: u32, len: u8, bypass: &[u32], out: &mut Vec<(Ipv4Addr, u8)>) {
        let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
        if !bypass.iter().any(|host| host & mask == net) {
            out.push((Ipv4Addr::from(net), len));
            return;
        }
        if len == 32 {
            return;
        }
        cover(net, len + 1, bypass, out);
        cover(net | 1 << (31 - len), len + 1, bypass, out);
    }
    let hosts: Vec<u32> = bypass.iter().map(|host| u32::from(*host)).collect();
    let mut out = Vec::new();
    cover(u32::from(CAMPUS.0), CAMPUS.1, &hosts, &mut out);
    out
}

/// What the stack's connections and the control loop share.
struct Plane {
    /// openconnect's interface index while a tunnel is up.
    tunnel: watch::Sender<Option<u32>>,
    /// A connection arrived since the last demand was sent.
    wanted: AtomicBool,
}

/// Serves one standby connection until the app goes away, a newer standby
/// takes over, or standby cannot run. Every way out removes the routes.
pub(crate) fn serve(mut writer: impl Write, cfg: &Config, state: &State, bypass: Vec<Ipv4Addr>) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let _active = ACTIVE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return failed(&mut writer, cfg, format!("standby runtime: {error}")),
    };
    let device = match runtime.block_on(async { platform::standby_device(cfg) }) {
        Ok(device) => device,
        Err(error) => return failed(&mut writer, cfg, format!("standby interface: {error}")),
    };
    let interface = match platform::standby_interface(&device) {
        Ok(interface) => interface,
        Err(error) => return failed(&mut writer, cfg, format!("standby interface: {error}")),
    };
    let prefixes = standby_prefixes(&bypass);
    log(
        cfg,
        &format!(
            "standby on {} ({} routes, {} bypassed)",
            interface.name,
            prefixes.len(),
            bypass.len()
        ),
    );
    let (tunnel, _) = watch::channel(None);
    let plane = Arc::new(Plane {
        tunnel,
        wanted: AtomicBool::new(false),
    });
    let routed = Mutex::new(false);
    let route = |on: bool| {
        let mut routed = routed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *routed == on {
            return Ok(());
        }
        *routed = on;
        for (net, len) in &prefixes {
            if on {
                platform::add_standby_route(*net, *len, &interface)?;
            } else {
                // Removal goes on past a failure, so nothing stays behind.
                if let Err(error) = platform::delete_standby_route(*net, *len, &interface) {
                    log(cfg, &format!("standby route {net}/{len}: {error}"));
                }
            }
        }
        Ok::<(), String>(())
    };

    let outcome = runtime.block_on(async {
        let mut config = IpStackConfig::default();
        config.mtu_unchecked(STANDBY_MTU);
        let mut stack = IpStack::new(config, device);
        let accepting = {
            let plane = plane.clone();
            let cfg = cfg.clone();
            tokio::spawn(async move {
                while let Ok(stream) = stack.accept().await {
                    plane.wanted.store(true, Ordering::SeqCst);
                    let plane = plane.clone();
                    match stream {
                        IpStackStream::Tcp(stream) => {
                            log(&cfg, &format!("standby held tcp to {}", stream.peer_addr()));
                            tokio::spawn(relay_tcp(stream, plane));
                        }
                        IpStackStream::Udp(stream) => {
                            log(&cfg, &format!("standby held udp to {}", stream.peer_addr()));
                            tokio::spawn(relay_udp(stream, plane));
                        }
                        // ICMP and the like only ask for the tunnel; once it
                        // is up they take its routes.
                        _ => {}
                    }
                }
                log(&cfg, "standby stack stopped");
            })
        };
        let mut last_demand: Option<Instant> = None;
        let mut last_beat: Option<Instant> = None;
        let mut ticks = tokio::time::interval(TICK);
        let result = loop {
            ticks.tick().await;
            if GENERATION.load(Ordering::SeqCst) != generation {
                break Ok(());
            }
            let up = current_tunnel(cfg, state);
            if up != *plane.tunnel.borrow() {
                if let Err(error) = route(up.is_none()) {
                    break Err(format!("standby route: {error}"));
                }
                plane.tunnel.send_replace(up);
            } else if up.is_none() {
                // The first pass: no tunnel, so standby takes the routes.
                if let Err(error) = route(true) {
                    break Err(format!("standby route: {error}"));
                }
            }
            let now = Instant::now();
            if up.is_none()
                && plane.wanted.load(Ordering::SeqCst)
                && last_demand.is_none_or(|at| now - at >= DEMAND_EVERY)
            {
                plane.wanted.store(false, Ordering::SeqCst);
                last_demand = Some(now);
                if send(&mut writer, &StandbyEvent::Demand).is_err() {
                    break Ok(());
                }
            }
            if up.is_some() {
                plane.wanted.store(false, Ordering::SeqCst);
            }
            if last_beat.is_none_or(|at| now - at >= Duration::from_secs(STANDBY_HEARTBEAT_SECS)) {
                last_beat = Some(now);
                let beat = StandbyEvent::Standby {
                    tunnel: up.is_some(),
                };
                // The app hung up: it quit, crashed or turned standby off.
                if send(&mut writer, &beat).is_err() {
                    break Ok(());
                }
            }
        };
        accepting.abort();
        result
    });
    if let Err(error) = route(false) {
        log(cfg, &format!("standby route: {error}"));
    }
    runtime.shutdown_timeout(Duration::from_secs(2));
    match outcome {
        Ok(()) => log(cfg, "standby ended"),
        Err(error) => failed(&mut writer, cfg, error),
    }
}

fn send(writer: &mut impl Write, event: &StandbyEvent) -> std::io::Result<()> {
    writeln!(writer, "{}", event.to_line())?;
    writer.flush()
}

fn failed(writer: &mut impl Write, cfg: &Config, detail: String) {
    log(cfg, &detail);
    let _ = send(writer, &StandbyEvent::Failed { detail });
}

/// The tunnel's interface once it is up, or `None` when it did not come up
/// within `FLOW_WAIT`.
async fn tunnel(plane: &Plane) -> Option<u32> {
    let mut up = plane.tunnel.subscribe();
    let found = tokio::time::timeout(FLOW_WAIT, up.wait_for(Option::is_some)).await;
    match found {
        Ok(Ok(index)) => *index,
        _ => None,
    }
}

/// A socket bound to the tunnel, so its packets never loop back into
/// standby whatever the routes say at the moment.
fn bound_socket(kind: socket2::Type, index: u32) -> std::io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, kind, None)?;
    platform::bind_to_interface(&socket, index)?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

async fn relay_tcp(mut stream: IpStackTcpStream, plane: Arc<Plane>) {
    let Some(index) = tunnel(&plane).await else {
        return;
    };
    let dial = async {
        let socket = bound_socket(socket2::Type::STREAM, index)?;
        let socket = tokio::net::TcpSocket::from_std_stream(socket.into());
        socket.connect(stream.peer_addr()).await
    };
    let Ok(Ok(mut upstream)) = tokio::time::timeout(DIAL_WAIT, dial).await else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut stream, &mut upstream).await;
}

/// One datagram flow: ipstack ends the stream after its UDP timeout.
async fn relay_udp(mut stream: IpStackUdpStream, plane: Arc<Plane>) {
    let Some(index) = tunnel(&plane).await else {
        return;
    };
    let peer: SocketAddr = stream.peer_addr();
    let open = || -> std::io::Result<tokio::net::UdpSocket> {
        let socket = bound_socket(socket2::Type::DGRAM, index)?;
        socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())?;
        tokio::net::UdpSocket::from_std(socket.into())
    };
    let Ok(upstream) = open() else {
        return;
    };
    if upstream.connect(peer).await.is_err() {
        return;
    }
    let mut out = vec![0u8; usize::from(STANDBY_MTU)];
    let mut back = vec![0u8; usize::from(STANDBY_MTU)];
    loop {
        tokio::select! {
            read = stream.read(&mut out) => match read {
                Ok(0) | Err(_) => return,
                Ok(n) => if upstream.send(&out[..n]).await.is_err() { return },
            },
            received = upstream.recv(&mut back) => match received {
                Ok(n) => if stream.write_all(&back[..n]).await.is_err() { return },
                Err(_) => return,
            },
        }
    }
}

/// One reading of the tunnel up: openconnect's pid, the bytes its interface
/// has carried, and whether a TCP connection is open through it.
#[derive(Clone, Copy)]
pub(crate) struct Reading {
    pub(crate) pid: u32,
    pub(crate) bytes: u64,
    pub(crate) open: bool,
}

/// The tunnel watched and when it was last used or held a connection.
struct Watched {
    pid: u32,
    /// The counter at the start of the current `USE_WINDOW`, and that start.
    window: (u64, Instant),
    used: Instant,
    held: Instant,
}

/// Decides when a tunnel is no longer used. Kept in the helper's memory: a
/// restarted helper begins again at the first idle step.
#[derive(Default)]
pub(crate) struct Idleness {
    watched: Option<Watched>,
    step: usize,
    idle_ended: Option<Instant>,
}

impl Idleness {
    /// The tunnel as read at `now` (`None` while none is up); `Some` when it
    /// is to be ended.
    pub(crate) fn observe(&mut self, now: Instant, reading: Option<Reading>) -> Option<Ending> {
        let Some(reading) = reading else {
            self.watched = None;
            return None;
        };
        let watched = match self.watched.as_mut() {
            Some(watched) if watched.pid == reading.pid => watched,
            _ => {
                let again = self
                    .idle_ended
                    .take()
                    .is_some_and(|ended| now.duration_since(ended) <= SOON_AGAIN);
                self.step = if again {
                    (self.step + 1).min(IDLE_STEPS.len() - 1)
                } else {
                    0
                };
                self.watched = Some(Watched {
                    pid: reading.pid,
                    window: (reading.bytes, now),
                    used: now,
                    held: now,
                });
                return None;
            }
        };
        if reading.bytes.wrapping_sub(watched.window.0) >= USE_BYTES {
            watched.used = now;
            watched.window = (reading.bytes, now);
        } else if now.duration_since(watched.window.1) >= USE_WINDOW {
            watched.window = (reading.bytes, now);
        }
        if reading.open {
            watched.held = now;
        }
        let quiet = now.duration_since(watched.used.max(watched.held));
        let ending = if now.duration_since(watched.used) >= UNUSED {
            // Forgotten rather than paused: the next tunnel starts afresh.
            self.step = 0;
            Ending::Unused
        } else if quiet >= IDLE_STEPS[self.step] {
            self.idle_ended = Some(now);
            Ending::Idle
        } else {
            return None;
        };
        self.watched = None;
        Some(ending)
    }
}

/// Ends the tunnel once `Idleness` says it is no longer used. Runs for the
/// helper's life: the app may have quit.
pub(crate) fn watch_idle(cfg: Config, state: State) {
    std::thread::spawn(move || {
        let mut idleness = Idleness::default();
        loop {
            std::thread::sleep(IDLE_CHECK);
            let tunnel = {
                let st = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                st.child
                    .as_ref()
                    .map(|child| (child.id(), st.address.clone()))
            };
            let reading = match (tunnel, current_tunnel(&cfg, &state)) {
                (Some((pid, address)), Some(index)) => {
                    // A counter that cannot be read is no reason to end it.
                    let Some(bytes) = platform::traffic(index) else {
                        continue;
                    };
                    let open = address
                        .and_then(|address| address.parse().ok())
                        .is_some_and(platform::has_connections);
                    Some(Reading { pid, bytes, open })
                }
                _ => None,
            };
            let Some(ending) = idleness.observe(Instant::now(), reading) else {
                continue;
            };
            log(
                &cfg,
                match ending {
                    Ending::Idle => "tunnel idle; disconnecting",
                    Ending::Unused => "tunnel unused for 30 minutes; disconnecting",
                },
            );
            let _ = stop(&cfg, &state);
            state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .ended = Some(ending);
        }
    });
}

#[cfg(test)]
#[path = "tests/ondemand.rs"]
mod tests;
