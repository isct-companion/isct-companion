//! Settings → "Clear school caches and resync" and "Full reset" (`docs/data-ownership.md`).
//!
//! A reset is a Core operation with one lease: it closes the planner's
//! admission (which cancels in-flight reads and moves the generation on, so
//! no task leased before the reset can commit afterwards), waits a bounded
//! time for commits already running, clears, reopens admission and only then
//! tells shells the workspace changed. The planner then finds every key
//! missing and reads them again in cold-start order.
//!
//! What each scope deletes is decided here and in `data/reset_store.rs`,
//! following `docs/data-ownership.md`. Only this device is affected: no
//! tombstone is written, so cloud copies and other devices keep their data.
//!
//! Cloud sync is stopped by the shell before a full reset, through
//! `CloudSync::prepare_reset`, because only the shell holds the cloud
//! transport. A school-cache reset leaves cloud sync running: school caches
//! never enter it, and it only writes user rows this reset keeps.
//!
//! The school caches are cache.db and sync.db, emptied whole
//! (`DataStore::rebuild_cache`); a full reset also empties user.db. User
//! writes need no separate gate.
//! Each database step is one transaction on the store's write connection,
//! so an edit commits wholly before or after it, and every intent is
//! revision-checked: an edit made against the data a full reset removed is
//! refused as a conflict.

use crate::{
    course_materials::purge_downloads,
    data::{DataError, DataStore, WorkspaceChangeSource},
    net::BoxFuture,
    school_auth::{CredentialAvailability, SchoolAuthProfile, SchoolAuthService},
    school_browser::SchoolBrowserEngine,
    sync_engine::{ResetReport, ResetScope, SyncEngine},
    Core,
};
use futures_util::future::{FutureExt, Shared};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

const PROFILES: [SchoolAuthProfile; 2] = [SchoolAuthProfile::Science, SchoolAuthProfile::TokyoTech];

/// How long a reset waits for commits that already started. A commit still
/// running afterwards belongs to the old generation and is refused by the
/// commit gate, so waiting longer would only delay the reset.
const IDLE_LIMIT: Duration = Duration::from_secs(30);

/// What a shell lends a reset: the files and the school WebView store live
/// on the platform side.
pub trait ResetDevice: Send + Sync {
    /// The course folder. Downloaded files live in its `materials` folders,
    /// next to the notes folders a reset never touches.
    fn course_files_root(&self) -> Option<PathBuf>;
    /// Whether this shell can sign the school WebView store out.
    fn can_sign_out_school_web(&self) -> bool;
    /// Removes the cookies and sessions of the school WebView store, so the
    /// next school read signs in again. Saved credentials stay.
    fn sign_out_school_web(&self) -> BoxFuture<'_, Result<(), String>>;
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum ResetError {
    /// Another kind of reset is running; this one did not start.
    #[error("Another reset is already running.")]
    Busy,
    #[error("{0} cannot be reset on this device.")]
    Unsupported(&'static str),
    #[error("Local data could not be cleared: {0}")]
    Storage(String),
    #[error("Saved sign-in data could not be removed: {0}")]
    Credentials(String),
    #[error("Downloaded course files could not be removed: {0}")]
    Files(String),
    #[error("School web pages could not be signed out: {0}")]
    SignOut(String),
    #[error("The reset stopped before it finished.")]
    Stopped,
}

impl From<DataError> for ResetError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

type Flight = Shared<futures_util::future::BoxFuture<'static, Result<ResetReport, ResetError>>>;

/// The one reset lease of a Core. A second call with the same scope joins
/// the running reset and receives its result.
#[derive(Default)]
pub struct ResetLease(Mutex<Option<(ResetScope, Flight)>>);

impl Core {
    /// Resets without shell ports. A scope that needs the course folder or
    /// the WebView store fails with [`ResetError::Unsupported`] before
    /// anything is cleared.
    pub async fn reset(&self, scope: ResetScope) -> Result<ResetReport, ResetError> {
        self.reset_with(scope, None).await
    }

    /// One reset at a time: a repeat with the same scope joins the running
    /// reset; a different scope is refused with [`ResetError::Busy`]. The
    /// reset runs as its own task, so a caller that stops waiting does not
    /// abort it halfway.
    pub async fn reset_with(
        &self,
        scope: ResetScope,
        device: Option<Arc<dyn ResetDevice>>,
    ) -> Result<ResetReport, ResetError> {
        let flight = self.reset_flight(scope, device)?;
        let result = flight.clone().await;
        if let Ok(mut slot) = self.reset_lease.0.lock() {
            if slot
                .as_ref()
                .is_some_and(|(_, running)| running.ptr_eq(&flight))
            {
                *slot = None;
            }
        }
        result
    }

    /// What a reset of `scope` would remove now, for the confirmation panel.
    /// `freed_bytes` counts database pages and downloaded files; it is an
    /// estimate, and a reset clears what exists when it runs.
    pub async fn reset_preview(
        &self,
        scope: ResetScope,
        device: Option<Arc<dyn ResetDevice>>,
    ) -> Result<ResetReport, ResetError> {
        let store = self.store.clone();
        let everything = matches!(scope, ResetScope::Everything);
        let root = match scope {
            ResetScope::SchoolCaches {
                delete_downloads: true,
                ..
            } => Some(
                device
                    .as_ref()
                    .and_then(|device| device.course_files_root())
                    .ok_or(ResetError::Unsupported("Downloaded course files"))?,
            ),
            _ => None,
        };
        blocking(move || {
            let cleared = store.reset_estimate(everything)?;
            let files = match root {
                Some(root) => {
                    purge_downloads(&root, false)
                        .map_err(|error| ResetError::Files(error.to_string()))?
                        .bytes
                }
                None => 0,
            };
            Ok(ResetReport {
                removed_rows: cleared.rows,
                freed_bytes: cleared.bytes + files,
            })
        })
        .await
    }

    fn reset_flight(
        &self,
        scope: ResetScope,
        device: Option<Arc<dyn ResetDevice>>,
    ) -> Result<Flight, ResetError> {
        let mut slot = self.reset_lease.0.lock().map_err(|_| ResetError::Stopped)?;
        // Poll once: if every caller left, the owned task can be finished
        // while the shared future has never observed its result.
        if let Some((running, flight)) = slot
            .as_ref()
            .filter(|(_, flight)| flight.clone().now_or_never().is_none())
        {
            return if *running == scope {
                Ok(flight.clone())
            } else {
                Err(ResetError::Busy)
            };
        }
        let run = ResetRun {
            scope,
            store: self.store.clone(),
            auth: self.auth.clone(),
            engine: self.reset_engine(),
            browser: self.school_browser().ok(),
            device,
        };
        run.check()?;
        let task = tokio::spawn(run.execute());
        let flight = async move { task.await.unwrap_or(Err(ResetError::Stopped)) }
            .boxed()
            .shared();
        *slot = Some((scope, flight.clone()));
        Ok(flight)
    }

    /// The planner whose admission a reset closes.
    fn reset_engine(&self) -> Arc<SyncEngine> {
        self.sync.clone()
    }
}

struct ResetRun {
    scope: ResetScope,
    store: DataStore,
    auth: Arc<SchoolAuthService>,
    engine: Arc<SyncEngine>,
    browser: Option<Arc<SchoolBrowserEngine>>,
    device: Option<Arc<dyn ResetDevice>>,
}

impl ResetRun {
    /// Refuses a scope this shell cannot carry out before anything closes.
    fn check(&self) -> Result<(), ResetError> {
        if let ResetScope::SchoolCaches {
            sign_out_web,
            delete_downloads,
        } = self.scope
        {
            let device = self.device.as_ref();
            if sign_out_web && !device.is_some_and(|device| device.can_sign_out_school_web()) {
                return Err(ResetError::Unsupported("School web sign-in"));
            }
            if delete_downloads
                && device
                    .and_then(|device| device.course_files_root())
                    .is_none()
            {
                return Err(ResetError::Unsupported("Downloaded course files"));
            }
        }
        Ok(())
    }

    async fn execute(self) -> Result<ResetReport, ResetError> {
        let admission = self.engine.close_admission().await;
        if !self.engine.wait_idle(IDLE_LIMIT).await {
            log::warn!(
                target: "app_core::reset",
                "Commits were still running after {} s; the new generation refuses them",
                IDLE_LIMIT.as_secs()
            );
        }
        let result = self.clear().await;
        drop(admission);
        // Announced even after a later step failed: a database step may
        // already have committed.
        self.store.announce(WorkspaceChangeSource::Reset);
        match &result {
            Ok(report) => log::info!(
                target: "app_core::reset",
                "Reset removed {} rows and about {} bytes",
                report.removed_rows,
                report.freed_bytes
            ),
            Err(error) => log::warn!(target: "app_core::reset", "Reset failed: {error}"),
        }
        result
    }

    async fn clear(&self) -> Result<ResetReport, ResetError> {
        let everything = matches!(self.scope, ResetScope::Everything);
        let store = self.store.clone();
        let cleared = blocking(move || {
            Ok(if everything {
                store.clear_everything()
            } else {
                store.clear_school_caches()
            }?)
        })
        .await?;
        let mut report = ResetReport {
            removed_rows: cleared.rows,
            freed_bytes: cleared.bytes,
        };
        self.forget_secrets(everything).await?;
        if let ResetScope::SchoolCaches {
            delete_downloads: true,
            ..
        } = self.scope
        {
            let root = self
                .device
                .as_ref()
                .and_then(|device| device.course_files_root())
                .ok_or(ResetError::Unsupported("Downloaded course files"))?;
            let removed = blocking(move || {
                purge_downloads(&root, true).map_err(|error| ResetError::Files(error.to_string()))
            })
            .await?;
            report.freed_bytes += removed.bytes;
        }
        // A full reset is the first-launch state, so the school web session
        // goes too whenever the shell can remove it.
        let sign_out = match self.scope {
            ResetScope::SchoolCaches { sign_out_web, .. } => sign_out_web,
            ResetScope::Everything => self
                .device
                .as_ref()
                .is_some_and(|device| device.can_sign_out_school_web()),
        };
        if sign_out {
            if let Some(device) = &self.device {
                device
                    .sign_out_school_web()
                    .await
                    .map_err(ResetError::SignOut)?;
            }
        }
        self.restart_sessions(everything || sign_out);
        Ok(report)
    }

    /// The Moodle token is a school cache: the next LMS read launches a new
    /// one. A full reset also removes the saved school credentials.
    async fn forget_secrets(&self, everything: bool) -> Result<(), ResetError> {
        let auth = self.auth.clone();
        blocking(move || {
            for profile in PROFILES {
                let result = if everything {
                    auth.delete(profile).map(|_| ())
                } else {
                    auth.delete_moodle_token(profile)
                };
                result.map_err(|error| ResetError::Credentials(error.to_string()))?;
            }
            Ok(())
        })
        .await
    }

    /// After the web session or the credentials went, the school browser
    /// must not keep reporting a live session: it signs in again first.
    fn restart_sessions(&self, changed: bool) {
        let Some(browser) = self.browser.as_ref().filter(|_| changed) else {
            return;
        };
        let status = match self.scope {
            ResetScope::Everything => None,
            ResetScope::SchoolCaches { .. } => self.auth.status().ok(),
        };
        for profile in PROFILES {
            let configured = status.as_ref().is_some_and(|status| {
                let summary = match profile {
                    SchoolAuthProfile::Science => &status.science,
                    SchoolAuthProfile::TokyoTech => &status.tokyo_tech,
                };
                summary.availability != CredentialAvailability::NotConfigured
            });
            if let Err(error) = browser.credentials_changed(profile, configured) {
                log::warn!(target: "app_core::reset", "School session could not restart: {error}");
            }
        }
    }
}

/// Database, vault and file work runs on a blocking worker.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ResetError> + Send + 'static,
) -> Result<T, ResetError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ResetError::Stopped)?
}

#[cfg(test)]
#[path = "tests/reset.rs"]
mod tests;
