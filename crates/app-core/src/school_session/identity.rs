//! An account's identity: the facts Core keeps about it, the one sign-in
//! and the one verification that may run for it, and the state shells see,
//! derived from those facts.
//!
//! The facts: whether credentials are saved (and the school's refusal of
//! exact credentials, which `school_auth` keeps with them); a pause after a
//! sign-in that got no verdict; the identity cookie snapshot, whether the
//! school found it invalid and when it last confirmed it; and the running
//! sign-in. No state is stored and moved from one value to another: `view`
//! derives what shells render.
//!
//! A snapshot is trusted in generations. One read back from the vault is
//! unverified; a sign-in makes a verified one; a verification (a bounded
//! SSO from a new jar holding the snapshot) turns an unverified or aging one
//! into a new verified generation, or finds it invalid when the school
//! sends it to the sign-in page. A network failure, the device offline or a
//! site's maintenance leaves it unverified. A web page landing on the
//! sign-in page says nothing about the identity, and a read that reused a
//! site's instance verifies nothing.
//!
//! The sign-in belongs to the identity, not to whoever asked: a caller that
//! stops waiting leaves it running for the others; signing out, a reset,
//! Core's shutdown and the system taking the background time back end it,
//! and the person's Connect replaces it. A sign-in's result applies only to
//! its own credentials, and only while it is still the account's sign-in: a
//! replaced one changes nothing. Passwords go out only when the identity
//! provider shows its sign-in page, and a refusal is kept with the
//! credentials it is about, so Core does not send them again on its own
//! until they change or the person retries.

use super::{
    guard, login, site::Walked, Inner, Permit, SchoolSessionState, SchoolSignInProblem,
    SchoolSignInProblemKind as Kind, Snapshot, RETRY_NOT_BEFORE, SIGN_IN_DEADLINE, VERIFIED_FRESH,
    VERIFY_DEADLINE,
};
use crate::school_auth::{SchoolAuthProfile, SchoolCredentials};
use chrono::{DateTime, Utc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;

/// How far a snapshot is trusted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Trust {
    /// Read back from the vault, or not confirmed since.
    Unverified,
    /// Made by a sign-in, or confirmed by a verification, at this instant.
    Verified(Instant),
    /// The school sent it to the sign-in page.
    Invalid,
}

/// Who asked for a sign-in.
#[derive(Clone)]
pub(super) enum How {
    /// A read or a download found the identity gone.
    Background,
    /// A web operation's one sign-in. `past_pause`: the person's Reload on
    /// the sign-in page, which may go through a pause, never through a
    /// refusal.
    Web { past_pause: bool },
    /// The person's Retry: once through a refusal or a pause.
    Retry,
    /// The person's Connect with these credentials.
    Connect(SchoolCredentials),
}

impl How {
    fn lifts_refusal(&self) -> bool {
        matches!(self, Self::Retry | Self::Connect(_))
    }

    fn lifts_pause(&self) -> bool {
        self.lifts_refusal() || matches!(self, Self::Web { past_pause: true })
    }
}

/// Why a sign-in gave no identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Ended {
    /// No credentials are saved.
    NotConfigured,
    /// The school refused the credentials, or wants the person on its page
    /// (`SchoolSignInProblemKind::needs_web`).
    Refused(SchoolSignInProblem),
    /// No verdict: the network, an unanswered password, an anomaly. A
    /// sign-in other than a Connect then pauses (`Identity::pause_until`).
    Unavailable(SchoolSignInProblem),
    /// The shell reports no network: nothing was sent.
    Offline,
    /// A Connect replaced it, or a sign-out, a reset or the system stopped
    /// it, before the school's verdict.
    Stopped,
}

/// How a verification ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Checked {
    /// The school accepted the identity: the verified generation.
    Verified(u64),
    /// The identity is gone (the school sent it to its sign-in page, or there
    /// is none to verify).
    Expired,
    /// No verdict: offline, the network, maintenance, out of time, stopped.
    Unconfirmed,
}

/// No sign-in on Core's own before `until`, and why.
#[derive(Clone)]
struct Pause {
    until: Instant,
    problem: SchoolSignInProblem,
}

struct State {
    /// Saved credentials exist (the status metadata).
    configured: bool,
    snapshot: Option<Arc<Snapshot>>,
    trust: Trust,
    /// Grows with every new snapshot. A site that finds its copy refused
    /// signs in only while this has not moved since it copied it.
    generation: u64,
    pause: Option<Pause>,
    flight: Option<Flight>,
    /// Numbers the sign-ins, so one applies its result only while it is
    /// still the account's.
    flights: u64,
    /// The one running verification.
    check: Option<watch::Receiver<Option<Checked>>>,
}

impl State {
    fn current(&self, flight: u64) -> bool {
        self.flight
            .as_ref()
            .is_some_and(|running| running.id == flight)
    }

    fn paused(&self) -> Option<&Pause> {
        self.pause
            .as_ref()
            .filter(|pause| Instant::now() < pause.until)
    }
}

/// The one running sign-in.
struct Flight {
    id: u64,
    done: watch::Receiver<Option<Result<u64, Ended>>>,
    cancel: CancellationToken,
}

pub(super) struct Identity {
    state: Mutex<State>,
}

impl Identity {
    pub(super) fn new(configured: bool) -> Self {
        Self {
            state: Mutex::new(State {
                configured,
                snapshot: None,
                trust: Trust::Unverified,
                generation: 0,
                pause: None,
                flight: None,
                flights: 0,
                check: None,
            }),
        }
    }

    pub(super) fn generation(&self) -> u64 {
        guard(&self.state).generation
    }

    /// Whether credentials are saved, as the caller read the status
    /// metadata.
    pub(super) fn set_configured(&self, configured: bool) {
        guard(&self.state).configured = configured;
    }

    /// The current snapshot and its generation, unless the school found it
    /// invalid.
    pub(super) fn snapshot(&self) -> Option<(Arc<Snapshot>, u64)> {
        let state = guard(&self.state);
        state
            .snapshot
            .clone()
            .filter(|_| state.trust != Trust::Invalid)
            .map(|snapshot| (snapshot, state.generation))
    }

    /// The snapshot of `generation`, while it is the verified one.
    pub(super) fn verified(&self, generation: u64) -> Option<Arc<Snapshot>> {
        let state = guard(&self.state);
        (state.generation == generation && matches!(state.trust, Trust::Verified(_)))
            .then(|| state.snapshot.clone())
            .flatten()
    }

    /// The generation verified less than `VERIFIED_FRESH` ago.
    pub(super) fn fresh(&self) -> Option<u64> {
        let state = guard(&self.state);
        match state.trust {
            Trust::Verified(at) if state.snapshot.is_some() && at.elapsed() < VERIFIED_FRESH => {
                Some(state.generation)
            }
            _ => None,
        }
    }

    /// When a pause still running ends.
    pub(super) fn pause_until(&self) -> Option<Instant> {
        guard(&self.state).paused().map(|pause| pause.until)
    }

    /// The school sent `generation` to its sign-in page: it is no identity
    /// any more, and the next use signs in.
    pub(super) fn invalidate(&self, generation: u64) {
        let mut state = guard(&self.state);
        if state.generation == generation {
            state.trust = Trust::Invalid;
        }
    }

    /// What `store` keeps: the snapshot.
    pub(super) fn persisted(&self) -> Option<Arc<Snapshot>> {
        guard(&self.state).snapshot.clone()
    }

    /// The state shells render, from the facts: a running sign-in first,
    /// then missing credentials, the school's `refusal` of the saved ones,
    /// a pause still running, and otherwise normal.
    pub(super) fn view(
        &self,
        refusal: Option<SchoolSignInProblem>,
    ) -> (
        SchoolSessionState,
        Option<SchoolSignInProblem>,
        Option<DateTime<Utc>>,
    ) {
        let state = guard(&self.state);
        if state.flight.is_some() {
            return (SchoolSessionState::SigningIn, None, None);
        }
        if !state.configured {
            return (SchoolSessionState::NotConfigured, None, None);
        }
        if let Some(problem) = refusal {
            let shown = if problem.kind.needs_web() {
                SchoolSessionState::NeedsWeb
            } else {
                SchoolSessionState::Refused
            };
            return (shown, Some(problem), None);
        }
        if let Some(pause) = state.paused() {
            let left = pause.until.saturating_duration_since(Instant::now());
            let retry_at = Utc::now() + chrono::Duration::from_std(left).unwrap_or_default();
            return (
                SchoolSessionState::Unavailable,
                Some(pause.problem.clone()),
                Some(retry_at),
            );
        }
        (SchoolSessionState::Normal, None, None)
    }

    /// A snapshot read back from the vault, unverified. `None` when a
    /// sign-in already made a newer one.
    pub(super) fn restore(&self, snapshot: Snapshot) -> Option<u64> {
        let mut state = guard(&self.state);
        if state.snapshot.is_some() {
            return None;
        }
        state.generation += 1;
        state.snapshot = Some(Arc::new(snapshot));
        state.trust = Trust::Unverified;
        Some(state.generation)
    }

    /// Drops a pause that ran out. `true` when it did.
    pub(super) fn expire_pause(&self, now: Instant) -> bool {
        let mut state = guard(&self.state);
        if state.pause.as_ref().is_some_and(|pause| now >= pause.until) {
            state.pause = None;
            return true;
        }
        false
    }

    /// Drops the account's identity (the barrier).
    pub(super) fn forget(&self) {
        let mut state = guard(&self.state);
        state.snapshot = None;
        state.trust = Trust::Unverified;
        state.pause = None;
        state.flight = None;
        state.check = None;
        state.generation += 1;
    }

    pub(super) fn cancel_flight(&self) {
        if let Some(flight) = &guard(&self.state).flight {
            flight.cancel.cancel();
        }
    }

    pub(super) async fn wait_flight(&self) {
        let done = guard(&self.state)
            .flight
            .as_ref()
            .map(|flight| flight.done.clone());
        if let Some(mut done) = done {
            let _ = done.wait_for(Option::is_some).await;
        }
    }

    /// The fresh snapshot of `generation` was sent straight back to the
    /// sign-in page: no further sign-in on Core's own until the pause ends.
    pub(super) fn anomaly(&self, generation: u64) {
        let mut state = guard(&self.state);
        if state.generation != generation || state.paused().is_some() {
            return;
        }
        state.pause = Some(Pause {
            until: Instant::now() + RETRY_NOT_BEFORE,
            problem: problem(
                Kind::Anomaly,
                "the school sent a fresh sign-in back to the sign-in page",
            ),
        });
    }

    /// Applies sign-in `id`'s end to the identity, while it is still the
    /// account's sign-in; `None` once another replaced it. Only a sign-in
    /// on Core's own (not a Connect) pauses after no verdict.
    fn apply(&self, id: u64, connect: bool, end: End) -> Option<Result<u64, Ended>> {
        let mut state = guard(&self.state);
        if !state.current(id) {
            return None;
        }
        Some(match end {
            End::SignedIn(snapshot) => {
                state.generation += 1;
                state.snapshot = Some(Arc::new(snapshot));
                state.trust = Trust::Verified(Instant::now());
                state.pause = None;
                Ok(state.generation)
            }
            End::Refused { problem, .. } => Err(Ended::Refused(problem)),
            End::Unavailable(problem) => {
                if !connect {
                    state.pause = Some(Pause {
                        until: Instant::now() + RETRY_NOT_BEFORE,
                        problem: problem.clone(),
                    });
                }
                Err(Ended::Unavailable(problem))
            }
            End::NotConfigured => Err(Ended::NotConfigured),
            End::Offline => Err(Ended::Offline),
            End::Stopped => Err(Ended::Stopped),
        })
    }

    /// Sign-in `id` is over: it is the account's no longer.
    fn land(&self, id: u64) {
        let mut state = guard(&self.state);
        if state.current(id) {
            state.flight = None;
        }
    }
}

fn problem(kind: Kind, message: impl Into<String>) -> SchoolSignInProblem {
    SchoolSignInProblem {
        kind,
        message: message.into(),
    }
}

/// How a sign-in task's flow ended, before it is applied.
enum End {
    SignedIn(Snapshot),
    /// No credentials are saved.
    NotConfigured,
    /// The school refused the credentials or wants the person on its page;
    /// `sent` is false when a refusal kept with them held them back.
    Refused {
        problem: SchoolSignInProblem,
        sent: bool,
    },
    Unavailable(SchoolSignInProblem),
    Offline,
    /// Stopped before the school's verdict, the password not yet sent.
    Stopped,
}

impl Inner {
    /// Makes sure `profile` has an identity newer than `seen`: one that
    /// appeared meanwhile is used as it is; otherwise this joins the running
    /// sign-in or starts one, unless the facts hold it back (no credentials,
    /// a refusal of them, a pause). The person's Connect never joins: it
    /// replaces the running sign-in with its own at once.
    pub(super) async fn sign_in(
        self: &Arc<Self>,
        profile: SchoolAuthProfile,
        seen: u64,
        how: How,
    ) -> Result<u64, Ended> {
        let account = self.account(profile);
        let identity = &account.identity;
        let joins = !matches!(how, How::Connect(_));
        loop {
            let mut done = {
                let mut state = guard(&identity.state);
                let newer = state.generation > seen
                    && state.snapshot.is_some()
                    && state.trust != Trust::Invalid;
                if newer && !how.lifts_refusal() {
                    return Ok(state.generation);
                }
                let running = state
                    .flight
                    .as_ref()
                    .filter(|_| joins)
                    .map(|flight| flight.done.clone());
                match running {
                    Some(done) => done,
                    None => {
                        match &state.flight {
                            Some(replaced) => replaced.cancel.cancel(),
                            None if joins => self.held(profile, &state, &how)?,
                            None => {}
                        }
                        let Some(permit) = account.owner.permit() else {
                            return Err(Ended::Stopped);
                        };
                        state.flights += 1;
                        let id = state.flights;
                        let (sender, done) = watch::channel(None);
                        let cancel = permit.cancel().child_token();
                        let previous = state.flight.replace(Flight {
                            id,
                            done: done.clone(),
                            cancel: cancel.clone(),
                        });
                        let inner = self.clone();
                        let how = how.clone();
                        let started = self.tasks.spawn(async move {
                            let result = inner.signing_in(profile, id, how, permit, cancel).await;
                            let _ = sender.send(Some(result));
                        });
                        if !started {
                            state.flight = previous;
                            return Err(Ended::Stopped);
                        }
                        done
                    }
                }
            };
            self.publish();
            let ended = match done.wait_for(Option::is_some).await {
                Ok(result) => result.clone(),
                Err(_) => None,
            };
            match ended.unwrap_or(Err(Ended::Stopped)) {
                // A Connect replaced the sign-in this waited for: wait for
                // the Connect's.
                Err(Ended::Stopped) if joins && guard(&identity.state).flight.is_some() => {}
                result => return result,
            }
        }
    }

    /// Why a sign-in on Core's own (or a web operation's) may not start now.
    fn held(&self, profile: SchoolAuthProfile, state: &State, how: &How) -> Result<(), Ended> {
        if !state.configured {
            return Err(Ended::NotConfigured);
        }
        if !how.lifts_refusal() {
            if let Some(problem) = self.auth.refusal(profile) {
                return Err(Ended::Refused(problem));
            }
        }
        match state.paused() {
            Some(pause) if !how.lifts_pause() => Err(Ended::Unavailable(pause.problem.clone())),
            _ => Ok(()),
        }
    }

    /// The sign-in task: runs the flow within `SIGN_IN_DEADLINE`, then
    /// applies what it found while still holding the permit.
    async fn signing_in(
        self: Arc<Self>,
        profile: SchoolAuthProfile,
        id: u64,
        how: How,
        permit: Permit,
        cancel: CancellationToken,
    ) -> Result<u64, Ended> {
        let submitted = AtomicBool::new(false);
        let mut used = None;
        let end = {
            let flow = self.sign_in_flow(profile, &how, &submitted, &mut used);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                end = tokio::time::timeout(SIGN_IN_DEADLINE, flow) => Some(end.ok()),
            }
        };
        let end = match end {
            Some(Some(end)) => end,
            // Stopped (cancelled or out of time) before the verdict. A
            // password already sent is the school's to judge: wait, and
            // never resend it at once.
            stopped => {
                if submitted.load(Ordering::SeqCst) {
                    End::Unavailable(problem(
                        Kind::Unanswered,
                        "the sign-in ended before the school answered",
                    ))
                } else if stopped.is_some() {
                    End::Unavailable(problem(Kind::Network, "the sign-in took too long"))
                } else {
                    End::Stopped
                }
            }
        };
        let result = self.finish_sign_in(profile, id, &how, used, end).await;
        self.account(profile).identity.land(id);
        drop(permit);
        self.publish();
        result
    }

    /// Reads the credentials `how` signs in with (into `used`) and signs in.
    async fn sign_in_flow(
        &self,
        profile: SchoolAuthProfile,
        how: &How,
        submitted: &AtomicBool,
        used: &mut Option<SchoolCredentials>,
    ) -> End {
        let credentials = match how {
            How::Connect(credentials) => credentials.clone(),
            How::Background | How::Web { .. } | How::Retry => {
                let auth = self.auth.clone();
                match tokio::task::spawn_blocking(move || auth.sign_in_credentials(profile)).await {
                    Ok(Ok(Some(saved))) => {
                        // A refusal kept with exactly these credentials holds
                        // them back.
                        if let Some(refusal) = saved.refusal.filter(|_| !how.lifts_refusal()) {
                            return End::Refused {
                                problem: refusal,
                                sent: false,
                            };
                        }
                        saved.credentials
                    }
                    Ok(Ok(None)) => return End::NotConfigured,
                    Ok(Err(error)) => {
                        return End::Unavailable(problem(
                            Kind::Network,
                            format!("the saved credentials could not be read: {error}"),
                        ))
                    }
                    Err(_) => return End::Stopped,
                }
            }
        };
        *used = Some(credentials.clone());
        if self.offline.load(Ordering::SeqCst) {
            return End::Offline;
        }
        log::info!(target: "app_core::school_session", "signing in profile={profile:?}");
        let end = match profile {
            SchoolAuthProfile::Science => {
                login::science(self.transport.as_ref(), &credentials, submitted).await
            }
            SchoolAuthProfile::TokyoTech => {
                login::tokyo(self.transport.as_ref(), &credentials, submitted).await
            }
        };
        match end {
            login::LoginEnd::SignedIn(snapshot) => End::SignedIn(snapshot),
            login::LoginEnd::Rejected(kind, message) | login::LoginEnd::NeedsWeb(kind, message) => {
                End::Refused {
                    problem: problem(kind, message),
                    sent: true,
                }
            }
            login::LoginEnd::Unavailable(kind, message) => End::Unavailable(problem(kind, message)),
        }
    }

    /// Applies a sign-in's end (`Identity::apply`), then what is kept of it,
    /// only for the credentials it `used`: the person's Connect input becomes
    /// the verified credentials once the school accepts it, a sign-in with
    /// the saved ones that the school accepts drops their refusal, and a
    /// refusal is kept only for credentials that are the verified ones. A
    /// Connect keeps no page that needs the person; one that fails leaves
    /// every fact as it was.
    async fn finish_sign_in(
        &self,
        profile: SchoolAuthProfile,
        id: u64,
        how: &How,
        used: Option<SchoolCredentials>,
        end: End,
    ) -> Result<u64, Ended> {
        let identity = &self.account(profile).identity;
        let connect = matches!(how, How::Connect(_));
        let sent = matches!(end, End::Refused { sent: true, .. });
        let Some(result) = identity.apply(id, connect, end) else {
            log::info!(target: "app_core::school_session", "a replaced sign-in ended profile={profile:?}");
            return Err(Ended::Stopped);
        };
        match (&result, used) {
            (Ok(generation), Some(credentials)) => {
                log::info!(target: "app_core::school_session", "signed in profile={profile:?} generation={generation}");
                // The school saw every secret these credentials hold.
                let auth = self.auth.clone();
                let saved = tokio::task::spawn_blocking(move || {
                    if connect {
                        auth.promote(profile, &credentials)
                    } else {
                        auth.clear_refusal(profile, &credentials)
                    }
                })
                .await;
                if let Ok(Err(error)) = saved {
                    log::warn!(target: "app_core::school_session", "verified credentials not saved profile={profile:?}: {error}");
                }
                identity.set_configured(self.auth.configured(profile));
                self.persist(profile).await;
            }
            (Err(Ended::Refused(problem)), Some(credentials))
                if sent && !(connect && problem.kind.needs_web()) =>
            {
                log::info!(target: "app_core::school_session", "sign-in refused profile={profile:?} kind={:?}", problem.kind);
                let auth = self.auth.clone();
                let kept = problem.clone();
                let recorded = tokio::task::spawn_blocking(move || {
                    auth.record_refusal(profile, &credentials, &kept)
                })
                .await;
                if let Ok(Err(error)) = recorded {
                    log::warn!(target: "app_core::school_session", "refusal not kept profile={profile:?}: {error}");
                }
            }
            (Err(ended), _) => {
                log::info!(target: "app_core::school_session", "sign-in ended profile={profile:?}: {ended:?}");
            }
            (Ok(_), None) => {}
        }
        result
    }
}

impl Inner {
    /// Makes sure `profile`'s identity is verified: one verified less than
    /// `VERIFIED_FRESH` ago is used as it is; otherwise this joins the running
    /// verification or starts one. A verification is one SSO from a new jar
    /// holding the snapshot, within `VERIFY_DEADLINE` for everything (the
    /// wait for a permit, the redirects, the new cookies); the identity
    /// cookies the school sets on the way are kept, and the snapshot they
    /// make is committed, as a new verified generation, only at the SSO's
    /// successful end. It sends no password.
    pub(super) async fn verify(self: &Arc<Self>, profile: SchoolAuthProfile) -> Checked {
        let account = self.account(profile);
        let identity = &account.identity;
        let mut done = {
            let mut state = guard(&identity.state);
            if let Trust::Verified(at) = state.trust {
                if state.snapshot.is_some() && at.elapsed() < VERIFIED_FRESH {
                    return Checked::Verified(state.generation);
                }
            }
            if let Some(check) = &state.check {
                check.clone()
            } else {
                let Some(snapshot) = state
                    .snapshot
                    .clone()
                    .filter(|_| state.trust != Trust::Invalid)
                else {
                    return Checked::Expired;
                };
                let Some(permit) = account.owner.permit() else {
                    return Checked::Unconfirmed;
                };
                let generation = state.generation;
                let (sender, done) = watch::channel(None);
                state.check = Some(done.clone());
                let inner = self.clone();
                let started = self.tasks.spawn(async move {
                    let checked = inner.checking(profile, snapshot, generation, permit).await;
                    let _ = sender.send(Some(checked));
                });
                if !started {
                    state.check = None;
                    return Checked::Unconfirmed;
                }
                done
            }
        };
        let checked = match done.wait_for(Option::is_some).await {
            Ok(checked) => checked.unwrap_or(Checked::Unconfirmed),
            Err(_) => Checked::Unconfirmed,
        };
        checked
    }

    /// The verification task: the SSO, then its verdict recorded while the
    /// permit is still held.
    async fn checking(
        self: Arc<Self>,
        profile: SchoolAuthProfile,
        snapshot: Arc<Snapshot>,
        generation: u64,
        permit: Permit,
    ) -> Checked {
        let walked = if self.offline.load(Ordering::SeqCst) {
            None
        } else {
            let cancel = permit.cancel().clone();
            tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                walked = tokio::time::timeout(VERIFY_DEADLINE, self.check_sso(profile, &snapshot)) => walked.ok(),
            }
        };
        let identity = &self.account(profile).identity;
        let checked = {
            let mut state = guard(&identity.state);
            state.check = None;
            if state.generation != generation {
                // A sign-in made a newer identity meanwhile: it decides.
                match state.trust {
                    Trust::Verified(_) if state.snapshot.is_some() => {
                        Checked::Verified(state.generation)
                    }
                    _ => Checked::Unconfirmed,
                }
            } else {
                match walked {
                    Some(Walked::Ready(fresh)) => {
                        state.generation += 1;
                        state.snapshot = Some(Arc::new(fresh));
                        state.trust = Trust::Verified(Instant::now());
                        // The school answered: a pause after a failure is over.
                        state.pause = None;
                        Checked::Verified(state.generation)
                    }
                    Some(Walked::SignIn) => {
                        state.trust = Trust::Invalid;
                        Checked::Expired
                    }
                    Some(Walked::Failed) | None => Checked::Unconfirmed,
                }
            }
        };
        log::info!(target: "app_core::school_session", "identity checked profile={profile:?} {checked:?}");
        if matches!(checked, Checked::Verified(verified) if verified > generation) {
            self.persist(profile).await;
        }
        drop(permit);
        self.publish();
        checked
    }
}
