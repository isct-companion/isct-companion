//! The account owner: every change of an account's session passes it.
//!
//! An operation first takes a permit stamped with the account generation;
//! it stores rows, saves jars, renames a download or promotes credentials
//! only while it holds one. Switching accounts, signing out or a reset runs
//! the barrier: stop issuing permits, cancel the reads (writes run on to
//! their end), wait until every permit is back, clean up in a fixed order,
//! and only then open the next generation. No lock spans the database, the
//! vault and the platform's cookie store, so the order is the guarantee.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct State {
    generation: u64,
    open: bool,
    permits: usize,
    /// Cancelled by the barrier: the reads and the sign-in of this generation.
    cancel: CancellationToken,
}

struct Shared {
    state: Mutex<State>,
    returned: Notify,
    /// One barrier at a time.
    turn: tokio::sync::Mutex<()>,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Clone)]
pub(super) struct Owner(Arc<Shared>);

/// The right to change one account's session, for one generation.
pub(crate) struct Permit {
    shared: Arc<Shared>,
    generation: u64,
    cancel: CancellationToken,
}

impl Permit {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    /// Cancelled when the barrier stops this generation's reads.
    pub(crate) fn cancel(&self) -> &CancellationToken {
        &self.cancel
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        {
            let mut state = self.shared.state();
            state.permits = state.permits.saturating_sub(1);
        }
        self.shared.returned.notify_waiters();
    }
}

/// The barrier in progress: dropping it opens the next generation.
pub(super) struct Closed<'a> {
    owner: &'a Owner,
    _turn: tokio::sync::MutexGuard<'a, ()>,
}

impl Drop for Closed<'_> {
    fn drop(&mut self) {
        let mut state = self.owner.0.state();
        state.generation = state.generation.saturating_add(1);
        state.cancel = CancellationToken::new();
        state.open = true;
    }
}

impl Owner {
    pub(super) fn new() -> Self {
        Self(Arc::new(Shared {
            state: Mutex::new(State {
                generation: 1,
                open: true,
                permits: 0,
                cancel: CancellationToken::new(),
            }),
            returned: Notify::new(),
            turn: tokio::sync::Mutex::new(()),
        }))
    }

    pub(super) fn generation(&self) -> u64 {
        self.0.state().generation
    }

    /// A permit for the current generation; `None` while a barrier runs.
    pub(super) fn permit(&self) -> Option<Permit> {
        let mut state = self.0.state();
        if !state.open {
            return None;
        }
        state.permits += 1;
        Some(Permit {
            shared: self.0.clone(),
            generation: state.generation,
            cancel: state.cancel.clone(),
        })
    }

    /// Stops issuing permits, cancels this generation's reads and sign-in,
    /// and waits until every permit is back. Writes are not cancelled: the
    /// wait lasts until they end.
    pub(super) async fn close(&self) -> Closed<'_> {
        let turn = self.0.turn.lock().await;
        {
            let mut state = self.0.state();
            state.open = false;
            state.cancel.cancel();
        }
        loop {
            let returned = self.0.returned.notified();
            tokio::pin!(returned);
            // Registered before the count is read, so a permit returned in
            // between still wakes this wait.
            returned.as_mut().enable();
            if self.0.state().permits == 0 {
                break;
            }
            returned.await;
        }
        Closed {
            owner: self,
            _turn: turn,
        }
    }
}
