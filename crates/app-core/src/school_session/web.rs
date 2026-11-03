//! The one way Core touches the web view's cookies: writing an account's
//! verified identity before a web operation opens a school page, and
//! clearing every school website when an account goes. Nothing is read
//! back. A clear always covers every school site, on every platform: the
//! account still signed in gets its identity written again by its next web
//! operation.
//!
//! Every write and clear goes through one queue, in order, and the queue
//! never moves past a call the platform has not answered: a write a caller
//! stopped waiting for (`SEED_WAIT`) still lands before anything queued
//! behind it, never after the clear that follows. The queue alone drops a
//! stale write: the caller closes both accounts' admission before it queues
//! a clear, and a write queued under the old admission, or whose identity
//! is no longer the one to write, is answered without reaching the
//! platform.
//! A call that runs past `WEB_STORE_LIMIT` makes the store unusable until it
//! answers: no write is admitted meanwhile.

use super::{guard, WEB_STORE_LIMIT};
use crate::{
    net::BoxFuture,
    runtime_tasks::RuntimeTasks,
    school_auth::SchoolAuthProfile,
    school_browser::{domain_under, school_domains},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::{mpsc, oneshot};

/// One identity cookie as the school sent it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebCookie {
    /// The response URL it came with.
    pub url: String,
    /// The raw `Set-Cookie` value. A `Domain` keeps its leading dot: a
    /// platform must parse it as a browser receives it, never rebuild it
    /// from properties (that turned `.titech.ac.jp` into a host-only cookie).
    pub set_cookie: String,
}

/// Whether a cookie's or a website data record's domain
/// (`.lms.s.isct.ac.jp`, `isct.ac.jp`) is a school website's, which a clear
/// removes.
pub fn is_school_domain(domain: &str) -> bool {
    school_domains().any(|registrable| domain_under(domain, registrable))
}

/// The shell's write-only port into its web cookie store. A call resolves
/// once the platform confirmed it, however long that takes: Core bounds the
/// wait, never the shell.
pub trait WebCookieSeed: Send + Sync {
    /// Writes `cookies` in order through the platform's own `Set-Cookie`
    /// parser and resolves once the store has committed every one.
    fn write(&self, cookies: Vec<WebCookie>) -> BoxFuture<'_, Result<(), String>>;
    /// Removes every school website's data (`is_school_domain`): cookies,
    /// storage and caches, both accounts' alike. Resolves once done.
    fn clear(&self) -> BoxFuture<'_, Result<(), String>>;
}

/// Why a write or clear did not happen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WebError {
    /// The shell has no web cookie port.
    Unsupported,
    /// A sign-out or reset closed the admission it was queued under, or its
    /// identity is no longer the one to write.
    Stale,
    /// A call ran past `WEB_STORE_LIMIT` and has not answered, or the queue
    /// stopped.
    Unavailable,
    /// The platform's own error.
    Platform(String),
}

impl std::fmt::Display for WebError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("the shell has no web cookie store"),
            Self::Stale => formatter.write_str("a sign-out or reset made the write stale"),
            Self::Unavailable => formatter.write_str("the web cookie store is unavailable"),
            Self::Platform(message) => formatter.write_str(message),
        }
    }
}

/// Whether a write still carries the identity to write, asked right before
/// it reaches the platform.
pub(crate) type Valid = Box<dyn Fn() -> bool + Send>;

enum Command {
    Write {
        profile: SchoolAuthProfile,
        admission: u64,
        cookies: Vec<WebCookie>,
        valid: Valid,
        done: oneshot::Sender<Result<(), WebError>>,
    },
    Clear {
        done: oneshot::Sender<Result<(), WebError>>,
    },
}

#[derive(Default)]
struct QueueState {
    /// Per account (Science, Tokyo Tech): grows when a clear closes its
    /// admission. A write carries the value it was queued under.
    admission: [u64; 2],
    /// Grows once the platform confirmed a clear.
    storage: u64,
    /// A call ran past `WEB_STORE_LIMIT` and has not answered.
    stuck: bool,
}

fn index(profile: SchoolAuthProfile) -> usize {
    match profile {
        SchoolAuthProfile::Science => 0,
        SchoolAuthProfile::TokyoTech => 1,
    }
}

#[derive(Default)]
struct Shared {
    seed: OnceLock<Arc<dyn WebCookieSeed>>,
    state: Mutex<QueueState>,
}

#[derive(Default)]
pub(super) struct WebQueue {
    shared: Arc<Shared>,
    commands: OnceLock<mpsc::UnboundedSender<Command>>,
}

impl WebQueue {
    pub(super) fn attach(&self, seed: Arc<dyn WebCookieSeed>) -> bool {
        self.shared.seed.set(seed).is_ok()
    }

    pub(super) fn attached(&self) -> bool {
        self.shared.seed.get().is_some()
    }

    /// Grows with every clear the platform confirmed.
    pub(super) fn storage(&self) -> u64 {
        guard(&self.shared.state).storage
    }

    /// Whether a write would be admitted now.
    pub(super) fn usable(&self) -> bool {
        self.attached() && !guard(&self.shared.state).stuck
    }

    /// Closes the admission of `profiles`: a write queued before this is
    /// dropped when its turn comes.
    pub(super) fn close_admission(&self, profiles: &[SchoolAuthProfile]) {
        let mut state = guard(&self.shared.state);
        for profile in profiles {
            state.admission[index(*profile)] += 1;
        }
    }

    /// The queue's runner starts with the first command, on the caller's
    /// runtime, and ends with Core's runtime tasks: a call the platform has
    /// already been handed is waited for until it answers or runs past its
    /// limit.
    fn sender(&self, tasks: &RuntimeTasks) -> Option<&mpsc::UnboundedSender<Command>> {
        if let Some(sender) = self.commands.get() {
            return Some(sender);
        }
        let (sender, mut receiver) = mpsc::unbounded_channel::<Command>();
        let shared = self.shared.clone();
        let stop = tasks.clone();
        let started = tasks.spawn(async move {
            loop {
                let command = tokio::select! {
                    biased;
                    _ = stop.closed() => break,
                    command = receiver.recv() => match command {
                        Some(command) => command,
                        None => break,
                    },
                };
                shared.run(command, &stop).await;
            }
            // Commands still queued are answered by their dropped senders.
        });
        if !started {
            return None;
        }
        let _ = self.commands.set(sender);
        self.commands.get()
    }

    /// Queues `cookies` for `profile` and returns the wait for their
    /// completion. `valid` is asked once more right before the platform
    /// gets them.
    pub(super) fn write(
        &self,
        tasks: &RuntimeTasks,
        profile: SchoolAuthProfile,
        cookies: Vec<WebCookie>,
        valid: Valid,
    ) -> oneshot::Receiver<Result<(), WebError>> {
        let (done, wait) = oneshot::channel();
        if !self.attached() {
            let _ = done.send(Err(WebError::Unsupported));
            return wait;
        }
        let admission = {
            let state = guard(&self.shared.state);
            if state.stuck {
                let _ = done.send(Err(WebError::Unavailable));
                return wait;
            }
            state.admission[index(profile)]
        };
        let command = Command::Write {
            profile,
            admission,
            cookies,
            valid,
            done,
        };
        match self.sender(tasks) {
            Some(sender) => {
                if let Err(mpsc::error::SendError(Command::Write { done, .. })) =
                    sender.send(command)
                {
                    let _ = done.send(Err(WebError::Unavailable));
                }
            }
            None => {
                if let Command::Write { done, .. } = command {
                    let _ = done.send(Err(WebError::Unavailable));
                }
            }
        }
        wait
    }

    /// Queues a clear of every school website and returns the wait for its
    /// completion. Close both accounts' admission first.
    pub(super) fn clear(&self, tasks: &RuntimeTasks) -> oneshot::Receiver<Result<(), WebError>> {
        let (done, wait) = oneshot::channel();
        if !self.attached() {
            let _ = done.send(Err(WebError::Unsupported));
            return wait;
        }
        match self.sender(tasks) {
            Some(sender) => {
                if let Err(mpsc::error::SendError(Command::Clear { done })) =
                    sender.send(Command::Clear { done })
                {
                    let _ = done.send(Err(WebError::Unavailable));
                }
            }
            None => {
                let _ = done.send(Err(WebError::Unavailable));
            }
        }
        wait
    }
}

impl Shared {
    async fn run(&self, command: Command, stop: &RuntimeTasks) {
        let Some(seed) = self.seed.get().cloned() else {
            let done = match command {
                Command::Write { done, .. } | Command::Clear { done, .. } => done,
            };
            let _ = done.send(Err(WebError::Unsupported));
            return;
        };
        match command {
            Command::Write {
                profile,
                admission,
                cookies,
                valid,
                done,
            } => {
                let admitted = {
                    let state = guard(&self.state);
                    state.admission[index(profile)] == admission && !state.stuck
                };
                if !admitted || !valid() {
                    let _ = done.send(Err(WebError::Stale));
                    return;
                }
                let result = self.answer(seed.write(cookies), stop).await;
                let _ = done.send(result);
            }
            Command::Clear { done } => {
                let result = self.answer(seed.clear(), stop).await;
                if result.is_ok() {
                    guard(&self.state).storage += 1;
                }
                let _ = done.send(result);
            }
        }
    }

    /// The platform's answer to a call it was handed. Within
    /// `WEB_STORE_LIMIT` it is awaited as it is, shutdown included. Past it
    /// the store is unusable until the answer comes; the queue still waits
    /// for it, so nothing queued behind can overtake it, unless Core is
    /// shutting down.
    async fn answer(
        &self,
        call: BoxFuture<'_, Result<(), String>>,
        stop: &RuntimeTasks,
    ) -> Result<(), WebError> {
        tokio::pin!(call);
        let answered = tokio::select! {
            answer = &mut call => Some(answer),
            _ = tokio::time::sleep(WEB_STORE_LIMIT) => None,
        };
        let answer = match answered {
            Some(answer) => answer,
            None => {
                log::warn!(target: "app_core::school_session", "the web cookie store did not answer in time; no writes until it does");
                guard(&self.state).stuck = true;
                let answer = tokio::select! {
                    biased;
                    answer = &mut call => answer,
                    _ = stop.closed() => return Err(WebError::Unavailable),
                };
                guard(&self.state).stuck = false;
                log::info!(target: "app_core::school_session", "the web cookie store answered late");
                answer
            }
        };
        answer.map_err(WebError::Platform)
    }
}
