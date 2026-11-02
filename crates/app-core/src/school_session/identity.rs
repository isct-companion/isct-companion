//! An account's identity: its state, its snapshot, the one sign-in and the
//! one verification that may run for it.
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
//! Only the identity changes its state. The sign-in belongs to the identity,
//! not to whoever asked: a caller that stops waiting leaves it running for
//! the others; signing out, a reset, an account switch, Core's shutdown and
//! the system taking the background time back end it. Passwords go out only
//! when the identity provider shows its sign-in page, and a refusal is kept
//! with the credentials it is about, so they are not sent again until they
//! change or the person retries.

use super::{
    guard, login, site::Walked, Inner, Permit, SchoolSessionState, SchoolSignInProblem,
    SchoolSignInProblemKind as Kind, Snapshot, Unavailable, RETRY_NOT_BEFORE, SIGN_IN_DEADLINE,
    VERIFIED_FRESH, VERIFY_DEADLINE,
};
use crate::school_auth::{SchoolAuthProfile, SchoolCredentials};
use chrono::{DateTime, Utc};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    NotConfigured,
    Idle,
    SigningIn,
    /// A snapshot no sign-in or verification has confirmed yet.
    Unverified,
    Verifying,
    /// A verified snapshot.
    Active,
    Rejected,
    NeedsWeb,
    Unavailable {
        not_before: Instant,
    },
}

impl Phase {
    fn state(self) -> SchoolSessionState {
        match self {
            Self::NotConfigured => SchoolSessionState::NotConfigured,
            Self::Idle => SchoolSessionState::Idle,
            Self::SigningIn => SchoolSessionState::SigningIn,
            Self::Unverified => SchoolSessionState::Unverified,
            Self::Verifying => SchoolSessionState::Verifying,
            Self::Active => SchoolSessionState::Active,
            Self::Rejected => SchoolSessionState::Rejected,
            Self::NeedsWeb => SchoolSessionState::NeedsWeb,
            Self::Unavailable { .. } => SchoolSessionState::Unavailable,
        }
    }
}

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
    /// the sign-in page, which may go through a pause after a failure, never
    /// through a refusal.
    Web { past_pause: bool },
    /// The person's Retry: once through a refusal on record.
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

struct State {
    phase: Phase,
    problem: Option<SchoolSignInProblem>,
    snapshot: Option<Arc<Snapshot>>,
    trust: Trust,
    /// The username the snapshot belongs to.
    account: Option<String>,
    /// Grows with every new snapshot. A site that finds its copy refused
    /// signs in only while this has not moved since it copied it.
    generation: u64,
    flight: Option<Flight>,
    /// The one running verification.
    check: Option<watch::Receiver<Option<Checked>>>,
}

/// The one running sign-in.
struct Flight {
    done: watch::Receiver<Option<Result<u64, Unavailable>>>,
    cancel: CancellationToken,
}

pub(super) struct Identity {
    state: Mutex<State>,
}

impl Identity {
    pub(super) fn new(configured: bool) -> Self {
        Self {
            state: Mutex::new(State {
                phase: if configured {
                    Phase::Idle
                } else {
                    Phase::NotConfigured
                },
                problem: None,
                snapshot: None,
                trust: Trust::Unverified,
                account: None,
                generation: 0,
                flight: None,
                check: None,
            }),
        }
    }

    pub(super) fn generation(&self) -> u64 {
        guard(&self.state).generation
    }

    pub(super) fn account(&self) -> Option<String> {
        guard(&self.state).account.clone()
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

    /// The snapshot a web view may be given: verified, of `generation`, with
    /// the account usable (not refused, waiting for the web, unconfigured,
    /// paused or signing in).
    pub(super) fn writable(&self, generation: u64) -> Option<Arc<Snapshot>> {
        let state = guard(&self.state);
        (state.generation == generation
            && state.phase == Phase::Active
            && matches!(state.trust, Trust::Verified(_)))
        .then(|| state.snapshot.clone())
        .flatten()
    }

    /// The verified generation a web operation may use as it is: verified
    /// less than `VERIFIED_FRESH` ago, with the account usable.
    pub(super) fn fresh(&self) -> Option<u64> {
        let state = guard(&self.state);
        match state.trust {
            Trust::Verified(at)
                if state.phase == Phase::Active
                    && state.snapshot.is_some()
                    && at.elapsed() < VERIFIED_FRESH =>
            {
                Some(state.generation)
            }
            _ => None,
        }
    }

    /// The state a web operation reads before it decides anything.
    pub(super) fn state(&self) -> SchoolSessionState {
        guard(&self.state).phase.state()
    }

    /// Whether a pause after a failure is still running.
    pub(super) fn paused(&self) -> bool {
        matches!(guard(&self.state).phase, Phase::Unavailable { not_before } if Instant::now() < not_before)
    }

    /// The school sent `generation` to its sign-in page: it is no identity
    /// any more, and the next use signs in.
    pub(super) fn invalidate(&self, generation: u64) {
        let mut state = guard(&self.state);
        if state.generation != generation || state.trust == Trust::Invalid {
            return;
        }
        state.trust = Trust::Invalid;
        if matches!(
            state.phase,
            Phase::Active | Phase::Unverified | Phase::Verifying
        ) {
            state.phase = Phase::Idle;
        }
    }

    /// What `store` keeps: the snapshot and whose it is.
    pub(super) fn persisted(&self) -> Option<(Arc<Snapshot>, String)> {
        let state = guard(&self.state);
        Some((state.snapshot.clone()?, state.account.clone()?))
    }

    pub(super) fn view(
        &self,
    ) -> (
        SchoolSessionState,
        Option<SchoolSignInProblem>,
        Option<DateTime<Utc>>,
    ) {
        let state = guard(&self.state);
        let retry_at = match state.phase {
            Phase::Unavailable { not_before } => Some(
                Utc::now()
                    + chrono::Duration::from_std(
                        not_before.saturating_duration_since(Instant::now()),
                    )
                    .unwrap_or_default(),
            ),
            _ => None,
        };
        (state.phase.state(), state.problem.clone(), retry_at)
    }

    /// A snapshot read back from the vault, unverified. `None` when a
    /// sign-in already made a newer one.
    pub(super) fn restore(&self, snapshot: Snapshot, account: String) -> Option<u64> {
        let mut state = guard(&self.state);
        if state.snapshot.is_some() || state.phase == Phase::NotConfigured {
            return None;
        }
        state.generation += 1;
        state.snapshot = Some(Arc::new(snapshot));
        state.trust = Trust::Unverified;
        state.account = Some(account);
        if state.phase == Phase::Idle {
            state.phase = Phase::Unverified;
        }
        Some(state.generation)
    }

    /// Credentials are saved (or not) now.
    pub(super) fn configured(&self, configured: bool) {
        let mut state = guard(&self.state);
        if !configured {
            state.phase = Phase::NotConfigured;
            state.problem = None;
        } else if state.phase == Phase::NotConfigured {
            state.phase = rest(&state);
        }
    }

    /// The saved credentials changed: a refusal no longer holds until the
    /// next sign-in reads them again.
    pub(super) fn lift(&self, configured: bool) {
        let mut state = guard(&self.state);
        if !configured {
            state.phase = Phase::NotConfigured;
            state.problem = None;
            return;
        }
        if matches!(
            state.phase,
            Phase::NotConfigured | Phase::Rejected | Phase::NeedsWeb | Phase::Unavailable { .. }
        ) {
            state.phase = rest(&state);
            state.problem = None;
        }
    }

    /// Lifts a pause that ran out. `true` when it did.
    pub(super) fn expire_pause(&self, now: Instant) -> bool {
        let mut state = guard(&self.state);
        match state.phase {
            Phase::Unavailable { not_before } if now >= not_before => {
                state.phase = rest(&state);
                state.problem = None;
                true
            }
            _ => false,
        }
    }

    /// Drops the account's identity (the barrier).
    pub(super) fn forget(&self) {
        let mut state = guard(&self.state);
        state.snapshot = None;
        state.trust = Trust::Unverified;
        state.account = None;
        state.problem = None;
        state.flight = None;
        state.check = None;
        state.generation += 1;
        if state.phase != Phase::NotConfigured {
            state.phase = Phase::Idle;
        }
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
    /// sign-in page: no further sign-in until the pause ends.
    pub(super) fn anomaly(&self, generation: u64) -> bool {
        let mut state = guard(&self.state);
        if state.generation != generation || matches!(state.phase, Phase::Unavailable { .. }) {
            return false;
        }
        state.phase = Phase::Unavailable {
            not_before: Instant::now() + RETRY_NOT_BEFORE,
        };
        state.problem = Some(problem(
            Kind::Anomaly,
            "the school sent a fresh sign-in back to the sign-in page",
        ));
        true
    }

    fn unavailable(&self, profile: SchoolAuthProfile) -> Unavailable {
        Unavailable {
            profile,
            state: guard(&self.state).phase.state(),
        }
    }
}

/// Where an identity rests when nothing is wrong.
fn rest(state: &State) -> Phase {
    match (&state.snapshot, state.trust) {
        (None, _) | (Some(_), Trust::Invalid) => Phase::Idle,
        (Some(_), Trust::Unverified) => Phase::Unverified,
        (Some(_), Trust::Verified(_)) => Phase::Active,
    }
}

fn problem(kind: Kind, message: impl Into<String>) -> SchoolSignInProblem {
    SchoolSignInProblem {
        kind,
        message: message.into(),
    }
}

/// How a sign-in task ended.
enum End {
    SignedIn {
        snapshot: Snapshot,
        credentials: SchoolCredentials,
    },
    Refused {
        problem: SchoolSignInProblem,
        /// The credentials the refusal is about, to keep it with them.
        credentials: Option<SchoolCredentials>,
    },
    Unavailable(SchoolSignInProblem),
    NotConfigured,
    /// Stopped before the school's verdict, the password not yet sent.
    Stopped,
}

impl Inner {
    /// Makes sure `profile` has an identity newer than `seen`: one that
    /// appeared meanwhile is used as it is; otherwise this joins the running
    /// sign-in or starts one, if the identity's state lets it.
    pub(super) async fn sign_in(
        self: &Arc<Self>,
        profile: SchoolAuthProfile,
        seen: u64,
        how: How,
    ) -> Result<u64, Unavailable> {
        let account = self.account(profile);
        let identity = &account.identity;
        let mut done = {
            let mut state = guard(&identity.state);
            if state.generation > seen
                && state.snapshot.is_some()
                && state.trust != Trust::Invalid
                && !how.lifts_refusal()
            {
                return Ok(state.generation);
            }
            if let Some(flight) = &state.flight {
                flight.done.clone()
            } else {
                let held = match state.phase {
                    Phase::NotConfigured => !matches!(how, How::Connect(_)),
                    Phase::Rejected | Phase::NeedsWeb => !how.lifts_refusal(),
                    Phase::Unavailable { not_before } => {
                        !how.lifts_pause() && Instant::now() < not_before
                    }
                    Phase::Idle
                    | Phase::Unverified
                    | Phase::Verifying
                    | Phase::Active
                    | Phase::SigningIn => false,
                };
                if held {
                    return Err(Unavailable {
                        profile,
                        state: state.phase.state(),
                    });
                }
                let Some(permit) = account.owner.permit() else {
                    return Err(Unavailable {
                        profile,
                        state: state.phase.state(),
                    });
                };
                let (sender, done) = watch::channel(None);
                let cancel = permit.cancel().child_token();
                state.flight = Some(Flight {
                    done: done.clone(),
                    cancel: cancel.clone(),
                });
                state.phase = Phase::SigningIn;
                let inner = self.clone();
                let started = self.tasks.spawn(async move {
                    let result = inner.signing_in(profile, how, permit, cancel).await;
                    let _ = sender.send(Some(result));
                });
                if !started {
                    state.flight = None;
                    state.phase = rest(&state);
                    return Err(Unavailable {
                        profile,
                        state: state.phase.state(),
                    });
                }
                done
            }
        };
        self.publish();
        let ended = match done.wait_for(Option::is_some).await {
            Ok(result) => result.clone(),
            Err(_) => None,
        };
        ended.unwrap_or_else(|| Err(identity.unavailable(profile)))
    }

    /// The sign-in task: runs the flow within `SIGN_IN_DEADLINE`, then
    /// records what it found while still holding the permit.
    async fn signing_in(
        self: Arc<Self>,
        profile: SchoolAuthProfile,
        how: How,
        permit: Permit,
        cancel: CancellationToken,
    ) -> Result<u64, Unavailable> {
        let submitted = AtomicBool::new(false);
        let flow = self.sign_in_flow(profile, &how, &submitted);
        let end = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            end = tokio::time::timeout(SIGN_IN_DEADLINE, flow) => Some(end.ok()),
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
        let result = self.finish_sign_in(profile, end, &permit).await;
        {
            let mut state = guard(&self.account(profile).identity.state);
            state.flight = None;
        }
        drop(permit);
        self.publish();
        result
    }

    async fn sign_in_flow(
        &self,
        profile: SchoolAuthProfile,
        how: &How,
        submitted: &AtomicBool,
    ) -> End {
        let saved = match how {
            How::Connect(credentials) => Some((credentials.clone(), None)),
            How::Background | How::Web { .. } | How::Retry => {
                let auth = self.auth.clone();
                match tokio::task::spawn_blocking(move || auth.sign_in_credentials(profile)).await {
                    Ok(Ok(Some(saved))) => Some((saved.credentials, saved.failure)),
                    Ok(Ok(None)) => None,
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
        let Some((credentials, failure)) = saved else {
            return End::NotConfigured;
        };
        // A refusal kept with exactly these credentials holds them back.
        if let Some(failure) = failure.filter(|_| !how.lifts_refusal()) {
            return End::Refused {
                problem: failure,
                credentials: None,
            };
        }
        if self.offline.load(Ordering::SeqCst) {
            return End::Unavailable(problem(Kind::Network, "the device is offline"));
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
            login::LoginEnd::SignedIn(snapshot) => End::SignedIn {
                snapshot,
                credentials,
            },
            login::LoginEnd::Rejected(kind, message) | login::LoginEnd::NeedsWeb(kind, message) => {
                End::Refused {
                    problem: problem(kind, message),
                    credentials: Some(credentials),
                }
            }
            login::LoginEnd::Unavailable(kind, message) => End::Unavailable(problem(kind, message)),
        }
    }

    /// Applies a sign-in's end to the identity and to what is kept of it.
    async fn finish_sign_in(
        &self,
        profile: SchoolAuthProfile,
        end: End,
        _permit: &Permit,
    ) -> Result<u64, Unavailable> {
        let identity = &self.account(profile).identity;
        match end {
            End::SignedIn {
                snapshot,
                credentials,
            } => {
                let generation = {
                    let mut state = guard(&identity.state);
                    state.generation += 1;
                    state.snapshot = Some(Arc::new(snapshot));
                    state.trust = Trust::Verified(Instant::now());
                    state.account = Some(credentials.username.clone());
                    state.phase = Phase::Active;
                    state.problem = None;
                    state.generation
                };
                log::info!(target: "app_core::school_session", "signed in profile={profile:?} generation={generation}");
                // The school saw every secret these credentials hold: they
                // become the verified, synchronized ones.
                let auth = self.auth.clone();
                let promoted =
                    tokio::task::spawn_blocking(move || auth.promote(profile, &credentials)).await;
                if let Ok(Err(error)) = promoted {
                    log::warn!(target: "app_core::school_session", "verified credentials not saved profile={profile:?}: {error}");
                }
                self.persist(profile).await;
                Ok(generation)
            }
            End::Refused {
                problem,
                credentials,
            } => {
                log::info!(target: "app_core::school_session", "sign-in refused profile={profile:?} kind={:?}", problem.kind);
                {
                    let mut state = guard(&identity.state);
                    state.phase = match problem.kind.state() {
                        SchoolSessionState::NeedsWeb => Phase::NeedsWeb,
                        _ => Phase::Rejected,
                    };
                    state.problem = Some(problem.clone());
                }
                if let Some(credentials) = credentials {
                    let auth = self.auth.clone();
                    let recorded = tokio::task::spawn_blocking(move || {
                        auth.record_failure(profile, &credentials, &problem)
                    })
                    .await;
                    if let Ok(Err(error)) = recorded {
                        log::warn!(target: "app_core::school_session", "refusal not kept profile={profile:?}: {error}");
                    }
                }
                Err(identity.unavailable(profile))
            }
            End::Unavailable(problem) => {
                log::info!(target: "app_core::school_session", "sign-in unavailable profile={profile:?} kind={:?}: {}", problem.kind, problem.message);
                let mut state = guard(&identity.state);
                state.phase = Phase::Unavailable {
                    not_before: Instant::now() + RETRY_NOT_BEFORE,
                };
                state.problem = Some(problem);
                Err(Unavailable {
                    profile,
                    state: SchoolSessionState::Unavailable,
                })
            }
            End::NotConfigured => {
                let mut state = guard(&identity.state);
                state.phase = Phase::NotConfigured;
                state.problem = None;
                Err(Unavailable {
                    profile,
                    state: SchoolSessionState::NotConfigured,
                })
            }
            End::Stopped => {
                let mut state = guard(&identity.state);
                state.phase = rest(&state);
                Err(Unavailable {
                    profile,
                    state: state.phase.state(),
                })
            }
        }
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
                if matches!(state.phase, Phase::Unverified | Phase::Active) {
                    state.phase = Phase::Verifying;
                }
                let inner = self.clone();
                let started = self.tasks.spawn(async move {
                    let checked = inner.checking(profile, snapshot, generation, permit).await;
                    let _ = sender.send(Some(checked));
                });
                if !started {
                    state.check = None;
                    if state.phase == Phase::Verifying {
                        state.phase = rest(&state);
                    }
                    return Checked::Unconfirmed;
                }
                done
            }
        };
        self.publish();
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
                        if matches!(state.phase, Phase::Verifying | Phase::Unavailable { .. }) {
                            state.phase = Phase::Active;
                            state.problem = None;
                        }
                        Checked::Verified(state.generation)
                    }
                    Some(Walked::SignIn) => {
                        state.trust = Trust::Invalid;
                        if state.phase == Phase::Verifying {
                            state.phase = Phase::Idle;
                        }
                        Checked::Expired
                    }
                    Some(Walked::Failed) | None => {
                        if state.phase == Phase::Verifying {
                            state.phase = rest(&state);
                        }
                        Checked::Unconfirmed
                    }
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
