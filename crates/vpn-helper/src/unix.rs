//! The macOS daemon: launchd keeps it running, the GUI reaches it on a
//! root:staff 0660 Unix socket, and the route hook is `assets/vpnc-script`,
//! whose `route add` line marks the tunnel as up.

use super::ondemand::StandbyIf;
use super::{handle, log, new_state, parse_args, Config, Inner};
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread;
use vpn_protocol::SOCKET_PATH;

const PID_FILE: &str = "/var/run/app.isctcompanion.vpnhelper.pid";
const LOG: &str = "/var/log/app.isctcompanion.vpnhelper.log";
/// The APM's address, which the route hook pins to the physical gateway.
const APM_PIN: &str = "131.112.12.180";

/// The hook's scoped DNS entry for the campus domains.
const DNS_KEY: &str = "State:/Network/Service/isct-campus-vpn/DNS";

/// Removes what the hook set up outside the tunnel. It does this itself when
/// openconnect shuts down in order; a daemon restart or a killed openconnect
/// skips it. A pin to an old network's gateway makes the APM unreachable, and
/// resolvers behind a tunnel that is gone make every campus name time out.
fn clear_leftovers() {
    let _ = Command::new("/sbin/route")
        .args(["-n", "delete", "-host", APM_PIN])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Ok(mut scutil) = Command::new("/usr/sbin/scutil")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut input) = scutil.stdin.take() {
            let _ = writeln!(input, "remove {DNS_KEY}");
        }
        let _ = scutil.wait();
    }
}

pub(crate) fn defaults() -> Config {
    Config {
        socket: SOCKET_PATH.into(),
        openconnect: "/usr/local/bin/openconnect".into(),
        script: "/Library/Application Support/IsctCompanion/vpn/vpnc-script".into(),
        server: super::DEFAULT_SERVER.into(),
        libdirs: Vec::new(),
        log: LOG.into(),
    }
}

pub(crate) fn main() {
    let cfg = parse_args(std::env::args().skip(1));
    clear_leftovers();
    let _ = fs::remove_file(&cfg.socket);
    let listener = match UnixListener::bind(&cfg.socket) {
        Ok(l) => l,
        Err(e) => {
            log(&cfg, &format!("bind {}: {e}", cfg.socket.display()));
            std::process::exit(1);
        }
    };
    let _ = fs::set_permissions(&cfg.socket, fs::Permissions::from_mode(0o660));
    if let Ok(c) = CString::new(cfg.socket.to_string_lossy().as_bytes()) {
        unsafe {
            libc::chown(c.as_ptr(), 0, 20); // root:staff
        }
    }
    log(&cfg, "helper started");

    let state = new_state();
    super::ondemand::watch_idle(cfg.clone(), Arc::clone(&state));
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let cfg = cfg.clone();
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Ok(writer) = stream.try_clone() {
                        handle(stream, writer, &cfg, &state);
                    }
                });
            }
            Err(e) => log(&cfg, &format!("accept: {e}")),
        }
    }
}

pub(crate) fn decode_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub(crate) fn prepare(cmd: &mut Command, cfg: &Config) {
    cmd.arg("--pid-file").arg(PID_FILE);
    if !cfg.libdirs.is_empty() {
        let joined = cfg
            .libdirs
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(":");
        cmd.env("DYLD_LIBRARY_PATH", joined);
    }
}

/// launchd owns the daemon's lifetime; openconnect needs no extra tie to it.
pub(crate) fn adopt(_child: &Child, _cfg: &Config) {}

/// The route hook's own output marks the route (`Observation::Route`).
pub(crate) fn confirm_route(_state: &mut Inner) {}

pub(crate) fn terminate(child: &Child, _cfg: &Config) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
}

pub(crate) fn after_stop(_cfg: &Config) {
    let _ = fs::remove_file(PID_FILE);
    clear_leftovers();
}

/// The tunnel's utun, named by the route hook's `route add` line.
pub(crate) fn tunnel_index(state: &Inner) -> Option<u32> {
    let name = CString::new(state.ifname.as_deref()?).ok()?;
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    (index != 0).then_some(index)
}

/// A utun of the helper's own for standby. With `packet_information` on,
/// tun strips the 4-byte header macOS puts on every utun packet and adds it
/// back on writes, so ipstack sees bare IP. tun sets the address and the mask
/// one call at a time, and macOS then keeps a /24; `ifconfig` sets the /32
/// point-to-point address in one, as the route hook does for the tunnel. The
/// helper installs every route.
pub(crate) fn standby_device(_cfg: &Config) -> Result<tun::AsyncDevice, String> {
    use super::ondemand::{STANDBY_ADDRESS, STANDBY_MTU};
    use tun::AbstractDevice;
    let mut config = tun::Configuration::default();
    config.mtu(STANDBY_MTU).up();
    config.platform_config(|platform| {
        platform.packet_information(true).enable_routing(false);
    });
    let device = tun::create_as_async(&config).map_err(|error| error.to_string())?;
    let name = device.tun_name().map_err(|error| error.to_string())?;
    let address = STANDBY_ADDRESS.to_string();
    let output = Command::new("/sbin/ifconfig")
        .args([
            &name,
            "inet",
            &address,
            &address,
            "netmask",
            "255.255.255.255",
            "up",
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(device)
}

pub(crate) fn standby_interface(device: &tun::AsyncDevice) -> Result<StandbyIf, String> {
    use tun::AbstractDevice;
    let name = device.tun_name().map_err(|error| error.to_string())?;
    Ok(StandbyIf { name })
}

fn route(action: &str, net: Ipv4Addr, len: u8, interface: &StandbyIf) -> Result<(), String> {
    let output = Command::new("/sbin/route")
        .args([
            "-n",
            action,
            "-net",
            &format!("{net}/{len}"),
            "-interface",
            &interface.name,
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
}

pub(crate) fn add_standby_route(
    net: Ipv4Addr,
    len: u8,
    interface: &StandbyIf,
) -> Result<(), String> {
    route("add", net, len, interface)
}

pub(crate) fn delete_standby_route(
    net: Ipv4Addr,
    len: u8,
    interface: &StandbyIf,
) -> Result<(), String> {
    route("delete", net, len, interface)
}

/// `IP_BOUND_IF`: the socket's packets leave through `index` only.
pub(crate) fn bind_to_interface(socket: &socket2::Socket, index: u32) -> std::io::Result<()> {
    socket.bind_device_by_index_v4(std::num::NonZeroU32::new(index))
}

/// Whether an established TCP connection has `address`, the tunnel's own,
/// as its local end: one going through the tunnel, such as an open SSH.
pub(crate) fn has_connections(address: Ipv4Addr) -> bool {
    let Ok(out) = Command::new("/usr/sbin/netstat")
        .args(["-an", "-p", "tcp", "-f", "inet"])
        .output()
    else {
        return false;
    };
    established_from(&String::from_utf8_lossy(&out.stdout), address)
}

/// `netstat -an` lines name the local end as `a.b.c.d.port`.
pub(crate) fn established_from(table: &str, address: Ipv4Addr) -> bool {
    table.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.len() >= 6
            && fields[5] == "ESTABLISHED"
            && fields[3]
                .rsplit_once('.')
                .and_then(|(host, _)| host.parse::<Ipv4Addr>().ok())
                == Some(address)
    })
}

/// Bytes in and out of interface `index` so far, from its link counters.
/// They wrap at 2^32 per direction; only a change is read from them.
pub(crate) fn traffic(index: u32) -> Option<u64> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut entry = list;
    while !entry.is_null() {
        let ifa = unsafe { &*entry };
        entry = ifa.ifa_next;
        if ifa.ifa_addr.is_null()
            || ifa.ifa_data.is_null()
            || i32::from(unsafe { (*ifa.ifa_addr).sa_family }) != libc::AF_LINK
            || unsafe { libc::if_nametoindex(ifa.ifa_name) } != index
        {
            continue;
        }
        let data = unsafe { &*(ifa.ifa_data as *const libc::if_data) };
        found = Some(u64::from(data.ifi_ibytes) + u64::from(data.ifi_obytes));
        break;
    }
    unsafe { libc::freeifaddrs(list) };
    found
}
