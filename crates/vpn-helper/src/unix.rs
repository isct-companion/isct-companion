//! The macOS daemon: launchd keeps it running, the GUI reaches it on a
//! root:staff 0660 Unix socket, and the route hook is `assets/vpnc-script`,
//! whose `route add` line marks the tunnel as up.

use super::{handle, log, new_state, parse_args, Config, Inner};
use std::ffi::CString;
use std::fs;
use std::io::Write;
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
