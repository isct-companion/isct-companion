//! An account's identity: its state, its snapshot and the one sign-in that
//! may run for it.
//!
//! Only the identity changes its state. The sign-in belongs to the identity,
//! not to whoever asked: a caller that stops waiting leaves it running for
//! the others; signing out, a reset, an account switch, Core's shutdown and
//! the system taking the background time back end it. Passwords go out only
//! when the identity provider shows its sign-in page, and a refusal is kept
//! with the credentials it is about, so they are not sent again until they
//! change or the person retries.

use super::{
    guard, login, Inner, Permit, SchoolSessionState, SchoolSignInProblem,
    SchoolSignInProblemKind as Kind, Snapshot, Unavailable, RETRY_NOT_BEFORE, SIGN_IN_DEADLINE,
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
    Active,
    Rejected,
    NeedsWeb,
    Unavailable { not_before: Instant },
}

impl Phase {
    fn state(self) -> SchoolSessionState {
        match self {
            Self::NotConfigured => SchoolSessionState::NotConfigured,
            Self::Idle => SchoolSessionState::Idle,
            Self::SigningIn => SchoolSessionState::SigningIn,
            Self::Active => SchoolSessionState::Active,
            Self::Rejected => SchoolSessionState::Rejected,
            Self::NeedsWeb => SchoolSessionState::NeedsWeb,
            Self::Unavailable { .. } => SchoolSessionState::Unavailable,
        }
    }
}

/// Who asked for a sign-in.
#[derive(Clone)]
pub(super) enum How {
    /// A read, a download or a web page found the identity gone.
    Background,
    /// The person's Retry: once through a refusal on record.
    Retry,
    /// The person's Connect with these credentials.
    Connect(SchoolCredentials),
}

impl How {
    fn forced(&self) -> bool {
        !matches!(self, Self::Background)
    }
}

struct State {
    phase: Phase,
    problem: Option<SchoolSignInProblem>,
    snapshot: Option<Arc<Snapshot>>,
    /// The username the snapshot belongs to.
    account: Option<String>,
    /// Grows with every new snapshot. A site that finds its copy refused
    /// signs in only while this has not moved since it copied it.
    generation: u64,
    flight: Option<Flight>,
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
                account: None,
                generation: 0,
                flight: None,
            }),
        }
    }

    pub(super) fn generation(&self) -> u64 {
        guard(&self.state).generation
    }

    pub(super) fn account(&self) -> Option<String> {
        guard(&self.state).account.clone()
    }

    /// The current snapshot and its generation.
    pub(super) fn snapshot(&self) -> Option<(Arc<Snapshot>, u64)> {
        let state = guard(&self.state);
        state
            .snapshot
            .clone()
            .map(|snapshot| (snapshot, state.generation))
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

    /// A snapshot read back from the vault. `None` when a sign-in already
    /// made a newer one.
    pub(super) fn restore(&self, snapshot: Snapshot, account: String) -> Option<u64> {
        let mut state = guard(&self.state);
        if state.snapshot.is_some() || state.phase == Phase::NotConfigured {
            return None;
        }
        state.generation += 1;
        state.snapshot = Some(Arc::new(snapshot));
        state.account = Some(account);
        if state.phase == Phase::Idle {
            state.phase = Phase::Active;
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
        state.account = None;
        state.problem = None;
        state.flight = None;
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
    if state.snapshot.is_some() {
        Phase::Active
    } else {
        Phase::Idle
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
            if state.generation > seen && state.snapshot.is_some() && !how.forced() {
                return Ok(state.generation);
            }
            if let Some(flight) = &state.flight {
                flight.done.clone()
            } else {
                let held = match state.phase {
                    Phase::NotConfigured => !matches!(how, How::Connect(_)),
                    Phase::Rejected | Phase::NeedsWeb => !how.forced(),
                    Phase::Unavailable { not_before } => {
                        !how.forced() && Instant::now() < not_before
                    }
                    Phase::Idle | Phase::Active | Phase::SigningIn => false,
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
            How::Background | How::Retry => {
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
        if let Some(failure) = failure.filter(|_| !how.forced()) {
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
