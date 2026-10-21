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
        // Only the probe goes out; everyone else waits for its result.
        assert_eq!(breaker.admit(HOST, until), Err(until));
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

/// Requests admitted before a pause that fail afterwards do not lengthen it.
#[test]
fn late_failures_from_before_the_pause_do_not_trip_again() {
    let breaker = Breaker::default();
    let in_flight = breaker.admit(HOST, T0).unwrap();
    breaker.trip(HOST, T0);
    breaker.record(HOST, in_flight, Outcome::Failure, T0 + 1);
    breaker.record(HOST, in_flight, Outcome::Failure, T0 + 1);
    breaker.record(HOST, in_flight, Outcome::Failure, T0 + 1);
    assert_eq!(breaker.admit(HOST, T0 + 1), Err(T0 + 5 * 60));
}
