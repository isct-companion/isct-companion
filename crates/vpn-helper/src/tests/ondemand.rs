use super::*;

/// Standby must cover every campus address except the bypassed ones: a
/// bypassed sign-in host caught by standby would wait on the very tunnel its
/// sign-in is meant to start, and a gap would send campus traffic past it.
#[test]
fn standby_routes_cover_campus_except_the_bypass() {
    let apm = Ipv4Addr::new(131, 112, 12, 180);
    let portal = Ipv4Addr::new(131, 112, 12, 30);
    let outside = Ipv4Addr::new(18, 180, 118, 233);
    let prefixes = standby_prefixes(&[apm, portal, outside, apm]);
    let covers = |host: Ipv4Addr| {
        prefixes
            .iter()
            .filter(|(net, len)| {
                let mask = u32::MAX << (32 - u32::from(*len));
                u32::from(host) & mask == u32::from(*net)
            })
            .count()
    };
    assert_eq!(covers(apm), 0);
    assert_eq!(covers(portal), 0);
    for host in [
        Ipv4Addr::new(131, 112, 0, 0),
        Ipv4Addr::new(131, 112, 12, 181),
        Ipv4Addr::new(131, 112, 12, 31),
        Ipv4Addr::new(131, 112, 23, 78),
        Ipv4Addr::new(131, 112, 255, 255),
    ] {
        assert_eq!(covers(host), 1, "{host}");
    }
    let size: u64 = prefixes.iter().map(|(_, len)| 1u64 << (32 - len)).sum();
    assert_eq!(size, (1 << 16) - 2);
    // Nothing to bypass is the whole campus in one route.
    assert_eq!(standby_prefixes(&[outside]), vec![CAMPUS]);
}

const MINUTE: Duration = Duration::from_secs(60);

/// Feeds `idleness` a reading every 15 seconds for `minutes`, carrying
/// `per_minute` bytes and `open` connections; the minute it ended, if it did.
fn run(
    idleness: &mut Idleness,
    start: Instant,
    pid: u32,
    minutes: u64,
    per_minute: u64,
    open: bool,
) -> Option<(u64, Ending)> {
    let mut bytes = 0;
    for tick in 0..=minutes * 4 {
        let reading = Reading { pid, bytes, open };
        if let Some(ending) = idleness.observe(start + IDLE_CHECK * tick as u32, Some(reading)) {
            return Some((tick / 4, ending));
        }
        bytes += per_minute / 4;
    }
    None
}

/// The school's ports are few: a tunnel nobody uses ends after the idle
/// time, an open connection (an SSH sitting at its prompt) keeps it, and
/// keepalives alone do not keep one for longer than half an hour.
#[test]
fn a_tunnel_ends_when_unused_and_open_connections_keep_it_until_half_an_hour() {
    let start = Instant::now();
    let mut idleness = Idleness::default();
    assert_eq!(
        run(&mut idleness, start, 1, 60, 0, false),
        Some((5, Ending::Idle))
    );

    // An SSH at its prompt with ServerAliveInterval: keepalives only.
    let mut idleness = Idleness::default();
    assert_eq!(
        run(&mut idleness, start, 1, 60, 200, true),
        Some((30, Ending::Unused))
    );

    // Real use keeps any tunnel, connections or not.
    let mut idleness = Idleness::default();
    assert_eq!(run(&mut idleness, start, 1, 120, 40 * 1024, false), None);

    // No tunnel resets the watch, and a reading never ends anything twice.
    assert_eq!(idleness.observe(start, None), None);
}

/// Someone who keeps coming back soon after an idle ending is not signed in
/// over and over: the idle time grows 5, 10, 15 minutes, and a long pause or
/// a forgotten tunnel brings it back to 5.
#[test]
fn the_idle_time_grows_when_a_tunnel_is_wanted_again_soon() {
    let mut idleness = Idleness::default();
    let mut now = Instant::now();
    let idle_after = |idleness: &mut Idleness, now: &mut Instant, pid, gap: u64| {
        *now += MINUTE * gap as u32;
        let (minutes, ending) = run(idleness, *now, pid, 60, 0, false).unwrap();
        assert_eq!(ending, Ending::Idle);
        *now += MINUTE * minutes as u32;
        minutes
    };
    assert_eq!(idle_after(&mut idleness, &mut now, 1, 0), 5);
    assert_eq!(idle_after(&mut idleness, &mut now, 2, 3), 10);
    assert_eq!(idle_after(&mut idleness, &mut now, 3, 3), 15);
    assert_eq!(idle_after(&mut idleness, &mut now, 4, 9), 15);
    assert_eq!(idle_after(&mut idleness, &mut now, 5, 11), 5);

    // Ended as forgotten, the next one starts at the first step again.
    assert_eq!(idle_after(&mut idleness, &mut now, 6, 1), 10);
    now += MINUTE;
    assert_eq!(
        run(&mut idleness, now, 7, 60, 0, true).map(|(_, ending)| ending),
        Some(Ending::Unused)
    );
    now += 31 * MINUTE;
    assert_eq!(idle_after(&mut idleness, &mut now, 8, 1), 5);
}

/// Only an established connection from the tunnel's own address counts: the
/// app's sign-in and refreshes leave from the physical interface.
#[cfg(target_os = "macos")]
#[test]
fn netstat_counts_established_connections_from_the_tunnel_address() {
    let tunnel = Ipv4Addr::new(10, 7, 0, 12);
    let table = "\
Active Internet connections (including servers)
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)
tcp4       0      0  192.168.1.20.56308     131.112.12.180.443     ESTABLISHED
tcp4       0      0  10.7.0.12.52011        131.112.125.5.22       TIME_WAIT
tcp4       0      0  10.7.0.120.52012       131.112.125.5.22       ESTABLISHED
";
    assert!(!platform::established_from(table, tunnel));
    let open = format!(
        "{table}tcp4       0      0  10.7.0.12.52013        131.112.125.5.22       ESTABLISHED\n"
    );
    assert!(platform::established_from(&open, tunnel));
}
