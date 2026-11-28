//! Per-host circuit breaker. Three consecutive failures, or a maintenance
//! page, pause every request to the host: 5 minutes, then 15, 45 … at most
//! 6 hours. After a pause exactly one probe request goes out; its success
//! closes the breaker, its failure pauses again at the next step. Fixed steps,
//! no jitter: the planner reads `until` and shows it. While the probe is
//! out, everyone else is told to wait [`PROBE_WAIT_SECS`] more, so callers
//! never spin on a pause that has already ended. A person's refresh or a
//! network that came back may take the probe early, at most once per
//! [`PROBE_WAIT_SECS`] (`d/kt8ts2`).

use super::Host;
use std::{collections::HashMap, sync::Mutex};

const FAILURES_TO_TRIP: u32 = 3;
const FIRST_PAUSE_SECS: i64 = 5 * 60;
const MAX_PAUSE_SECS: i64 = 6 * 60 * 60;
/// How long others wait while a probe is out, and the least time between
/// two early probes.
pub(crate) const PROBE_WAIT_SECS: i64 = 30;

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

/// A host's health: closed (requests go out), paused, or probing. Normal
/// requests are admitted only while closed, so a normal request that ends
/// while the host is paused was admitted before the pause: whatever it
/// says, only the probe opens the host again (`d/qrj1bk`).
#[derive(Debug, Default)]
struct Health {
    consecutive_failures: u32,
    /// Pauses since the last success; picks the next pause length.
    trips: u32,
    paused_until: Option<i64>,
    probing: bool,
}

#[derive(Debug, Default)]
struct HostState {
    health: Health,
    /// When the last early probe was let through. Not health: the spacing
    /// holds across pauses and the successes between them.
    early_probe_at: Option<i64>,
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

    /// Lets a request through, or returns when to try again: the end of the
    /// current pause, or a moment from now while the probe is out.
    pub(crate) fn admit(&self, host: Host, now: i64) -> Result<Admission, i64> {
        self.with(host, |state| {
            let health = &mut state.health;
            match health.paused_until {
                None => Ok(Admission::Normal),
                Some(until) if now < until => Err(until),
                Some(_) if health.probing => Err(now.saturating_add(PROBE_WAIT_SECS)),
                Some(_) => {
                    health.probing = true;
                    Ok(Admission::Probe)
                }
            }
        })
    }

    /// When to try again, while the host is paused or probed, without taking
    /// the probe.
    pub(crate) fn paused_until(&self, host: Host, now: i64) -> Option<i64> {
        self.with(host, |state| match state.health.paused_until {
            Some(until) if now < until => Some(until),
            Some(_) if state.health.probing => Some(now.saturating_add(PROBE_WAIT_SECS)),
            _ => None,
        })
    }

    /// Ends a pause now so the next request is its probe, for a refresh the
    /// person asked for or a network that came back. Refused while a probe
    /// is out, when the host is not paused, or within [`PROBE_WAIT_SECS`] of
    /// the last early probe, however many pauses ago; returns whether it was
    /// let through.
    pub(crate) fn probe_early(&self, host: Host, now: i64) -> bool {
        self.with(host, |state| {
            let health = &mut state.health;
            let paused = health.paused_until.is_some_and(|until| now < until);
            let spaced = state
                .early_probe_at
                .is_none_or(|at| now.saturating_sub(at) >= PROBE_WAIT_SECS);
            if !paused || health.probing || !spaced {
                return false;
            }
            health.paused_until = Some(now);
            state.early_probe_at = Some(now);
            true
        })
    }

    /// What a finished request says about the host. Only the probe that is
    /// out opens or pauses a paused host; a normal request counts only while
    /// the host is still closed, the state it was admitted in.
    pub(crate) fn record(&self, host: Host, admission: Admission, outcome: Outcome, now: i64) {
        self.with(host, |state| {
            let health = &mut state.health;
            match admission {
                Admission::Probe if health.probing => {
                    health.probing = false;
                    match outcome {
                        Outcome::Success => *health = Health::default(),
                        Outcome::Failure => trip(health, now),
                        // Released: the next request is the probe.
                        Outcome::Neutral => {}
                    }
                }
                // A probe a newer pause superseded says nothing.
                Admission::Probe => {}
                Admission::Normal if health.paused_until.is_none() => match outcome {
                    Outcome::Success => *health = Health::default(),
                    Outcome::Failure => {
                        health.consecutive_failures += 1;
                        if health.consecutive_failures >= FAILURES_TO_TRIP {
                            trip(health, now);
                        }
                    }
                    Outcome::Neutral => {}
                },
                // Admitted before the pause began: says nothing new.
                Admission::Normal => {}
            }
        });
    }

    /// The caller saw a maintenance page: pause now, at the next step.
    pub(crate) fn trip(&self, host: Host, now: i64) {
        self.with(host, |state| {
            state.health.probing = false;
            trip(&mut state.health, now);
        });
    }
}

fn trip(health: &mut Health, now: i64) {
    health.paused_until = Some(now.saturating_add(pause_secs(health.trips)));
    health.trips = health.trips.saturating_add(1);
    health.consecutive_failures = 0;
}

#[cfg(test)]
#[path = "../tests/net__breaker.rs"]
mod tests;
