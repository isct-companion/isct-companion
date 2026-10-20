//! Per-host circuit breaker. Three consecutive failures, or a maintenance
//! page, pause every request to the host: 5 minutes, then 15, 45 … at most
//! 6 hours. After a pause exactly one probe request goes out; its success
//! closes the breaker, its failure pauses again at the next step. Fixed steps,
//! no jitter: the planner reads `until` and shows it.

use super::Host;
use std::{collections::HashMap, sync::Mutex};

const FAILURES_TO_TRIP: u32 = 3;
const FIRST_PAUSE_SECS: i64 = 5 * 60;
const MAX_PAUSE_SECS: i64 = 6 * 60 * 60;

/// How a request was let through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Admission {
    Normal,
    /// The single request allowed after a pause.
    Probe,
}

/// What a finished request says about the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Success,
    Failure,
    /// Says nothing about the host (offline, TLS, cancelled, too large).
    Neutral,
}

#[derive(Debug, Default)]
struct HostState {
    consecutive_failures: u32,
    /// Pauses since the last success; picks the next pause length.
    trips: u32,
    paused_until: Option<i64>,
    probing: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Breaker {
    hosts: Mutex<HashMap<Host, HostState>>,
}

fn pause_secs(trips: u32) -> i64 {
    let mut secs = FIRST_PAUSE_SECS;
    for _ in 0..trips {
        secs = secs.saturating_mul(3);
        if secs >= MAX_PAUSE_SECS {
            return MAX_PAUSE_SECS;
        }
    }
    secs.min(MAX_PAUSE_SECS)
}

impl Breaker {
    fn with<T>(&self, host: Host, f: impl FnOnce(&mut HostState) -> T) -> T {
        let mut hosts = self
            .hosts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(hosts.entry(host).or_default())
    }

    /// Lets a request through, or returns the end of the current pause. While
    /// the probe is out, everyone else still sees the (elapsed) pause end.
    pub(crate) fn admit(&self, host: Host, now: i64) -> Result<Admission, i64> {
        self.with(host, |state| match state.paused_until {
            None => Ok(Admission::Normal),
            Some(until) if now < until || state.probing => Err(until),
            Some(_) => {
                state.probing = true;
                Ok(Admission::Probe)
            }
        })
    }

    /// The end of the current pause, without taking the probe.
    pub(crate) fn paused_until(&self, host: Host, now: i64) -> Option<i64> {
        self.with(host, |state| {
            state
                .paused_until
                .filter(|until| now < *until || state.probing)
        })
    }

    pub(crate) fn record(&self, host: Host, admission: Admission, outcome: Outcome, now: i64) {
        self.with(host, |state| {
            if admission == Admission::Probe {
                state.probing = false;
            }
            match outcome {
                Outcome::Success => *state = HostState::default(),
                Outcome::Neutral => {}
                Outcome::Failure if admission == Admission::Probe => trip(state, now),
                Outcome::Failure => {
                    // A request admitted before a pause began says nothing new.
                    if state.paused_until.is_none() {
                        state.consecutive_failures += 1;
                        if state.consecutive_failures >= FAILURES_TO_TRIP {
                            trip(state, now);
                        }
                    }
                }
            }
        });
    }

    /// The caller saw a maintenance page: pause now, at the next step.
    pub(crate) fn trip(&self, host: Host, now: i64) {
        self.with(host, |state| {
            state.probing = false;
            trip(state, now);
        });
    }
}

fn trip(state: &mut HostState, now: i64) {
    state.paused_until = Some(now.saturating_add(pause_secs(state.trips)));
    state.trips = state.trips.saturating_add(1);
    state.consecutive_failures = 0;
}

#[cfg(test)]
#[path = "../tests/net__breaker.rs"]
mod tests;
