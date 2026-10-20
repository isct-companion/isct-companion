use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{Arc, PoisonError, RwLock};

/// What committed a store write. Shells receive it as the `source` of
/// `workspace-changed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceChangeSource {
    UserIntent,
    ReferencePull,
    ReferencePoll,
    SchoolPull,
    Scheduler,
    Mcp,
    CloudSync,
    CloudRestore,
    /// Settings cleared the school caches or the whole device.
    Reset,
}

impl WorkspaceChangeSource {
    /// `updatedBy` for a row written before the device has an ID.
    pub(super) fn actor(self) -> &'static str {
        match self {
            Self::UserIntent => "workspace",
            Self::ReferencePull | Self::ReferencePoll => "pull-coordinator",
            Self::SchoolPull => "school",
            Self::Scheduler => "scheduler",
            Self::Mcp => "mcp",
            Self::CloudSync | Self::CloudRestore => "sync",
            Self::Reset => "reset",
        }
    }
}

/// Hears every commit that changed workspace rows. Core installs one to
/// publish the change and schedule cloud capture, so no caller of the store
/// has to remember either.
pub trait CommitObserver: Send + Sync {
    /// After the commit, outside the transaction. `revision` is the
    /// user-intent revision when the write was a workspace intent. `tables`
    /// are the tables the transaction inserted into, updated or deleted from
    /// (collected with SQLite's update hook on the write connection),
    /// unqualified names, sorted, never empty.
    fn committed(
        &self,
        source: WorkspaceChangeSource,
        revision: Option<String>,
        tables: &BTreeSet<String>,
    );
}

/// Shared by every clone of one store, so an observer installed after the
/// store was handed to services still hears their writes.
#[derive(Clone, Default)]
pub(super) struct Observer(Arc<RwLock<Option<Arc<dyn CommitObserver>>>>);

impl Observer {
    pub(super) fn set(&self, observer: Arc<dyn CommitObserver>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(observer);
    }

    /// Tells the observer; a commit that changed no table says nothing.
    pub(super) fn committed(
        &self,
        source: WorkspaceChangeSource,
        revision: Option<String>,
        tables: &BTreeSet<String>,
    ) {
        if tables.is_empty() {
            return;
        }
        let observer = self
            .0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(observer) = observer {
            observer.committed(source, revision, tables);
        }
    }
}
