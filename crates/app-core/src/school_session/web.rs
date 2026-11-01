//! The one way Core touches the web view's cookies: writing an account's
//! identity cookies before an App-initiated school navigation, and clearing
//! the school cookies when an account goes. Nothing is ever read back.
//!
//! Every write and clear goes through one queue, in order, each waiting for
//! the platform's completion before the next starts. A clear stamps a new
//! generation; a write stamped with an older one is dropped, so a write that
//! a caller gave up on (`SEED_WAIT`) can never land after the clear that an
//! account switch or a sign-out queued behind it.

use crate::net::BoxFuture;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};
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

/// The shell's write-only port into its web cookie store.
pub trait WebCookieSeed: Send + Sync {
    /// Writes `cookies` in order through the platform's own `Set-Cookie`
    /// parser and resolves once the store has committed every one. A shell
    /// drops a write whose `generation` is older than its last clear.
    fn write(&self, generation: u64, cookies: Vec<WebCookie>) -> BoxFuture<'_, Result<(), String>>;
    /// Removes the school sites' cookies (Android: the whole store and web
    /// storage, which it cannot remove by domain) and resolves when done.
    fn clear(&self, generation: u64) -> BoxFuture<'_, Result<(), String>>;
}

enum Command {
    Write {
        generation: u64,
        cookies: Vec<WebCookie>,
        done: oneshot::Sender<Result<(), String>>,
    },
    Clear {
        generation: u64,
        done: oneshot::Sender<Result<(), String>>,
    },
}

pub(super) struct WebQueue {
    seed: Arc<OnceLock<Arc<dyn WebCookieSeed>>>,
    commands: OnceLock<mpsc::UnboundedSender<Command>>,
    /// The generation the next write is stamped with: the last clear queued.
    generation: AtomicU64,
}

impl Default for WebQueue {
    fn default() -> Self {
        Self {
            seed: Arc::new(OnceLock::new()),
            commands: OnceLock::new(),
            generation: AtomicU64::new(1),
        }
    }
}

impl WebQueue {
    pub(super) fn attach(&self, seed: Arc<dyn WebCookieSeed>) -> bool {
        self.seed.set(seed).is_ok()
    }

    pub(super) fn attached(&self) -> bool {
        self.seed.get().is_some()
    }

    /// The queue's runner starts with the first command, on the caller's
    /// runtime, and lives as long as the queue's sender.
    fn sender(
        &self,
        tasks: &crate::runtime_tasks::RuntimeTasks,
    ) -> Option<&mpsc::UnboundedSender<Command>> {
        if let Some(sender) = self.commands.get() {
            return Some(sender);
        }
        let (sender, mut receiver) = mpsc::unbounded_channel::<Command>();
        let seed = self.seed.clone();
        let started = tasks.spawn(async move {
            let mut cleared = 0_u64;
            while let Some(command) = receiver.recv().await {
                let Some(seed) = seed.get().cloned() else {
                    let done = match command {
                        Command::Write { done, .. } | Command::Clear { done, .. } => done,
                    };
                    let _ = done.send(Err("no web cookie store".into()));
                    continue;
                };
                match command {
                    Command::Write {
                        generation,
                        cookies,
                        done,
                    } => {
                        let result = if generation < cleared {
                            Err("an account change cleared the web store since".into())
                        } else {
                            seed.write(generation, cookies).await
                        };
                        let _ = done.send(result);
                    }
                    Command::Clear { generation, done } => {
                        cleared = cleared.max(generation);
                        let _ = done.send(seed.clear(generation).await);
                    }
                }
            }
        });
        if !started {
            return None;
        }
        let _ = self.commands.set(sender);
        self.commands.get()
    }

    /// Queues `cookies` and returns the wait for their completion.
    pub(super) fn write(
        &self,
        tasks: &crate::runtime_tasks::RuntimeTasks,
        cookies: Vec<WebCookie>,
    ) -> oneshot::Receiver<Result<(), String>> {
        let (done, wait) = oneshot::channel();
        let generation = self.generation.load(Ordering::SeqCst);
        match self.sender(tasks) {
            Some(sender) => {
                if let Err(mpsc::error::SendError(Command::Write { done, .. })) =
                    sender.send(Command::Write {
                        generation,
                        cookies,
                        done,
                    })
                {
                    let _ = done.send(Err("the web cookie queue has stopped".into()));
                }
            }
            None => {
                let _ = done.send(Err("the web cookie queue has stopped".into()));
            }
        }
        wait
    }

    /// Queues a clear under a new generation and waits for it.
    pub(super) async fn clear(
        &self,
        tasks: &crate::runtime_tasks::RuntimeTasks,
    ) -> Result<(), String> {
        if !self.attached() {
            return Ok(());
        }
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (done, wait) = oneshot::channel();
        let sender = self
            .sender(tasks)
            .ok_or_else(|| "the web cookie queue has stopped".to_string())?;
        sender
            .send(Command::Clear { generation, done })
            .map_err(|_| "the web cookie queue has stopped".to_string())?;
        wait.await
            .unwrap_or_else(|_| Err("the web cookie queue has stopped".into()))
    }
}
