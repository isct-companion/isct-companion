#[cfg(test)]
use super::KEY_DEADLINE;
use super::{guard, AccountWait, Dataset, Depth, Key, KeyState};
use crate::{
    data::{DataError, DataStore, WorkspaceChangeSource},
    net::Host,
    school_auth::SchoolAuthProfile,
    sync_state::{self, RunOutcome, RunRecord},
};
use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

// -- Commit gate --------------------------------------------------------------

#[derive(Default)]
struct GateState {
    generation: u64,
    committing: usize,
    /// Moves when an LMS write starts on a key's resource; a lease taken
    /// before it never commits (`PlanEngine::bump_resource`).
    resources: HashMap<Key, u64>,
}

/// Every write of a lease passes here. Closing admission bumps the
/// generation under the same lock, so a commit either entered before the
/// bump (and `wait_idle` waits for it) or is refused; likewise a write that
/// started on the key's resource after the lease was taken.
pub(super) struct Gate {
    state: Mutex<GateState>,
    /// The commits in progress, for `wait_idle`.
    pub(super) committing: watch::Sender<usize>,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            committing: watch::channel(0).0,
        }
    }
}

impl Gate {
    pub(super) fn generation(&self) -> u64 {
        guard(&self.state).generation
    }

    pub(super) fn bump(&self) -> u64 {
        let mut state = guard(&self.state);
        state.generation = state.generation.saturating_add(1);
        state.generation
    }

    pub(super) fn resource(&self, key: &Key) -> u64 {
        guard(&self.state).resources.get(key).copied().unwrap_or(0)
    }

    #[cfg_attr(not(feature = "agent"), allow(dead_code))]
    pub(super) fn bump_resource(&self, key: &Key) -> u64 {
        let mut state = guard(&self.state);
        let revision = state.resources.entry(key.clone()).or_default();
        *revision = revision.saturating_add(1);
        *revision
    }

    fn enter(
        self: &Arc<Self>,
        generation: u64,
        resource: Option<&(Key, u64)>,
    ) -> Option<GateGuard> {
        let mut state = guard(&self.state);
        if state.generation != generation {
            return None;
        }
        if let Some((key, revision)) = resource {
            if state
                .resources
                .get(key)
                .is_some_and(|current| current > revision)
            {
                return None;
            }
        }
        state.committing += 1;
        self.committing.send_replace(state.committing);
        Some(GateGuard(self.clone()))
    }
}

/// Holds the gate for one commit; leaving it may let a reset proceed.
pub(crate) struct GateGuard(Arc<Gate>);

impl Drop for GateGuard {
    fn drop(&mut self) {
        let mut state = guard(&self.0.state);
        state.committing = state.committing.saturating_sub(1);
        self.0.committing.send_replace(state.committing);
    }
}

/// Why a gated write did not happen.
#[derive(Debug)]
pub(crate) enum Gated<E> {
    /// The lease belongs to an earlier generation (a reset ran).
    Stale,
    Failed(E),
}

/// Writes for one lease. Cloneable into blocking workers.
#[derive(Clone)]
pub(crate) struct LeaseWriter {
    pub(super) store: DataStore,
    pub(super) gate: Arc<Gate>,
    pub(super) generation: u64,
    /// The key's resource revision when it was leased.
    pub(super) resource: Option<(Key, u64)>,
}

impl LeaseWriter {
    pub(crate) fn store(&self) -> &DataStore {
        &self.store
    }

    /// Holds the gate while the caller writes through store methods that
    /// open their own transaction. `None`: the lease is stale.
    pub(crate) fn enter(&self) -> Option<GateGuard> {
        self.gate.enter(self.generation, self.resource.as_ref())
    }

    /// One transaction inside the gate.
    pub(crate) fn write<T, E: From<DataError>>(
        &self,
        source: Option<WorkspaceChangeSource>,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, Gated<E>> {
        let Some(_guard) = self.enter() else {
            return Err(Gated::Stale);
        };
        self.store.write(source, work).map_err(Gated::Failed)
    }

    /// One transaction to `year`'s catalog inside the gate
    /// (`DataStore::catalog_write`).
    pub(crate) fn catalog_write<T, E: From<DataError>>(
        &self,
        year: u16,
        work: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, Gated<E>> {
        let Some(_guard) = self.enter() else {
            return Err(Gated::Stale);
        };
        self.store.catalog_write(year, work).map_err(Gated::Failed)
    }
}

// -- Leases -------------------------------------------------------------------

/// The right to read one key, with what its reader needs.
pub(crate) struct Lease {
    pub(crate) key: Key,
    /// Correlates the `sync_runs` rows of one run in the snapshot.
    pub(crate) run: String,
    /// The Isct-Data index of a reference key.
    pub(crate) locator: Option<String>,
    pub(crate) depth: Depth,
    pub(crate) started: DateTime<Utc>,
    /// The key's deadline, counted from its start.
    pub(crate) deadline: tokio::time::Instant,
    pub(crate) cancel: CancellationToken,
    pub(super) writer: LeaseWriter,
}

impl Lease {
    /// The key's resource revision the lease was taken at.
    pub(super) fn revision(&self) -> u64 {
        self.writer
            .resource
            .as_ref()
            .map_or(0, |(_, revision)| *revision)
    }
}

impl Lease {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub(crate) fn writer(&self) -> LeaseWriter {
        self.writer.clone()
    }

    /// The `sync_runs` row of a read that ends now.
    pub(crate) fn run_record<'a>(
        &'a self,
        outcome: RunOutcome,
        changed: usize,
        source: &'a str,
        error_kind: Option<&'a str>,
        finished_at: DateTime<Utc>,
    ) -> RunRecord<'a> {
        RunRecord {
            run: &self.run,
            key: &self.key,
            started_at: self.started,
            finished_at,
            outcome,
            changed,
            requests: None,
            bytes: None,
            source,
            error_kind,
        }
    }

    /// A lease outside any engine, for executor tests.
    #[cfg(test)]
    pub(crate) fn detached(store: &DataStore, key: Key) -> Self {
        Self {
            key,
            run: uuid::Uuid::new_v4().to_string(),
            locator: None,
            depth: Depth::Full,
            started: crate::clock::now(),
            deadline: tokio::time::Instant::now() + KEY_DEADLINE,
            cancel: CancellationToken::new(),
            writer: LeaseWriter {
                store: store.clone(),
                gate: Arc::new(Gate::default()),
                generation: 0,
                resource: None,
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn with_locator(mut self, locator: &str) -> Self {
        self.locator = Some(locator.into());
        self
    }
}

/// How one key read ended. Executors commit complete reads themselves (rows,
/// freshness and the run row in one transaction); every other ending is
/// recorded by the runner.
#[derive(Debug)]
pub(crate) enum KeyEnd {
    Done {
        changed: usize,
    },
    /// Nothing landed; the key backs off.
    Failed {
        kind: String,
        source: String,
        /// What the reader or the projection said, for the run's row.
        reason: Option<String>,
    },
    /// Already recorded by the executor with its own backoff (a public file
    /// held back after shrinking).
    Refused {
        kind: String,
    },
    /// The read could not reach the account's school: no credential on its
    /// path served it and none could be got. Not a failure, no backoff: the
    /// planner holds this key (at this depth) while the credential facts
    /// stay at `version`, the one the sessions decided under
    /// (`Facts::waited`), and derives anew from the facts what else waits.
    Blocked {
        profile: SchoolAuthProfile,
        wait: AccountWait,
        version: u64,
        source: String,
    },
    /// A sign-in the read needed found the shell offline and sent nothing.
    /// Not a failure, no backoff, and the refresh button's press stays: the
    /// planner holds every key while the shell reports no network.
    Offline {
        source: String,
    },
    /// The host's breaker paused it: every key of the host is held, and this
    /// key's own failure count stays.
    HostPaused {
        host: Host,
        until: DateTime<Utc>,
        source: String,
    },
    Cancelled {
        source: String,
    },
    /// The enrolment list no longer (or not yet) holds the course: nothing
    /// landed and nothing backs off. The planner holds the key until a list
    /// that holds the course lands, then reads it at once.
    NotEnrolled {
        source: String,
    },
    /// The lease's generation is gone; nothing was written.
    Stale,
}

impl KeyEnd {
    pub(crate) fn failed(kind: impl Into<String>, source: impl Into<String>) -> Self {
        Self::Failed {
            kind: kind.into(),
            source: source.into(),
            reason: None,
        }
    }

    /// A failure with the reason shown on the run's row. Bounded, since a
    /// storage or client error's text is not ours.
    pub(crate) fn failed_because(
        kind: impl Into<String>,
        source: impl Into<String>,
        reason: &str,
    ) -> Self {
        Self::Failed {
            kind: kind.into(),
            source: source.into(),
            reason: Some(reason.chars().take(MAX_FAILURE_REASON).collect()),
        }
    }

    /// The ending of a gated write that did not happen.
    pub(crate) fn from_gated<E: std::fmt::Display>(
        gated: Gated<E>,
        kind: &str,
        source: &str,
    ) -> Self {
        match gated {
            Gated::Stale => Self::Stale,
            Gated::Failed(error) => {
                log::warn!(target: "app_core::sync", "commit failed: {error}");
                Self::failed(kind, source)
            }
        }
    }
}

/// What the runner records for a read that wrote no rows: the key's
/// backoff (unless the cause was not the key: the account, a paused host, a
/// cancel) and its `sync_runs` row, inside the lease's gate.
pub(crate) struct Ending {
    writer: LeaseWriter,
    key: Key,
    run: String,
    started: DateTime<Utc>,
    source: String,
    record: Option<(RunOutcome, String, bool)>,
}

impl Ending {
    /// Blocking: writes the record and returns the key's state after it.
    pub(crate) fn write(self) -> Option<KeyState> {
        if let Some((outcome, kind, backoff)) = &self.record {
            let now = crate::clock::now();
            let written = self.writer.write(None, |transaction| {
                if *backoff {
                    sync_state::fail(transaction, &self.key, kind, now)?;
                }
                sync_state::record_run(
                    transaction,
                    &RunRecord {
                        run: &self.run,
                        key: &self.key,
                        started_at: self.started,
                        finished_at: now,
                        outcome: *outcome,
                        changed: 0,
                        requests: None,
                        bytes: None,
                        source: &self.source,
                        error_kind: Some(kind),
                    },
                )
            });
            if let Err(Gated::Failed(error)) = written {
                crate::diagnostics::report("sync", &error);
            }
        }
        self.writer
            .store()
            .catalog_key_states(std::slice::from_ref(&self.key))
            .ok()
            .and_then(|mut states| states.pop().flatten())
    }
}

impl Lease {
    pub(crate) fn ending(&self, end: &KeyEnd) -> Ending {
        let source = match end {
            KeyEnd::Failed { source, .. }
            | KeyEnd::Blocked { source, .. }
            | KeyEnd::Offline { source }
            | KeyEnd::HostPaused { source, .. }
            | KeyEnd::Cancelled { source }
            | KeyEnd::NotEnrolled { source } => source.clone(),
            _ => String::new(),
        };
        let source = if source.is_empty() {
            default_source(&self.key, self.locator.as_deref())
        } else {
            source
        };
        let record = match end {
            KeyEnd::Done { .. } | KeyEnd::Refused { .. } | KeyEnd::Stale => None,
            KeyEnd::Failed { kind, .. } => Some((RunOutcome::Failed, kind.clone(), true)),
            // Waiting for the account is not the key's failure: the planner
            // holds the key until its credential facts move.
            KeyEnd::Blocked { .. } => {
                Some((RunOutcome::SignInRequired, "sign-in".to_string(), false))
            }
            KeyEnd::Offline { .. } => Some((RunOutcome::Waiting, "offline".to_string(), false)),
            KeyEnd::HostPaused { .. } => {
                Some((RunOutcome::Failed, "host-paused".to_string(), false))
            }
            KeyEnd::Cancelled { .. } => {
                Some((RunOutcome::Cancelled, "cancelled".to_string(), false))
            }
            KeyEnd::NotEnrolled { .. } => {
                Some((RunOutcome::Waiting, "not-enrolled".to_string(), false))
            }
        };
        Ending {
            writer: self.writer(),
            key: self.key.clone(),
            run: self.run.clone(),
            started: self.started,
            source,
            record,
        }
    }
}

/// Canonical `sync_runs.source` of a key whose reader did not name one.
fn default_source(key: &Key, locator: Option<&str>) -> String {
    match key.dataset {
        Dataset::LmsEnrolment
        | Dataset::LmsDeadlines
        | Dataset::LmsCourse
        | Dataset::LmsAssign
        | Dataset::LmsHistory => crate::net::moodle::source(key.dataset).into(),
        Dataset::KyomuPage => adpt_isct::kyomu::page_url(&key.key)
            .and_then(|url| url::Url::parse(&url).ok())
            .map(|url| sync_state::source_of_url(&url))
            .unwrap_or_default(),
        Dataset::CatalogOfficial => format!(
            "{}{}",
            adpt_isct::kyomu::HOST,
            crate::catalog::KYOMU_SEARCH_PATH
        ),
        Dataset::Reference => locator.unwrap_or_default().into(),
        Dataset::Announcements => crate::announcements::URL.into(),
    }
}

/// Characters of a failure's reason kept for the run's row.
const MAX_FAILURE_REASON: usize = 200;
