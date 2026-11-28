use super::*;

const HOST: Host = Host::Kyomu;
const T0: i64 = 1_800_000_000;

fn fail(breaker: &Breaker, now: i64) {
    let admission = breaker.admit(HOST, now).unwrap();
    breaker.record(HOST, admission, Outcome::Failure, now);
}

#[test]
fn three_consecutive_failures_pause_the_host_and_a_success_resets_the_count() {
    let breaker = Breaker::default();
    fail(&breaker, T0);
    fail(&breaker, T0);
    let ok = breaker.admit(HOST, T0).unwrap();
    breaker.record(HOST, ok, Outcome::Success, T0);
    fail(&breaker, T0);
    fail(&breaker, T0);
    // Offline, TLS or a dropped request says nothing about the host.
    let neutral = breaker.admit(HOST, T0).unwrap();
    breaker.record(HOST, neutral, Outcome::Neutral, T0);
    assert_eq!(breaker.admit(HOST, T0), Ok(Admission::Normal));
    fail(&breaker, T0);
    assert_eq!(breaker.admit(HOST, T0 + 1), Err(T0 + 5 * 60));
    assert_eq!(breaker.paused_until(HOST, T0 + 1), Some(T0 + 5 * 60));
    // Other hosts keep working.
    assert_eq!(breaker.admit(Host::Moodle, T0 + 1), Ok(Admission::Normal));
}

#[test]
fn one_probe_after_each_pause_with_growing_pauses_until_a_success() {
    let breaker = Breaker::default();
    breaker.trip(HOST, T0);
    let mut until = T0 + 5 * 60;
    for next_pause in [15 * 60, 45 * 60, 135 * 60, 6 * 60 * 60, 6 * 60 * 60] {
        assert_eq!(breaker.admit(HOST, until - 1), Err(until));
        assert_eq!(breaker.admit(HOST, until), Ok(Admission::Probe));
        // Only the probe goes out; everyone else waits for its result, told
        // a time still ahead (2026-11-18: told the elapsed pause end, the
        // planner leased the same keys about 400 times in five seconds).
        assert_eq!(breaker.admit(HOST, until), Err(until + PROBE_WAIT_SECS));
        assert_eq!(
            breaker.paused_until(HOST, until + 2),
            Some(until + 2 + PROBE_WAIT_SECS)
        );
        breaker.record(HOST, Admission::Probe, Outcome::Failure, until);
        assert_eq!(breaker.admit(HOST, until), Err(until + next_pause));
        until += next_pause;
    }

    assert_eq!(breaker.admit(HOST, until), Ok(Admission::Probe));
    breaker.record(HOST, Admission::Probe, Outcome::Success, until);
    assert_eq!(breaker.admit(HOST, until), Ok(Admission::Normal));
    // Closed again: the next pause starts back at five minutes.
    breaker.trip(HOST, until);
    assert_eq!(breaker.admit(HOST, until), Err(until + 5 * 60));
}

/// A probe whose request was dropped must not leave the host paused forever.
#[test]
fn an_unfinished_probe_releases_the_next_probe() {
    let breaker = Breaker::default();
    breaker.trip(HOST, T0);
    let end = T0 + 5 * 60;
    assert_eq!(breaker.admit(HOST, end), Ok(Admission::Probe));
    breaker.record(HOST, Admission::Probe, Outcome::Neutral, end);
    assert_eq!(breaker.admit(HOST, end), Ok(Admission::Probe));
}

/// Requests admitted before a pause say nothing about it (`d/qrj1bk`): a
/// late failure does not lengthen it, and a late success, as Moodle's
/// several slots produce, neither ends it nor the probe already out.
#[test]
fn late_answers_from_before_the_pause_neither_trip_again_nor_reopen_it() {
    let breaker = Breaker::default();
    let in_flight = breaker.admit(HOST, T0).unwrap();
    breaker.trip(HOST, T0);
    for _ in 0..3 {
        breaker.record(HOST, in_flight, Outcome::Failure, T0 + 1);
    }
    assert_eq!(breaker.admit(HOST, T0 + 1), Err(T0 + 5 * 60));
    breaker.record(HOST, in_flight, Outcome::Success, T0 + 2);
    assert_eq!(breaker.admit(HOST, T0 + 2), Err(T0 + 5 * 60));
    assert!(breaker.probe_early(HOST, T0 + 3));
    assert_eq!(breaker.admit(HOST, T0 + 3), Ok(Admission::Probe));
    breaker.record(HOST, in_flight, Outcome::Success, T0 + 4);
    assert_eq!(breaker.admit(HOST, T0 + 4), Err(T0 + 4 + PROBE_WAIT_SECS));
}

/// A press or a way back to the school lets the probe out before the pause
/// ends, at most once per [`PROBE_WAIT_SECS`], never beside a probe already
/// out, and never for a host that is not paused (`d/kt8ts2`).
#[test]
fn an_early_probe_is_spaced_and_needs_a_pause() {
    let breaker = Breaker::default();
    assert!(!breaker.probe_early(HOST, T0));
    breaker.trip(HOST, T0);
    assert!(breaker.probe_early(HOST, T0 + 10));
    assert_eq!(breaker.admit(HOST, T0 + 10), Ok(Admission::Probe));
    assert!(!breaker.probe_early(HOST, T0 + 11));
    breaker.record(HOST, Admission::Probe, Outcome::Failure, T0 + 12);
    // Failed: paused at the next step, and the next early probe waits.
    assert_eq!(breaker.admit(HOST, T0 + 12), Err(T0 + 12 + 15 * 60));
    assert!(!breaker.probe_early(HOST, T0 + 10 + PROBE_WAIT_SECS - 1));
    assert!(breaker.probe_early(HOST, T0 + 10 + PROBE_WAIT_SECS));
    let at = T0 + 10 + PROBE_WAIT_SECS;
    assert_eq!(breaker.admit(HOST, at), Ok(Admission::Probe));
    breaker.record(HOST, Admission::Probe, Outcome::Success, at);
    assert_eq!(breaker.admit(HOST, at), Ok(Admission::Normal));
    // Another maintenance pause on this host still shares its probe interval.
    breaker.trip(HOST, at + 1);
    assert!(!breaker.probe_early(HOST, at + PROBE_WAIT_SECS - 1));
    assert!(breaker.probe_early(HOST, at + PROBE_WAIT_SECS));
}
