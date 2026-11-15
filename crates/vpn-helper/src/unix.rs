//! The macOS daemon: registered from inside the app bundle with
//! SMAppService, so launchd runs it as root straight from
//! `Contents/Resources/resources/vpn` and the person allows it once in System
//! Settings. The GUI reaches it on a root:staff 0660 Unix socket, and the
//! route hook is `assets/vpnc-script`, whose `route add` line marks the
//! tunnel as up.
//!
//! The bundle belongs to the person, so nothing in it runs as root without a
//! check: the route hook is compiled into this binary and written to a
//! root-owned folder, and a release build verifies openconnect's signature
//! before each start (`prepare`). When an app update replaces this binary it
//! exits after its reply, with no tunnel up, and launchd starts the new one.

use super::ondemand::StandbyIf;
use super::{handle, log, new_state, parse_args, Config, Inner};
use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::net::Ipv4Addr;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
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

/// Root-owned folder the route hook is written to; the earlier installer
/// copied openconnect here as well.
const SUPPORT_DIR: &str = "/Library/Application Support/IsctCompanion/vpn";
const ROUTE_HOOK: &str = include_str!("../assets/vpnc-script");

/// What the osascript installer of helper revisions 1–3 put under /Library.
/// This daemon has its own label, so it can stop that one and remove it.
const LEGACY_LABEL: &str = "app.isctcompanion.vpnhelper";
const LEGACY_FILES: &[&str] = &[
    "/Library/LaunchDaemons/app.isctcompanion.vpnhelper.plist",
    "/Library/PrivilegedHelperTools/app.isctcompanion.vpnhelper",
    "/Library/PrivilegedHelperTools/isct-vpn-script",
];

/// The signer openconnect must carry, set by `bundle-vpn.mjs` when it signs
/// with a Developer ID. A local ad hoc build has none and skips the check.
const SIGNER_TEAM: Option<&str> = option_env!("ISCT_VPN_SIGNER_TEAM");

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
    bundled(&own_dir())
}

/// The payload beside this binary in the app bundle: openconnect with its
/// libraries as `bundle-vpn.mjs` lays them out.
pub(crate) fn bundled(dir: &Path) -> Config {
    Config {
        socket: SOCKET_PATH.into(),
        openconnect: dir.join("openconnect/bin/openconnect"),
        script: Path::new(SUPPORT_DIR).join("vpnc-script"),
        server: super::DEFAULT_SERVER.into(),
        libdirs: vec![dir.join("openconnect/lib")],
        log: LOG.into(),
    }
}

fn own_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// The identity of this binary's file; an update puts another file there.
fn exe_identity() -> Option<(u64, u64)> {
    let meta = fs::metadata(std::env::current_exe().ok()?).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// Stops and removes the earlier osascript install. Each step is allowed to
/// find nothing: a Mac that never had it, or one already cleaned.
fn retire_legacy() {
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("system/{LEGACY_LABEL}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    for file in LEGACY_FILES {
        let _ = fs::remove_file(file);
    }
    let _ = fs::remove_dir_all(Path::new(SUPPORT_DIR).join("openconnect"));
}

/// Writes the route hook root-owned and not writable by anyone else, through
/// a temporary file so openconnect never runs half of it.
fn write_route_hook(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755))?;
    let staged = dir.join(".vpnc-script.new");
    fs::write(&staged, ROUTE_HOOK)?;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    fs::rename(&staged, path)
}

/// The code requirement openconnect is checked against, for a team.
pub(crate) fn signer_requirement(team: &str) -> String {
    format!(
        "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] and \
         certificate leaf[field.1.2.840.113635.100.6.1.13] and certificate leaf[subject.OU] = \"{team}\""
    )
}

fn verify_signer(binary: &Path) -> Result<(), String> {
    let Some(team) = SIGNER_TEAM else {
        return Ok(());
    };
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict"])
        .arg(format!("-R={}", signer_requirement(team)))
        .arg(binary)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("codesign: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} is not signed by the app's developer",
            binary.display()
        ))
    }
}

pub(crate) fn main() {
    let cfg = parse_args(std::env::args().skip(1));
    retire_legacy();
    clear_leftovers();
    if let Err(e) = write_route_hook(&cfg.script) {
        log(&cfg, &format!("route hook {}: {e}", cfg.script.display()));
        std::process::exit(1);
    }
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
    let started_as = exe_identity();
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let cfg = cfg.clone();
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    if let Ok(writer) = stream.try_clone() {
                        handle(stream, writer, &cfg, &state);
                    }
                    // An update replaced this binary: hand over to the new
                    // one once no tunnel depends on this process.
                    if exe_identity() != started_as && state.lock().unwrap().child.is_none() {
                        log(&cfg, "helper replaced; exiting for launchd to start it");
                        std::process::exit(0);
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

pub(crate) fn prepare(cmd: &mut Command, cfg: &Config) -> Result<(), String> {
    verify_signer(&cfg.openconnect)?;
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
    Ok(())
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
    run_route(&[
        "-n",
        action,
        "-net",
        &format!("{net}/{len}"),
        "-interface",
        &interface.name,
    ])
}

/// The tunnel's routes point at its utun by name.
pub(crate) type Via = String;

pub(crate) fn tunnel_via(state: &Inner) -> Option<Via> {
    state.ifname.clone()
}

/// A publisher's address through the tunnel.
pub(crate) fn add_host_route(ip: Ipv4Addr, via: &Via) -> Result<(), String> {
    run_route(&["-n", "add", "-host", &ip.to_string(), "-interface", via])
}

pub(crate) fn delete_host_route(ip: Ipv4Addr, via: &Via) -> Result<(), String> {
    run_route(&["-n", "delete", "-host", &ip.to_string(), "-interface", via])
}

/// A publisher's IPv6 address refused at once (`-reject` answers
/// unreachable), so a browser falls back to its IPv4 one, which goes through
/// the tunnel.
pub(crate) fn add_refused_route(ip: std::net::Ipv6Addr) -> Result<(), String> {
    run_route(&[
        "-n",
        "add",
        "-inet6",
        "-host",
        &ip.to_string(),
        "::1",
        "-reject",
    ])
}

pub(crate) fn delete_refused_route(ip: std::net::Ipv6Addr) -> Result<(), String> {
    run_route(&["-n", "delete", "-inet6", "-host", &ip.to_string()])
}

fn run_route(args: &[&str]) -> Result<(), String> {
    let output = Command::new("/sbin/route")
        .args(args)
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
