use super::*;
use std::collections::VecDeque;

/// Replays scripted helper answers, one per helper call; a call with none
/// left fails the test.
#[derive(Default)]
struct FakePort {
    answers: Mutex<VecDeque<Result<HelperReport, VpnError>>>,
    /// The publisher addresses the last `routes` call handed over.
    routes: Mutex<(Vec<std::net::Ipv4Addr>, Vec<std::net::Ipv6Addr>)>,
}

impl FakePort {
    fn answering(answers: Vec<Result<HelperReport, VpnError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            routes: Mutex::default(),
        })
    }

    fn next(&self) -> Result<HelperReport, VpnError> {
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("unscripted helper call")
    }
}

impl VpnPort for FakePort {
    fn status(&self) -> Result<HelperReport, VpnError> {
        self.next()
    }
    fn start(&self, _cookie: &SessionCookie) -> Result<HelperReport, VpnError> {
        self.next()
    }
    fn stop(&self) -> Result<HelperReport, VpnError> {
        self.next()
    }
    fn routes(
        &self,
        v4: &[std::net::Ipv4Addr],
        v6: &[std::net::Ipv6Addr],
    ) -> Result<HelperReport, VpnError> {
        *self.routes.lock().unwrap() = (v4.to_vec(), v6.to_vec());
        self.next()
    }
}

fn report(state: HelperState) -> HelperReport {
    HelperReport {
        state,
        address: None,
        detail: None,
        expires_at: None,
        revision: HELPER_REVISION,
        ended: None,
        program: None,
    }
}

fn cookie() -> SessionCookie {
    SessionCookie::new("MRHSession=abc; F5_ST=def").unwrap()
}

/// A helper of an earlier revision is still installed after an app update.
/// With a tunnel up it is left alone; with none, the person is asked to
/// update it before the next connect, which the countdown needs.
#[test]
fn an_old_helper_is_updated_only_when_no_tunnel_is_up() {
    let old = |state| HelperReport {
        revision: HELPER_REVISION - 1,
        ..report(state)
    };
    assert!(matches!(
        project(old(HelperState::Connected), None),
        VpnState::Connected { .. }
    ));
    assert_eq!(
        project(old(HelperState::Disconnected), None),
        VpnState::Error(VpnError::HelperOutdated)
    );
    assert_eq!(VpnError::HelperOutdated.code(), "helper_outdated");
}

/// launchd runs a macOS helper inside whichever copy of the app registered
/// it. A helper from another copy (a build folder, a disk image) is not
/// updated with this app and stops for good once that copy is deleted, so
/// with no tunnel up it is registered again from this app; a tunnel it has
/// up is left alone, and a helper that does not say where it runs, or a
/// shell with no own helper to compare (Windows), is taken as is.
#[test]
fn a_helper_from_another_copy_of_the_app_is_replaced_when_no_tunnel_is_up() {
    let own = PathBuf::from(
        "/Applications/IsctCompanion.app/Contents/Resources/resources/vpn/isct-vpn-helper",
    );
    let other = "/Users/me/IsctApp/target/debug/bundle/macos/IsctCompanion.app/Contents/Resources/resources/vpn/isct-vpn-helper";
    let from = |state, program: Option<&str>| HelperReport {
        program: program.map(PathBuf::from),
        ..report(state)
    };
    let service = VpnService::new(FakePort::answering(vec![
        Ok(from(HelperState::Disconnected, Some(other))),
        Ok(from(HelperState::Connected, Some(other))),
        Ok(from(HelperState::Disconnected, own.to_str())),
        Ok(from(HelperState::Disconnected, None)),
    ]))
    .with_own_helper(own.clone());
    assert_eq!(
        service.refresh(),
        VpnState::Error(VpnError::HelperElsewhere(other.into()))
    );
    assert!(matches!(service.refresh(), VpnState::Connected { .. }));
    assert_eq!(service.refresh(), VpnState::Disconnected);
    assert_eq!(service.refresh(), VpnState::Disconnected);
    assert_eq!(
        project(from(HelperState::Disconnected, Some(other)), None),
        VpnState::Disconnected
    );
    // Reconnecting cannot fix either, so on-demand stops on them; installing
    // again does, so the card offers it.
    for error in [
        VpnError::HelperElsewhere(other.into()),
        VpnError::HelperNotStarting,
    ] {
        assert!(hopeless(&error));
        assert!(error.install_fixes());
    }
    assert!(!VpnError::HelperNeedsApproval.install_fixes());
    // Installing again is what left it stuck.
    assert!(!VpnError::HelperRegistrationStuck.install_fixes());
}

/// One tunnel through the helper. A press connects and keeps the tunnel's
/// address; a second press while it is up spends no new one-time session
/// (there is no second `start` answer to give). Refresh and disconnect
/// follow the helper. A start that failed leaves its reason behind, and
/// reading that as a clean disconnect would hide the failure. A helper that
/// does not answer, a missing one (so the card can offer the install step)
/// and a login that failed before the helper are each the state the person
/// sees, the login keeping its own code for the guidance.
#[test]
fn a_tunnel_follows_the_helper_through_connect_refresh_and_disconnect() {
    let up = VpnState::Connected {
        address: Some("10.1.2.3".into()),
        expires_at: None,
    };
    let service = VpnService::new(FakePort::answering(vec![
        Ok(HelperReport {
            address: Some("10.1.2.3".into()),
            ..report(HelperState::Connected)
        }),
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Disconnected)),
        Ok(HelperReport {
            detail: Some("openconnect exited".into()),
            ..report(HelperState::Disconnected)
        }),
        Err(VpnError::Timeout),
        Err(VpnError::HelperNotInstalled),
    ]));
    assert_eq!(service.connect(&cookie()), up);
    assert_eq!(service.connect(&cookie()), up);
    assert!(matches!(service.refresh(), VpnState::Connected { .. }));
    assert_eq!(service.disconnect(), VpnState::Disconnected);
    assert_eq!(
        service.refresh(),
        VpnState::Error(VpnError::Tunnel("openconnect exited".into()))
    );
    assert_eq!(service.refresh(), VpnState::Error(VpnError::Timeout));
    assert_eq!(
        service.connect(&cookie()),
        VpnState::Error(VpnError::HelperNotInstalled)
    );
    assert_eq!(
        service.state(),
        VpnState::Error(VpnError::HelperNotInstalled)
    );

    let login = CampusVpnError::NeedsAccount;
    let VpnState::Error(error) = service.failed(VpnError::Login(login.clone())) else {
        panic!("not an error state");
    };
    assert_eq!(error.code(), login.code());
    assert_eq!(service.state(), VpnState::Error(error));
}

fn connected_until(end: DateTime<Utc>) -> HelperReport {
    HelperReport {
        expires_at: Some(end),
        ..report(HelperState::Connected)
    }
}

fn at(seconds: i64) -> DateTime<Utc> {
    use chrono::TimeZone;
    Utc.timestamp_opt(seconds, 0).single().unwrap()
}

/// The ending reminder comes once, inside the last ten minutes, with the
/// minutes left rounded up; not before, not again on the next check.
#[test]
fn the_ending_reminder_comes_once_in_the_last_ten_minutes() {
    let end = at(1_793_696_107);
    let port = FakePort::answering(vec![
        Ok(connected_until(end)),
        Ok(connected_until(end)),
        Ok(connected_until(end)),
    ]);
    let service = VpnService::new(port);
    let minutes = |m: i64| end - chrono::Duration::minutes(m);
    assert_eq!(service.watch(minutes(11)).notice, None);
    let check = service.watch(minutes(10) + chrono::Duration::seconds(30));
    assert_eq!(check.notice, Some(VpnNotice::EndingSoon { minutes: 10 }));
    assert_eq!(
        check.state,
        VpnState::Connected {
            address: None,
            expires_at: Some(end)
        }
    );
    assert_eq!(service.watch(minutes(5)).notice, None);
}

/// A tunnel that drops on its own is announced; one the person disconnected
/// is not, and a helper that merely did not answer is not an ending.
#[test]
fn an_ending_is_announced_only_when_the_tunnel_ends_by_itself() {
    let now = at(1_793_690_000);
    let dropped = VpnService::new(FakePort::answering(vec![
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Disconnected)),
    ]));
    dropped.refresh();
    assert_eq!(dropped.watch(now).notice, Some(VpnNotice::Ended));

    let disconnected = VpnService::new(FakePort::answering(vec![
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Disconnected)),
        Ok(report(HelperState::Disconnected)),
    ]));
    disconnected.refresh();
    disconnected.disconnect();
    assert_eq!(disconnected.watch(now).notice, None);

    let silent = VpnService::new(FakePort::answering(vec![
        Ok(report(HelperState::Connected)),
        Err(VpnError::Timeout),
    ]));
    silent.refresh();
    assert_eq!(silent.watch(now).notice, None);

    // A forgotten SSH the helper cut is told even with on-demand on, where a
    // plain ending is not news.
    let unused = VpnService::new(FakePort::answering(vec![
        Ok(report(HelperState::Connected)),
        Ok(HelperReport {
            ended: Some(vpn_protocol::Ending::Unused),
            ..report(HelperState::Disconnected)
        }),
    ]));
    unused.refresh();
    unused.set_on_demand(true);
    assert_eq!(unused.watch(now).notice, Some(VpnNotice::Unused));
}

fn minutes_after(start: DateTime<Utc>, minutes: i64) -> DateTime<Utc> {
    start + chrono::Duration::minutes(minutes)
}

/// A standby demand signs in only while on-demand is on and no tunnel is up,
/// waits longer after each failure, and turns itself off on what retrying
/// cannot fix: each demand would otherwise post a refused password again.
/// Being offline only waits: no password was accepted, so it never stops.
#[test]
fn demands_sign_in_with_growing_waits_and_stop_on_what_cannot_work() {
    let start = at(1_793_690_000);
    let service = VpnService::new(FakePort::answering(vec![Ok(report(
        HelperState::Connected,
    ))]));
    assert!(!service.demanded(start), "off by default");
    service.set_on_demand(true);
    assert!(service.demanded(start));

    let offline = VpnState::Error(VpnError::Login(CampusVpnError::Unreachable("x".into())));
    let mut at_minute = 0;
    for wait in [1, 2, 5, 5] {
        assert_eq!(
            service.demand_finished(&offline, minutes_after(start, at_minute)),
            None
        );
        assert!(!service.demanded(minutes_after(start, at_minute + wait - 1)));
        at_minute += wait;
        assert!(service.demanded(minutes_after(start, at_minute)));
    }
    for minute in 0..20 {
        assert_eq!(
            service.demand_finished(&offline, minutes_after(start, 100 + minute)),
            None
        );
    }
    assert!(service.on_demand(), "offline never gives up");

    assert!(matches!(
        service.connect(&cookie()),
        VpnState::Connected { .. }
    ));
    assert!(!service.demanded(minutes_after(start, 300)), "already up");

    let refused = VpnState::Error(VpnError::Login(CampusVpnError::Rejected));
    assert_eq!(
        service.demand_finished(&refused, start),
        Some(VpnNotice::DemandStopped {
            code: "vpn_login_rejected"
        })
    );
    assert!(!service.on_demand());

    // The sign-in itself must never wait on standby.
    let bypass: Vec<_> = VpnService::standby_bypass_hosts().collect();
    for host in [
        f5::CAMPUS_APM_HOST,
        "portal.nap.gsic.titech.ac.jp",
        "rp.nap.gsic.titech.ac.jp",
    ] {
        assert!(bypass.contains(&host), "{host}");
    }
}

/// Logins the school accepts but whose tunnel keeps dying end on-demand, so
/// a broken tunnel does not post the password on every connection. A tunnel
/// up long enough clears that count, and its ending needs no notice: the
/// next use brings it back.
#[test]
fn logins_that_never_last_give_up_and_lasting_ones_reset_the_count() {
    let start = at(1_793_690_000);
    let up = VpnState::Connected {
        address: None,
        expires_at: None,
    };
    let never_lasting = VpnService::new(FakePort::answering(vec![]));
    never_lasting.set_on_demand(true);
    for _ in 1..MAX_SPENT {
        assert_eq!(never_lasting.demand_finished(&up, start), None);
    }
    assert_eq!(
        never_lasting.demand_finished(&up, start),
        Some(VpnNotice::DemandStopped {
            code: "tunnel_failed"
        })
    );

    let service = VpnService::new(FakePort::answering(vec![
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Disconnected)),
    ]));
    service.refresh();
    service.set_on_demand(true);
    for minute in 0..i64::from(MAX_SPENT) - 1 {
        assert_eq!(
            service.demand_finished(&up, minutes_after(start, minute)),
            None
        );
    }
    for minute in [0, STABLE_MINUTES] {
        service.watch(minutes_after(start, 10 + minute));
    }
    assert_eq!(service.demand_finished(&up, minutes_after(start, 20)), None);
    assert!(service.on_demand(), "a lasting tunnel restarted the count");
    let ended = service.watch(minutes_after(start, 40));
    assert_eq!(ended.state, VpnState::Disconnected);
    assert_eq!(ended.notice, None);
}

/// Something touching the campus network every few minutes must not sign in
/// each time: on-demand signs in at most six times an hour by itself, says
/// until when it holds back, and a press is never held back.
#[test]
fn demands_sign_in_at_most_six_times_an_hour() {
    let start = at(1_793_690_000);
    let service = VpnService::new(FakePort::answering(vec![Ok(report(
        HelperState::Connected,
    ))]));
    service.set_on_demand(true);
    let offline = VpnState::Error(VpnError::Login(CampusVpnError::Unreachable("x".into())));
    let mut signed_in = vec![];
    for minute in 0..60 {
        let now = minutes_after(start, minute);
        if service.demanded(now) {
            signed_in.push(minute);
            service.demand_finished(&offline, now);
        }
    }
    assert_eq!(signed_in, [0, 1, 3, 8, 13, 18]);
    assert_eq!(
        service.demand_paused_until(minutes_after(start, 30)),
        Some(minutes_after(start, 60))
    );
    assert!(service.demanded(minutes_after(start, 60)));
    // The hour slides: the next frees up when the second one turns an hour.
    assert_eq!(
        service.demand_paused_until(minutes_after(start, 60)),
        Some(minutes_after(start, 61))
    );
    assert!(service.on_demand(), "holding back is not giving up");
    assert!(matches!(
        service.connect(&cookie()),
        VpnState::Connected { .. }
    ));
}

/// On the campus network campus hosts are reached directly, so automatic
/// management neither holds standby nor signs in, and picks up again once the
/// device leaves. A tunnel's own campus address, which appears before Core
/// knows it, must not read as the campus network.
#[test]
fn automatic_management_stands_down_on_the_campus_network() {
    let start = at(1_793_690_000);
    let science_tokyo_x: std::net::IpAddr = "2403:4440:40:61::5".parse().unwrap();
    let home: std::net::IpAddr = "192.168.1.20".parse().unwrap();
    let service = VpnService::new(FakePort::answering(vec![Ok(report(
        HelperState::Connected,
    ))]));
    service.set_on_demand(true);
    assert!(service.network([science_tokyo_x]));
    assert!(!service.wants_standby());
    assert!(!service.demanded(start));
    assert!(!service.network([home]));
    assert!(service.wants_standby());

    // Mid-sign-in, the tunnel's address shows up first.
    let login = service.begin_login();
    assert!(!service.network([home, "131.112.10.19".parse().unwrap()]));
    drop(login);
    service.connect(&cookie());
    assert!(!service.network([home, "131.112.10.19".parse().unwrap()]));
}

/// A press and a demand must not each spend a one-time session.
#[test]
fn logins_are_one_at_a_time() {
    let service = VpnService::new(FakePort::answering(vec![]));
    let first = service.begin_login();
    assert!(first.is_some());
    assert!(service.begin_login().is_none());
    drop(first);
    assert!(service.begin_login().is_some());
}

/// What a person pastes into the publisher list becomes the host to route:
/// a full article address must not be stored as a host that never resolves,
/// and junk must not reach the helper's routes.
#[test]
fn publishers_are_normalized_and_the_default_list_comes_back() {
    for (input, host) in [
        (
            "https://www.Nature.com/articles/s41586?x=1",
            "www.nature.com",
        ),
        ("  link.springer.com/  ", "link.springer.com"),
        ("http://user@pubs.acs.org:443/doi", "pubs.acs.org"),
        ("ieeexplore.ieee.org.", "ieeexplore.ieee.org"),
    ] {
        assert_eq!(normalize_publisher(input).as_deref(), Some(host), "{input}");
    }
    for junk in [
        "",
        "localhost",
        "exa mple.com",
        "-bad.com",
        "a..b",
        "a_b.com",
    ] {
        assert_eq!(normalize_publisher(junk), None, "{junk}");
    }
    assert_eq!(
        VpnService::publisher_names("nature.com"),
        ["nature.com", "www.nature.com"]
    );
    assert_eq!(
        VpnService::publisher_names("link.springer.com"),
        ["link.springer.com"]
    );

    let service = VpnService::new(FakePort::answering(vec![]));
    assert!(service.publishers_are_default());
    assert_eq!(service.publishers().len(), DEFAULT_PUBLISHERS.len());
    service.set_publishers(Some(vec![
        "nature.com".into(),
        "https://nature.com/x".into(),
        "not a host".into(),
    ]));
    assert_eq!(service.publishers(), ["nature.com"]);
    assert!(!service.publishers_are_default());
    service.set_publishers(None);
    assert!(service.publishers_are_default());
}

/// The helper gets each publisher's addresses under both its names, each
/// once; a campus address is routed already and a name that does not
/// resolve sends nothing.
#[test]
fn publisher_routes_carry_every_name_once() {
    let port = FakePort::answering(vec![Ok(report(HelperState::Disconnected))]);
    let service = VpnService::new(port.clone());
    service.set_publishers(Some(vec!["nature.com".into(), "gone.example".into()]));
    let shared: std::net::IpAddr = "162.159.135.42".parse().unwrap();
    service
        .send_publisher_routes(|name| match name {
            "nature.com" => vec![shared, "131.112.1.1".parse().unwrap()],
            "www.nature.com" => vec![shared, "2606:4700::6810:1".parse().unwrap()],
            _ => vec![],
        })
        .unwrap();
    let (v4, v6) = port.routes.lock().unwrap().clone();
    assert_eq!(
        v4,
        ["162.159.135.42".parse::<std::net::Ipv4Addr>().unwrap()]
    );
    assert_eq!(
        v6,
        ["2606:4700::6810:1".parse::<std::net::Ipv6Addr>().unwrap()]
    );
}
