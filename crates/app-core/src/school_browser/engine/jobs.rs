//! One owned session job per profile. Every way into a school session — HTTP
//! reads, downloads, launch and focus checks, shell wakes, a connect with new
//! credentials — joins the job that is running for the profile or starts one.
//! The job runs on the engine's own tasks, so a caller that stops waiting (a
//! timeout, a dropped request) never stops the sign-in it started, and never
//! leaves it holding the lane with nobody to drive it.
use super::*;
use tokio::sync::watch;

/// How far a job takes the session. A service session is a portal session
/// whose target service (LMS, 教務Web) has also set its own cookie.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Reach {
    Portal,
    Service,
}

pub(super) enum Work {
    Check(Reach),
    /// Signs in with credentials the person just entered. It takes the
    /// profile over from any job running for it.
    Connect(SchoolCredentials),
}

impl Work {
    fn reach(&self) -> Reach {
        match self {
            Self::Check(reach) => *reach,
            Self::Connect(_) => Reach::Portal,
        }
    }
}

/// A job's answer, with how far the job that gave it took the session.
type Outcome = Option<(Reach, Result<bool, SchoolBrowserError>)>;
type Answer = watch::Receiver<Outcome>;

pub(super) struct SessionJob {
    id: u64,
    done: Answer,
    /// A connect that took the profile over: this job drops its own attempt
    /// and its waiters get the connect's answer.
    successor: watch::Sender<Option<Answer>>,
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
            let (done, _) = self.join_or_start(profile, Work::Check(reach))?;
            let (reached, result) = outcome(done).await;
            // A portal check or a connect that was already running when a
            // service session was asked for: go on to the service once the
            // portal is live.
            if reached >= reach || !matches!(result, Ok(true)) {
                return result;
            }
        }
    }

    /// Starts a portal job unless one is running. `false` when it joined.
    pub(super) fn start_session(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, SchoolBrowserError> {
        self.join_or_start(profile, Work::Check(Reach::Portal))
            .map(|(_, started)| started)
    }

    /// Signs in with `credentials` as the profile's job. `Ok(true)` once the
    /// school's own page is reached, `Ok(false)` when the school turned the
    /// sign-in down.
    pub(super) async fn connect_job(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<bool, SchoolBrowserError> {
        let (done, _) = self.join_or_start(profile, Work::Connect(credentials))?;
        outcome(done).await.1
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
        work: Work,
    ) -> Result<(Answer, bool), SchoolBrowserError> {
        let reach = work.reach();
        let (id, sender, done, successor) = {
            let mut state = self.0.state.guard();
            let connect = matches!(work, Work::Connect(_));
            if let Some(job) = state.jobs.get(&profile).filter(|_| !connect) {
                return Ok((job.done.clone(), false));
            }
            let id = self.0.next_job.fetch_add(1, Ordering::Relaxed);
            let (sender, done) = watch::channel(None);
            let (successor_sender, successor) = watch::channel(None);
            let previous = state.jobs.insert(
                profile,
                SessionJob {
                    id,
                    done: done.clone(),
                    successor: successor_sender,
                },
            );
            if let Some(previous) = previous {
                previous.successor.send_replace(Some(done.clone()));
            }
            if connect {
                supersede(&mut state, profile);
            }
            (id, sender, done, successor)
        };
        let deadline = Deadline::admitted();
        let engine = self.clone();
        let _entered = self.0.runtime.enter();
        let spawned = self.0.tasks.spawn(async move {
            let mut deadline = deadline;
            let body = async {
                match work {
                    Work::Check(Reach::Portal) => {
                        engine.portal_session(profile, &mut deadline).await
                    }
                    Work::Check(Reach::Service) => {
                        engine.service_session(profile, &mut deadline).await
                    }
                    Work::Connect(credentials) => {
                        engine
                            .connect_session(profile, credentials, &mut deadline)
                            .await
                    }
                }
            };
            // Dropping a superseded body ends its attempt and frees the lane.
            let answer = tokio::select! {
                result = body => (reach, result),
                next = superseded(successor) => outcome(next).await,
            };
            // Leave the slot before answering, so a waiter that asks again
            // on this answer starts a fresh job instead of rejoining this one.
            engine.0.state.with(|state| {
                if state.jobs.get(&profile).is_some_and(|job| job.id == id) {
                    state.jobs.remove(&profile);
                }
            });
            let _ = sender.send(Some(answer));
        });
        if !spawned {
            self.0.state.with(|state| {
                if state.jobs.get(&profile).is_some_and(|job| job.id == id) {
                    state.jobs.remove(&profile);
                }
            });
            return Err(SchoolBrowserError::Stopped);
        }
        Ok((done, true))
    }
}

/// A connect takes `profile` over: no result of an earlier attempt may
/// publish from here on, and the hidden check it replaces stops driving the
/// profile's sign-in at once rather than when its task notices.
fn supersede(state: &mut State, profile: SchoolAuthProfile) {
    begin_attempt(state, profile);
    if state
        .heartbeat
        .is_some_and(|heartbeat| heartbeat.profile == profile)
    {
        state.heartbeat = None;
    }
    let visible = state.browsing;
    if state
        .login
        .as_ref()
        .is_some_and(|login| login.profile == profile && Some(login.browser.site) != visible)
    {
        state.login = None;
    }
}

/// The answer of the connect that took this job's profile over.
async fn superseded(mut successor: watch::Receiver<Option<Answer>>) -> Answer {
    if let Ok(next) = successor.wait_for(Option::is_some).await {
        if let Some(next) = next.clone() {
            return next;
        }
    }
    // The job finished first and its slot went away: nobody takes it over.
    std::future::pending().await
}

/// The job's answer. A job that ended without one was stopped with the engine.
async fn outcome(mut done: Answer) -> (Reach, Result<bool, SchoolBrowserError>) {
    loop {
        if let Some(answer) = done.borrow_and_update().clone() {
            return answer;
        }
        if done.changed().await.is_err() {
            return done
                .borrow()
                .clone()
                .unwrap_or((Reach::Portal, Err(SchoolBrowserError::Stopped)));
        }
    }
}
