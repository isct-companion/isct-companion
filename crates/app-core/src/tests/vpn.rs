use super::*;
use std::collections::VecDeque;

/// Replays scripted helper answers and records what the service asked for.
#[derive(Default)]
struct FakePort {
    answers: Mutex<VecDeque<Result<HelperReport, VpnError>>>,
    calls: Mutex<Vec<&'static str>>,
}

impl FakePort {
    fn answering(answers: Vec<Result<HelperReport, VpnError>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn next(&self, call: &'static str) -> Result<HelperReport, VpnError> {
        self.calls.lock().unwrap().push(call);
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("unscripted helper call")
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

impl VpnPort for FakePort {
    fn status(&self) -> Result<HelperReport, VpnError> {
        self.next("status")
    }
    fn start(&self, _cookie: &SessionCookie) -> Result<HelperReport, VpnError> {
        self.next("start")
    }
    fn stop(&self) -> Result<HelperReport, VpnError> {
        self.next("stop")
    }
}

fn report(state: HelperState) -> HelperReport {
    HelperReport {
        state,
        address: None,
        detail: None,
        expires_at: None,
        revision: Some(HELPER_REVISION),
        ended: None,
    }
}

fn cookie() -> SessionCookie {
    SessionCookie::new("MRHSession=abc; F5_ST=def").unwrap()
}

/// The helper installed by the first test build reports no revision. With a
/// tunnel up it is left alone; with none, the person is asked to update it
/// before the next connect, which the countdown needs.
#[test]
fn an_old_helper_is_updated_only_when_no_tunnel_is_up() {
    let old = |state| HelperReport {
        revision: None,
        ..report(state)
    };
    assert!(matches!(
        project(old(HelperState::Connected)),
        VpnState::Connected { .. }
    ));
    assert_eq!(
        project(old(HelperState::Disconnected)),
        VpnState::Error(VpnError::HelperOutdated)
    );
    assert_eq!(VpnError::HelperOutdated.code(), "helper_outdated");
}

/// The UI keys its guidance on these names; a rename breaks the install prompt.
#[test]
fn error_codes_are_stable_names() {
    assert_eq!(VpnError::HelperNotInstalled.code(), "helper_not_installed");
    assert_eq!(VpnError::Timeout.code(), "helper_timeout");
    assert_eq!(VpnError::Tunnel("x".into()).code(), "tunnel_failed");
}

#[test]
fn a_connected_report_keeps_the_tunnel_address() {
    let next = project(HelperReport {
        state: HelperState::Connected,
        address: Some("10.1.2.3".into()),
        detail: None,
        expires_at: None,
        revision: Some(HELPER_REVISION),
        ended: None,
    });
    assert_eq!(
        next,
        VpnState::Connected {
            address: Some("10.1.2.3".into()),
            expires_at: None,
        }
    );
}

/// A failed start leaves its reason behind in the helper; reading it as a
/// clean disconnect would hide the failure from the person.
#[test]
fn a_disconnected_report_with_a_detail_is_a_failure() {
    let failed = project(HelperReport {
        state: HelperState::Disconnected,
        address: None,
        detail: Some("openconnect exited".into()),
        expires_at: None,
        revision: Some(HELPER_REVISION),
        ended: None,
    });
    assert_eq!(
        failed,
        VpnState::Error(VpnError::Tunnel("openconnect exited".into()))
    );
    assert_eq!(
        project(report(HelperState::Disconnected)),
        VpnState::Disconnected
    );
}

#[test]
fn connect_goes_connecting_then_connected() {
    let port = FakePort::answering(vec![Ok(HelperReport {
        state: HelperState::Connected,
        address: Some("10.1.2.3".into()),
        detail: None,
        expires_at: None,
        revision: Some(HELPER_REVISION),
        ended: None,
    })]);
    let service = VpnService::new(port.clone());
    assert_eq!(
        service.connect(&cookie()),
        VpnState::Connected {
            address: Some("10.1.2.3".into()),
            expires_at: None,
        }
    );
    assert_eq!(port.calls(), vec!["start"]);
}

/// A missing helper must surface as its own state so the UI can offer the
/// install step instead of a generic failure.
#[test]
fn a_missing_helper_is_reported_as_not_installed() {
    let port = FakePort::answering(vec![Err(VpnError::HelperNotInstalled)]);
    let service = VpnService::new(port);
    assert_eq!(
        service.connect(&cookie()),
        VpnState::Error(VpnError::HelperNotInstalled)
    );
    assert_eq!(
        service.state(),
        VpnState::Error(VpnError::HelperNotInstalled)
    );
}

/// Each cookie is one-time: a second press while the tunnel is up must not
/// spend a new login or start a second openconnect.
#[test]
fn connecting_while_connected_does_not_start_again() {
    let port = FakePort::answering(vec![Ok(report(HelperState::Connected))]);
    let service = VpnService::new(port.clone());
    service.connect(&cookie());
    service.connect(&cookie());
    assert_eq!(port.calls(), vec!["start"]);
}

#[test]
fn refresh_reads_the_helper_and_disconnect_clears_it() {
    let port = FakePort::answering(vec![
        Ok(report(HelperState::Connected)),
        Ok(report(HelperState::Disconnected)),
    ]);
    let service = VpnService::new(port.clone());
    assert_eq!(
        service.refresh(),
        VpnState::Connected {
            address: None,
            expires_at: None
        }
    );
    assert_eq!(service.disconnect(), VpnState::Disconnected);
    assert_eq!(port.calls(), vec!["status", "stop"]);
}

#[test]
fn an_unreachable_helper_on_refresh_is_an_error_state() {
    let port = FakePort::answering(vec![Err(VpnError::Timeout)]);
    let service = VpnService::new(port);
    assert_eq!(service.refresh(), VpnState::Error(VpnError::Timeout));
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

/// A login that failed before the helper is the state the person sees, with
/// the login's own code for the guidance.
#[test]
fn a_failed_login_is_shown_with_its_code() {
    use crate::school_session::CampusVpnError;
    let service = VpnService::new(FakePort::answering(vec![]));
    let state = service.failed(VpnError::Login(CampusVpnError::NeedsAccount));
    let VpnState::Error(error) = state else {
        panic!("not an error state");
    };
    assert_eq!(error.code(), "vpn_needs_account");
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
    let port = FakePort::answering(vec![Ok(report(HelperState::Connected))]);
    let service = VpnService::new(port.clone());
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
    assert_eq!(port.calls(), ["start"]);
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
