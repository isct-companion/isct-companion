//! Privileged helper for the IsctCompanion campus VPN.
//!
//! The GUI never runs privileged code: it sends a start request with the
//! F5 session cookie to this helper's local socket, and the helper runs
//! openconnect with a route hook that installs only campus routes. On macOS
//! it is a launchd daemon on a root:staff 0660 Unix socket (`unix.rs`); on
//! Windows a LocalSystem service on a named pipe (`windows.rs`). A production
//! build must also verify the peer's code signature.

mod ondemand;
#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(windows)]
mod windows_net;

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use vpn_protocol::{Ending, HelperState, Reply, Request, SessionCookie, HELPER_REVISION};

const DEFAULT_SERVER: &str = "apm.nap.gsic.titech.ac.jp";
/// The only subnet sent through the tunnel; the default route stays put.
const CAMPUS_NET: &str = "131.112.0.0";

#[derive(Clone)]
struct Config {
    socket: PathBuf,
    openconnect: PathBuf,
    script: PathBuf,
    server: String,
    libdirs: Vec<PathBuf>,
    log: PathBuf,
}

struct Inner {
    child: Option<Child>,
    address: Option<String>,
    ifname: Option<String>,
    /// The tunnel's interface index, where openconnect reports one (Windows).
    tun_index: Option<u32>,
    routed: bool,
    error: Option<String>,
    expires: Option<i64>,
    /// Why the helper ended the last tunnel itself; kept past `clear` until
    /// the next start or stop.
    ended: Option<Ending>,
}

impl Inner {
    fn clear(&mut self) {
        self.address = None;
        self.ifname = None;
        self.tun_index = None;
        self.routed = false;
        self.error = None;
        self.expires = None;
    }
}

type State = Arc<Mutex<Inner>>;

/// Notices an openconnect that exited by itself and forgets its tunnel. One
/// that had come up ended (its session, or a network change); one that never
/// did leaves its failure for the next report. True when it exited.
fn reap(st: &mut Inner) -> bool {
    let Some(status) = st.child.as_mut().and_then(|c| c.try_wait().ok()).flatten() else {
        return false;
    };
    let failure = (!st.routed).then(|| format!("openconnect exited: {status}"));
    st.child = None;
    st.clear();
    st.error = failure;
    true
}

/// A fact openconnect printed that the daemon acts on. Everything else is
/// only logged.
#[derive(Debug, PartialEq, Eq)]
enum Observation {
    Address(String),
    /// The route hook's own `route add` output for the campus net (macOS).
    Route(String),
    /// The Wintun adapter openconnect opened, and its index (Windows).
    Device {
        name: String,
        index: u32,
    },
    /// When the server ends the session (Unix seconds).
    Expires(i64),
    Other,
}

fn observe(line: &str) -> Observation {
    if let Some(rest) = line.strip_prefix("Configured as ") {
        let addr = rest.split(',').next().unwrap_or("").trim().to_string();
        return Observation::Address(addr);
    }
    let route_prefix = format!("add net {CAMPUS_NET}: gateway ");
    if let Some(ifname) = line.strip_prefix(&route_prefix).map(str::trim) {
        return Observation::Route(ifname.to_string());
    }
    // `Using Wintun device 'isct-campus', index 23`
    if let Some((name, index)) = line
        .strip_prefix("Using Wintun device '")
        .and_then(|rest| rest.rsplit_once("', index "))
    {
        if let Ok(index) = index.trim().parse() {
            return Observation::Device {
                name: name.to_string(),
                index,
            };
        }
    }
    if let Some(at) = line
        .strip_prefix("Session authentication will expire at ")
        .and_then(utc_seconds)
    {
        return Observation::Expires(at);
    }
    Observation::Other
}

/// Unix seconds of openconnect's `Tue, 03 Nov 2026 08:55:07 UTC`. The helper
/// runs openconnect with `TZ=UTC`, so the zone is always UTC; another zone is
/// not read rather than read wrong.
fn utc_seconds(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [_, day, month, year, time, "UTC"] = parts.as_slice() else {
        return None;
    };
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|name| name == month)? as i64 + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let clock: Vec<i64> = time
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [hour, minute, second] = clock.as_slice() else {
        return None;
    };
    // Days from 1970-01-01 for a proleptic Gregorian date (Hinnant's
    // days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// The state the daemon reports for the facts it holds. A child process that
/// has not installed the route yet is still connecting.
fn state_of(routed: bool, has_child: bool) -> HelperState {
    if routed {
        HelperState::Connected
    } else if has_child {
        HelperState::Connecting
    } else {
        HelperState::Disconnected
    }
}

fn main() {
    platform::main();
}

fn new_state() -> State {
    Arc::new(Mutex::new(Inner {
        child: None,
        address: None,
        ifname: None,
        tun_index: None,
        routed: false,
        error: None,
        ended: None,
        expires: None,
    }))
}

/// openconnect's interface while its tunnel carries the campus routes. Like
/// a status, it first forgets a tunnel whose openconnect has exited.
fn current_tunnel(cfg: &Config, state: &State) -> Option<u32> {
    let mut st = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if reap(&mut st) {
        platform::after_stop(cfg);
    }
    platform::confirm_route(&mut st);
    st.routed.then(|| platform::tunnel_index(&st)).flatten()
}

/// Answers one request line read from `reader` on `writer`.
fn handle(reader: impl Read, mut writer: impl Write, cfg: &Config, state: &State) {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let reply = match Request::from_line(&line) {
        Ok(Request::Start { cookie }) => start(cfg, state, &cookie),
        // It writes its own lines until the app hangs up.
        Ok(Request::Standby { bypass }) => return ondemand::serve(writer, cfg, state, bypass),
        Ok(Request::Stop) => {
            let reply = stop(cfg, state);
            state.lock().unwrap().ended = None;
            reply
        }
        Ok(Request::Status) => status(cfg, state),
        Err(e) => Reply::error(format!("bad request: {e}")),
    };
    let _ = writeln!(
        writer,
        "{}",
        serde_json::to_string(&reply).unwrap_or_default()
    );
}

fn start(cfg: &Config, state: &State, cookie: &SessionCookie) -> Reply {
    let _ = stop(cfg, state);
    {
        let mut st = state.lock().unwrap();
        st.clear();
        st.ended = None;
    }

    let mut cmd = Command::new(&cfg.openconnect);
    cmd.arg("--protocol=f5")
        .arg("--cookie-on-stdin")
        .arg("--non-inter")
        .arg("--script")
        .arg(&cfg.script);
    platform::prepare(&mut cmd, cfg);
    cmd.arg(&cfg.server)
        // Its session-expiry line then names UTC, which `utc_seconds` reads.
        .env("TZ", "UTC")
        // The lines `observe` reads are openconnect's untranslated ones.
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Reply::error(format!("spawn openconnect: {e}")),
    };
    platform::adopt(&child, cfg);
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(cookie.as_str().as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    {
        let mut st = state.lock().unwrap();
        st.child = Some(child);
    }
    if let Some(out) = stdout {
        let s = Arc::clone(state);
        let c = cfg.clone();
        thread::spawn(move || read_stream(out, c, s));
    }
    if let Some(err) = stderr {
        let s = Arc::clone(state);
        let c = cfg.clone();
        thread::spawn(move || read_stream(err, c, s));
    }

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        {
            let mut st = state.lock().unwrap();
            // A stop, or a newer start, took this tunnel away; what happens
            // from here belongs to them.
            if st.child.as_ref().map(Child::id) != Some(pid) {
                return Reply::error("the connection was stopped");
            }
            platform::confirm_route(&mut st);
            if let (Some(addr), true) = (st.address.clone(), st.routed) {
                return Reply {
                    ok: true,
                    state: HelperState::Connected,
                    detail: None,
                    address: Some(addr),
                    ifname: st.ifname.clone(),
                    pid: Some(pid),
                    expires: st.expires,
                    revision: Some(HELPER_REVISION),
                    ended: None,
                };
            }
            if let Some(err) = st.error.clone() {
                return Reply::error(err);
            }
            let exited = st.child.as_mut().and_then(|c| c.try_wait().ok()).flatten();
            if let Some(status) = exited {
                st.child = None;
                st.error = Some(format!("openconnect exited: {status}"));
                return Reply::error(format!("openconnect exited: {status}"));
            }
        }
        if Instant::now() > deadline {
            // Reported as failed, so nothing half up may stay behind it.
            let _ = stop(cfg, state);
            return Reply::error("timeout waiting for openconnect");
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// Calls `each` with every line of `reader`. openconnect prints system error
/// messages in the system's code page, not UTF-8; such a line must not end
/// the reading, or every later line, the reason for a failure included, is
/// lost.
fn each_line(reader: impl Read, mut each: impl FnMut(&str)) {
    let mut reader = BufReader::new(reader);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(_) => each(platform::decode_output(&bytes).trim_end_matches(['\r', '\n'])),
        }
    }
}

fn read_stream<R: Read + Send + 'static>(reader: R, cfg: Config, state: State) {
    each_line(reader, |line| {
        log(&cfg, line);
        match observe(line) {
            Observation::Address(addr) => state.lock().unwrap().address = Some(addr),
            Observation::Route(ifname) => {
                let mut st = state.lock().unwrap();
                st.ifname = Some(ifname);
                st.routed = true;
            }
            Observation::Device { name, index } => {
                let mut st = state.lock().unwrap();
                st.ifname = Some(name);
                st.tun_index = Some(index);
            }
            Observation::Expires(at) => state.lock().unwrap().expires = Some(at),
            Observation::Other => {}
        }
    });
}

fn stop(cfg: &Config, state: &State) -> Reply {
    let child = {
        let mut st = state.lock().unwrap();
        st.child.take()
    };
    if let Some(mut child) = child {
        // Asked to end, openconnect logs out and runs the hook's disconnect.
        platform::terminate(&child, cfg);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }
    platform::after_stop(cfg);
    state.lock().unwrap().clear();
    Reply::disconnected()
}

fn status(cfg: &Config, state: &State) -> Reply {
    let mut st = state.lock().unwrap();
    // A killed openconnect skips its hook's disconnect; what it left behind
    // must not outlive it.
    if reap(&mut st) {
        platform::after_stop(cfg);
    }
    Reply {
        ok: true,
        state: state_of(st.routed, st.child.is_some()),
        detail: st.error.clone(),
        address: st.address.clone(),
        ifname: st.ifname.clone(),
        pid: st.child.as_ref().map(|c| c.id()),
        expires: st.expires.filter(|_| st.routed),
        revision: Some(HELPER_REVISION),
        ended: st.ended,
    }
}

fn log(cfg: &Config, msg: &str) {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // One write, so the stdout and stderr readers do not interleave a line.
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&cfg.log) {
        let _ = f.write_all(format!("{ts} {msg}\n").as_bytes());
    }
}

/// The daemon's arguments over the platform's installed defaults.
fn parse_args(args: impl IntoIterator<Item = String>) -> Config {
    let mut cfg = platform::defaults();
    let args: Vec<String> = args.into_iter().collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" if i + 1 < args.len() => {
                i += 1;
                cfg.socket = args[i].clone().into();
            }
            "--openconnect" if i + 1 < args.len() => {
                i += 1;
                cfg.openconnect = args[i].clone().into();
            }
            "--script" if i + 1 < args.len() => {
                i += 1;
                cfg.script = args[i].clone().into();
            }
            "--server" if i + 1 < args.len() => {
                i += 1;
                cfg.server = args[i].clone();
            }
            "--libdir" if i + 1 < args.len() => {
                i += 1;
                cfg.libdirs.push(args[i].clone().into());
            }
            "--log" if i + 1 < args.len() => {
                i += 1;
                cfg.log = args[i].clone().into();
            }
            _ => {}
        }
        i += 1;
    }
    cfg
}

#[cfg(test)]
#[path = "tests/helper.rs"]
mod tests;
