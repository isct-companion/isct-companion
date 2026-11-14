use super::*;

/// The daemon acts only on these openconnect lines. A change in wording
/// upstream must show up here, not as a silently never-connected tunnel.
#[test]
fn only_the_address_device_and_campus_route_lines_are_acted_on() {
    assert_eq!(
        observe("Configured as 10.1.2.3, with SSL connected"),
        Observation::Address("10.1.2.3".into())
    );
    assert_eq!(
        observe("add net 131.112.0.0: gateway utun7"),
        Observation::Route("utun7".into())
    );
    // The default route and any other subnet never mark the tunnel as up.
    assert_eq!(
        observe("add net 0.0.0.0: gateway utun7"),
        Observation::Other
    );
    assert_eq!(observe("add net 131.112.0.0: gateway"), Observation::Other);
    // On Windows the route is read from the table on the adapter named here.
    assert_eq!(
        observe("Using Wintun device 'isct-campus', index 23"),
        Observation::Device {
            name: "isct-campus".into(),
            index: 23
        }
    );
    assert_eq!(
        observe("Using Wintun device 'isct-campus', index none"),
        Observation::Other
    );
}

/// A line in the system code page (here GBK, as on a Chinese Windows) once
/// stopped the reader, so the Wintun line after it and the failure's reason
/// never arrived.
#[test]
fn a_line_that_is_not_utf8_does_not_end_the_reading() {
    let output: &[u8] = b"Set up UDP failed\r\n\xD6\xD0\xCE\xC4 error\nUsing Wintun device 'isct-campus', index 49\n";
    let mut lines = Vec::new();
    each_line(output, |line| lines.push(line.to_string()));
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "Set up UDP failed");
    assert!(lines[1].ends_with(" error"));
    assert_eq!(
        observe(&lines[2]),
        Observation::Device {
            name: "isct-campus".into(),
            index: 49
        }
    );
}

#[test]
fn state_follows_the_route_then_the_child_process() {
    assert_eq!(state_of(true, true), HelperState::Connected);
    assert_eq!(state_of(false, true), HelperState::Connecting);
    assert_eq!(state_of(false, false), HelperState::Disconnected);
}

/// The session end openconnect prints is what the countdown and the ending
/// reminder rest on; the helper runs it with TZ=UTC so the zone is fixed.
#[test]
fn the_session_end_is_read_from_openconnects_utc_line() {
    // 2026-11-03 08:55:07 UTC, the 17:55:07 JST a real session reported.
    assert_eq!(
        observe("Session authentication will expire at Tue, 03 Nov 2026 08:55:07 UTC"),
        Observation::Expires(1_793_696_107)
    );
    assert_eq!(utc_seconds("Thu, 01 Jan 1970 00:00:00 UTC"), Some(0));
    assert_eq!(
        utc_seconds("Thu, 29 Feb 2024 23:59:59 UTC"),
        Some(1_709_251_199)
    );
    // Another zone is not guessed at.
    assert_eq!(
        observe("Session authentication will expire at Tue, 03 Nov 2026 17:55:07 JST"),
        Observation::Other
    );
}

#[cfg(unix)]
fn exiting_child() -> std::process::Child {
    Command::new("/usr/bin/true").spawn().unwrap()
}

#[cfg(windows)]
fn exiting_child() -> std::process::Child {
    Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
}

fn inner_with_exited_child(routed: bool) -> Inner {
    let mut child = exiting_child();
    child.wait().unwrap();
    Inner {
        child: Some(child),
        address: Some("10.1.2.3".into()),
        ifname: Some("utun7".into()),
        tun_index: None,
        routed,
        error: None,
        expires: Some(1),
        ended: None,
    }
}

/// openconnect exits on its own when the session or the network ends. The
/// helper must stop reporting the tunnel, or the app shows it connected and
/// never announces the end; that is an ending, not a failure.
#[test]
fn a_tunnel_whose_openconnect_exited_is_reported_ended() {
    let mut st = inner_with_exited_child(true);
    assert!(reap(&mut st));
    assert_eq!(
        state_of(st.routed, st.child.is_some()),
        HelperState::Disconnected
    );
    assert_eq!((st.address, st.expires, st.error), (None, None, None));
}

/// One that never came up failed, and the reason stays for the next report.
#[test]
fn an_openconnect_that_never_came_up_leaves_its_failure() {
    let mut st = inner_with_exited_child(false);
    assert!(reap(&mut st));
    assert!(st
        .error
        .is_some_and(|error| error.contains("openconnect exited")));
    let mut idle = Inner {
        child: None,
        address: None,
        ifname: None,
        tun_index: None,
        routed: false,
        error: None,
        expires: None,
        ended: None,
    };
    assert!(!reap(&mut idle));
}

/// The route hook run for real over stub `route`, `netstat`, `scutil`,
/// `ifconfig` and `logger`, which record their calls; the routes are the ones
/// a Mac had on a phone hotspot behind a Tailscale exit node.
#[cfg(unix)]
fn hook(reason: &str, name: &str) -> String {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("isct-hook-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let calls = dir.join("calls");
    fs::write(&calls, "").unwrap();
    for tool in ["route", "scutil", "ifconfig", "logger"] {
        // Only scutil is fed its commands on stdin.
        let input = if tool == "scutil" { "cat" } else { "true" };
        let stub = dir.join(tool);
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\n{{ echo \"{tool} $*\"; {input}; }} >> '{}'\n",
                calls.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let netstat = dir.join("netstat");
    fs::write(
        &netstat,
        "#!/bin/sh\ncat <<'ROUTES'\nDestination Gateway Flags Netif\n\
         default link#48 UCSg utun14\n\
         default 172.20.10.1 UGScIg en0\n\
         default link#33 UCSIg bridge100 !\n\
         ROUTES\n",
    )
    .unwrap();
    fs::set_permissions(&netstat, fs::Permissions::from_mode(0o755)).unwrap();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vpnc-script");
    let status = Command::new("/bin/sh")
        .arg(script)
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .env("reason", reason)
        .env("TUNDEV", "utun13")
        .env("INTERNAL_IP4_ADDRESS", "131.112.9.9")
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let recorded = fs::read_to_string(&calls).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    recorded
}

/// A network change must re-pin the APM to the gateway of the network the Mac
/// is on now, past a Tailscale default route, or openconnect can never reach
/// it again; and the campus resolvers go until the tunnel is back, or campus
/// names, the APM's included, time out for a new sign-in too.
#[cfg(unix)]
#[test]
fn a_reconnect_attempt_repins_the_apm_and_withdraws_the_campus_resolvers() {
    let calls = hook("attempt-reconnect", "attempt");
    assert!(
        calls.contains("route delete -host 131.112.12.180"),
        "{calls}"
    );
    assert!(
        calls.contains("route add -host 131.112.12.180 172.20.10.1"),
        "{calls}"
    );
    assert!(
        calls.contains("remove State:/Network/Service/isct-campus-vpn/DNS"),
        "{calls}"
    );
    assert!(
        !calls.contains("-net 131.112.0.0/16"),
        "the tunnel's route stays: {calls}"
    );
    assert!(hook("reconnect", "back").contains("d.add ServerAddresses"));
}

/// Nothing outside the tunnel outlives it.
#[cfg(unix)]
#[test]
fn a_disconnect_removes_the_pin_the_route_and_the_resolvers() {
    let calls = hook("disconnect", "down");
    assert!(
        calls.contains("route delete -net 131.112.0.0/16"),
        "{calls}"
    );
    assert!(
        calls.contains("route delete -host 131.112.12.180"),
        "{calls}"
    );
    assert!(calls.contains("remove State:/Network"), "{calls}");
}
