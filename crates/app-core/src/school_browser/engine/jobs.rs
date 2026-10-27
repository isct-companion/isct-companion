//! One owned session job per profile. Every way into a school session — HTTP
//! reads, downloads, launch and focus checks, shell wakes — joins the job
//! that is running for the profile or starts one. The job runs on the
//! engine's own tasks, so a caller that stops waiting (a timeout, a dropped
//! request) never stops the sign-in it started, and never leaves it holding
//! the lane with nobody to drive it.
use super::*;
use tokio::sync::watch;

/// How far a job takes the session. A service session is a portal session
/// whose target service (LMS, 教務Web) has also set its own cookie.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Reach {
    Portal,
    Service,
}

type Outcome = Option<Result<bool, SchoolBrowserError>>;

pub(super) struct SessionJob {
    id: u64,
    reach: Reach,
    done: watch::Receiver<Outcome>,
}

impl SchoolBrowserEngine {
    /// Waits for a session that reaches at least `reach`. `Ok(false)` when
    /// the school wants the person (or no saved sign-in can run).
    pub(super) async fn session(
        &self,
        profile: SchoolAuthProfile,
        reach: Reach,
    ) -> Result<bool, SchoolBrowserError> {
        loop {
            let (done, covers, _) = self.join_or_start(profile, reach)?;
            let result = outcome(done).await;
            // A portal check that was already running when a service session
            // was asked for: go on to the service once the portal is live.
            if covers || !matches!(result, Ok(true)) {
                return result;
            }
        }
    }

    /// Starts a portal job unless one is running. `false` when it joined.
    pub(super) fn start_session(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        self.join_or_start(profile, Reach::Portal)
            .map(|(_, _, started)| started)
    }

    /// Waits for the jobs already running for these profiles, starting none.
    pub(super) async fn join_running(&self, profiles: &[SchoolAuthProfile]) {
        let running: Vec<_> = self.0.state.with(|state| {
            profiles
                .iter()
                .filter_map(|profile| state.jobs.get(profile).map(|job| job.done.clone()))
                .collect()
        });
        for done in running {
            let _ = outcome(done).await;
        }
    }

    fn join_or_start(
        &self,
        profile: SchoolAuthProfile,
        reach: Reach,
    ) -> Result<(watch::Receiver<Outcome>, bool, bool), SchoolBrowserError> {
        let (id, sender, done) = {
            let mut state = self.0.state.guard();
            if let Some(job) = state.jobs.get(&profile) {
                return Ok((job.done.clone(), job.reach >= reach, false));
            }
            let id = self.0.next_job.fetch_add(1, Ordering::Relaxed);
            let (sender, done) = watch::channel(None);
            state.jobs.insert(
                profile,
                SessionJob {
                    id,
                    reach,
                    done: done.clone(),
                },
            );
            (id, sender, done)
        };
        let engine = self.clone();
        let _entered = self.0.runtime.enter();
        let spawned = self.0.tasks.spawn(async move {
            let result = match reach {
                Reach::Portal => engine.portal_session(profile).await,
                Reach::Service => engine.service_session(profile).await,
            };
            // Leave the slot before answering, so a waiter that asks again
            // on this answer starts a fresh job instead of rejoining this one.
            engine.0.state.with(|state| {
                if state.jobs.get(&profile).is_some_and(|job| job.id == id) {
                    state.jobs.remove(&profile);
                }
            });
            let _ = sender.send(Some(result));
        });
        if !spawned {
            self.0.state.with(|state| {
                if state.jobs.get(&profile).is_some_and(|job| job.id == id) {
                    state.jobs.remove(&profile);
                }
            });
            return Err(SchoolBrowserError::Stopped);
        }
        Ok((done, true, true))
    }
}

/// The job's answer. A job that ended without one was stopped with the engine.
async fn outcome(mut done: watch::Receiver<Outcome>) -> Result<bool, SchoolBrowserError> {
    loop {
        if let Some(result) = done.borrow_and_update().clone() {
            return result;
        }
        if done.changed().await.is_err() {
            return done
                .borrow()
                .clone()
                .unwrap_or(Err(SchoolBrowserError::Stopped));
        }
    }
}
