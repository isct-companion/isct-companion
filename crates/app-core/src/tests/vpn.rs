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

/// A service with a tunnel up and auto-continue on for an hour, then the
/// helper answering `answers`.
fn keeping(start: DateTime<Utc>, answers: Vec<Result<HelperReport, VpnError>>) -> VpnService {
    let mut script = vec![Ok(report(HelperState::Connected))];
    script.extend(answers);
    let service = VpnService::new(FakePort::answering(script));
    service.refresh();
    service.set_keep(Some(1), start).unwrap();
    service
}

/// Auto-continue always has an end, and starts from a tunnel that is up.
#[test]
fn auto_continue_needs_a_bounded_duration_and_a_tunnel() {
    let now = at(1_793_690_000);
    let down = VpnService::new(FakePort::answering(vec![]));
    assert_eq!(down.set_keep(Some(4), now), Err(VpnKeepError::NotConnected));
    let up = keeping(now, vec![]);
    assert_eq!(
        up.set_keep(Some(0), now),
        Err(VpnKeepError::InvalidDuration)
    );
    assert_eq!(
        up.set_keep(Some(KEEP_MAX_HOURS + 1), now),
        Err(VpnKeepError::InvalidDuration)
    );
    assert_eq!(
        up.keep_until(),
        Some(minutes_after(now, 60)),
        "a refusal changes nothing"
    );
    assert_eq!(up.set_keep(None, now), Ok(None));
    assert_eq!(up.keep_until(), None);
}

/// A tunnel ending inside the duration reconnects, without announcing an end
/// that does not matter; the next attempts wait, so a tunnel that keeps dying
/// does not loop on the password.
#[test]
fn a_tunnel_ending_inside_the_duration_reconnects_with_growing_waits() {
    let start = at(1_793_690_000);
    let service = keeping(
        start,
        vec![
            Ok(report(HelperState::Disconnected)),
            Ok(report(HelperState::Disconnected)),
            Ok(report(HelperState::Disconnected)),
        ],
    );
    let dropped = service.watch(minutes_after(start, 10));
    assert_eq!(dropped.action, Some(VpnAction::Reconnect));
    assert_eq!(dropped.notice, None);
    let up = VpnState::Connected {
        address: None,
        expires_at: None,
    };
    assert_eq!(service.reconnected(&up, minutes_after(start, 10)), None);
    // It died again at once: not before the wait is over.
    assert_eq!(service.watch(minutes_after(start, 10)).action, None);
    assert_eq!(
        service.watch(minutes_after(start, 11)).action,
        Some(VpnAction::Reconnect)
    );
}

/// The duration's end disconnects, once, and its reminder replaces the
/// session's, which would only have reconnected.
#[test]
fn the_end_of_the_duration_is_announced_and_disconnects() {
    let start = at(1_793_690_000);
    let session_ends = minutes_after(start, 52);
    let service = keeping(
        start,
        vec![
            Ok(connected_until(session_ends)),
            Ok(connected_until(session_ends)),
            Ok(report(HelperState::Connected)),
            Ok(report(HelperState::Disconnected)),
        ],
    );
    assert_eq!(service.watch(minutes_after(start, 45)).notice, None);
    assert_eq!(
        service.watch(minutes_after(start, 50)).notice,
        Some(VpnNotice::EndingSoon { minutes: 10 })
    );
    let end = service.watch(minutes_after(start, 60));
    assert_eq!(end.notice, Some(VpnNotice::KeepEnded));
    assert_eq!(end.action, Some(VpnAction::Disconnect));
    assert_eq!(service.keep_until(), None);
    // Nothing reconnects after it.
    assert_eq!(service.watch(minutes_after(start, 61)).action, None);
}

/// A refused password is never sent again; being offline only waits.
#[test]
fn reconnecting_stops_on_what_it_cannot_fix_and_waits_out_the_rest() {
    let start = at(1_793_690_000);
    let refused = VpnState::Error(VpnError::Login(CampusVpnError::Rejected));
    let service = keeping(start, vec![]);
    assert_eq!(
        service.reconnected(&refused, start),
        Some(VpnNotice::KeepStopped {
            code: "vpn_login_rejected"
        })
    );
    assert_eq!(service.keep_until(), None);

    let offline = VpnState::Error(VpnError::Login(CampusVpnError::Unreachable("x".into())));
    let service = keeping(start, vec![]);
    for attempt in 0..20 {
        assert_eq!(
            service.reconnected(&offline, minutes_after(start, attempt)),
            None
        );
    }
    assert!(
        service.keeps(),
        "offline never gives up before the duration"
    );
}

/// Logins the school accepts but whose tunnel keeps dying end it, so a broken
/// tunnel does not post the password every few minutes for a day.
#[test]
fn logins_that_never_last_give_up() {
    let start = at(1_793_690_000);
    let service = keeping(start, vec![]);
    let up = VpnState::Connected {
        address: None,
        expires_at: None,
    };
    for _ in 1..KEEP_MAX_SPENT {
        assert_eq!(service.reconnected(&up, start), None);
    }
    assert_eq!(
        service.reconnected(&up, start),
        Some(VpnNotice::KeepStopped {
            code: "tunnel_failed"
        })
    );
}

/// A disconnect turns it off, and two sign-ins never run at once.
#[test]
fn a_disconnect_ends_auto_continue_and_logins_are_one_at_a_time() {
    let start = at(1_793_690_000);
    let service = keeping(start, vec![Ok(report(HelperState::Disconnected))]);
    service.disconnect();
    assert!(!service.keeps());
    let first = service.begin_login();
    assert!(first.is_some());
    assert!(service.begin_login().is_none());
    drop(first);
    assert!(service.begin_login().is_some());
}
