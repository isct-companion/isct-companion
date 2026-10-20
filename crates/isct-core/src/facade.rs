//! One application service owner, shared by native shells and agent adapters.
use crate::school_browser::{SchoolBrowser, SchoolBrowserEngine, SchoolBrowserError};
use crate::{
    agent::AgentService,
    data::{CommitObserver, DataStore, WorkspaceChangeSource},
    ports::EventSink,
    runtime::{CloudWakeError, CoreRuntime, RuntimeError},
    scheduler::{ScheduledExporter, SchedulerServices},
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    sync_engine::{Hint, Network, SyncEngine, SyncHints, SyncSnapshot},
    workspace::{WorkspaceError, WorkspaceService},
    workspace_changes::{self, Coalescer, WorkspaceChange, WorkspacePatch, COALESCE_WINDOW},
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

pub struct Core {
    pub store: DataStore,
    pub runtime: Arc<CoreRuntime>,
    pub auth: Arc<SchoolAuthService>,
    pub agent: AgentService,
    pub workspace: WorkspaceService,
    pub reminders: crate::schedule::ReminderService,
    /// Every network read of school and reference data; `Core::reset`
    /// closes its admission.
    pub(crate) sync: Arc<SyncEngine>,
    school_browser: Mutex<Option<Arc<SchoolBrowserEngine>>>,
    events: Option<Arc<CoreEvents>>,
    pub(crate) reset_lease: crate::reset::ResetLease,
}

struct CoreEvents {
    sink: Mutex<Option<Arc<dyn EventSink>>>,
    /// Every event passes the relay below, so sign-in and maintenance facts
    /// reach the sync snapshot once for all shells.
    sync: SyncHints,
}

struct NoopEvents;

impl EventSink for NoopEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        Ok(())
    }
}

impl EventSink for CoreEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        if !matches!(event, crate::events::CoreEvent::SyncSnapshot(_)) {
            self.sync.observe(&event);
        }
        let sink = self
            .sink
            .lock()
            .map_err(|_| crate::ports::EventError::Delivery("Event bridge is unavailable.".into()))?
            .clone();
        let Some(sink) = sink else {
            return Err(crate::ports::EventError::Delivery(
                "Event bridge has stopped.".into(),
            ));
        };
        sink.emit(event)
    }
}

/// Every committed workspace write reaches shells and cloud sync from here,
/// whichever service, executor or agent tool made it.
struct StoreChanges {
    events: Option<(Arc<CoreEvents>, Arc<Coalescer>)>,
    runtime: Arc<CoreRuntime>,
}

impl CommitObserver for StoreChanges {
    fn committed(
        &self,
        source: WorkspaceChangeSource,
        revision: Option<String>,
        tables: &BTreeSet<String>,
    ) {
        if workspace_changes::touches_user_data(tables) {
            self.runtime.request_cloud_capture();
        }
        let Some((events, coalescer)) = &self.events else {
            return;
        };
        let changes = workspace_changes::changes_for(tables);
        if source == WorkspaceChangeSource::UserIntent {
            // The shell that made the intent waits on this event; it is
            // never held back. Events are hints with snapshot recovery; a
            // failed delivery does not undo the committed write.
            let _ = events.emit(crate::events::CoreEvent::WorkspaceChanged {
                source,
                revision,
                changes: changes.into_iter().collect(),
            });
        } else if !changes.is_empty() {
            coalescer.push(source, changes);
        }
    }
}

impl CoreEvents {
    fn clear(&self) {
        if let Ok(mut sink) = self.sink.lock() {
            *sink = None;
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        // The sync driver holds the engine; it ends once its tasks close.
        self.sync.stop();
    }
}

impl Core {
    /// The network starts as `Unknown`; shells send `Hint::Network`.
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>) -> Self {
        Self::build(store, auth, None)
    }

    pub fn with_events(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self::build(store, auth, Some(events))
    }

    fn build(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        events: Option<Arc<dyn EventSink>>,
    ) -> Self {
        let sync = SyncEngine::new(store.clone(), auth.clone(), Network::Unknown);
        let events = events.map(|events| {
            Arc::new(CoreEvents {
                sink: Mutex::new(Some(events)),
                sync: sync.hints(),
            })
        });
        if let Some(events) = &events {
            sync.set_events(events.clone());
        }
        let runtime = Arc::new(CoreRuntime::new(sync.hints()));
        let sync = Arc::new(sync);
        store.observe(Arc::new(StoreChanges {
            events: events.clone().map(|events| {
                let sink = events.clone();
                let coalescer = Coalescer::new(COALESCE_WINDOW, move |source, changes| {
                    let _ = sink.emit(crate::events::CoreEvent::WorkspaceChanged {
                        source,
                        revision: None,
                        changes,
                    });
                });
                (events, coalescer)
            }),
            runtime: runtime.clone(),
        }));
        Self {
            agent: AgentService::new(store.clone()),
            workspace: WorkspaceService::new(store.clone()),
            reminders: crate::schedule::ReminderService::new(store.clone()),
            sync,
            school_browser: Mutex::new(None),
            store,
            auth,
            runtime,
            events,
            reset_lease: Default::default(),
        }
    }

    /// A shell fact or intent for the sync planner. Never blocks.
    pub fn sync_hint(&self, hint: Hint) {
        self.sync.hint(hint);
    }

    /// What sync is doing now. The same value is sent as `sync-snapshot`
    /// whenever it changes; shells read this on start and after a missed
    /// event.
    pub async fn sync_snapshot(&self) -> SyncSnapshot {
        self.sync.snapshot()
    }

    /// Starts the sync driver on the shell's runtime. A hint from inside a
    /// runtime starts it too.
    pub fn start(
        &self,
        handle: &tokio::runtime::Handle,
    ) -> Result<(), crate::runtime_tasks::RuntimeTaskError> {
        self.sync.start(handle)
    }

    /// Attaches the shell's school WebView and the cookie port that lets
    /// Core's HTTP clients read with the WebView's school session.
    pub fn attach_school_browser(
        &self,
        browser: Arc<dyn SchoolBrowser>,
        cookies: Arc<dyn crate::net::SessionCookies>,
    ) -> Result<Arc<SchoolBrowserEngine>, SchoolBrowserError> {
        let mut owner = self
            .school_browser
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?;
        if owner.is_some() {
            return Err(SchoolBrowserError::InvalidInput(
                "School browser is already attached.".into(),
            ));
        }
        let events = self
            .events
            .as_ref()
            .map(|events| events.clone() as Arc<dyn EventSink>)
            .unwrap_or_else(|| Arc::new(NoopEvents));
        let engine =
            SchoolBrowserEngine::new(self.auth.clone(), events, browser, self.sync.hints())?;
        let http = crate::net::Http::new()
            .map_err(|error| SchoolBrowserError::Platform(error.to_string()))?;
        engine.attach_transport(http, cookies)?;
        *owner = Some(engine.clone());
        self.sync.attach_school(engine.clone());
        Ok(engine)
    }

    pub fn school_browser(&self) -> Result<Arc<SchoolBrowserEngine>, SchoolBrowserError> {
        self.school_browser
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .clone()
            .ok_or_else(|| SchoolBrowserError::Platform("School browser is unavailable.".into()))
    }

    pub fn save_credentials(
        &self,
        profile: SchoolAuthProfile,
        credentials: SchoolCredentials,
    ) -> Result<bool, CredentialMutationError> {
        let saved = self.auth.save(profile, credentials)?;
        self.credentials_changed(profile, true)?;
        Ok(saved)
    }

    pub fn delete_credentials(
        &self,
        profile: SchoolAuthProfile,
    ) -> Result<bool, CredentialMutationError> {
        let deleted = self.auth.delete(profile)?;
        self.credentials_changed(profile, false)?;
        Ok(deleted)
    }

    fn credentials_changed(
        &self,
        profile: SchoolAuthProfile,
        configured: bool,
    ) -> Result<(), CredentialMutationError> {
        // The account keys belong to may have changed.
        self.sync.reevaluate();
        let browser = self
            .school_browser
            .lock()
            .map_err(|_| SchoolBrowserError::Stopped)?
            .clone();
        if let Some(browser) = browser {
            browser.credentials_changed(profile, configured)?;
        }
        Ok(())
    }

    /// Ports are scoped to operations, so completed work releases native handles.
    pub fn services(&self, exporter: Arc<dyn ScheduledExporter>) -> SchedulerServices {
        SchedulerServices {
            store: self.store.clone(),
            exporter,
        }
    }

    /// The records `changes` feed, from one snapshot, for a shell that heard
    /// `workspace-changed` and patches its copy instead of reading it all.
    pub fn workspace_records(
        &self,
        changes: &[WorkspaceChange],
    ) -> Result<WorkspacePatch, WorkspaceError> {
        Ok(workspace_changes::patch(
            self.workspace.snapshot()?,
            changes,
        ))
    }

    /// Evaluate a page's related queries against one coherent database snapshot.
    pub fn schedule_query(
        &self,
        queries: Vec<crate::schedule::WorkspaceQuery>,
    ) -> Result<ScheduleQuerySnapshot, ReadModelError> {
        if queries.len() > 64 {
            return Err(ReadModelError::InvalidInput(
                "Too many schedule queries.".into(),
            ));
        }
        let mut snapshot = self.workspace.snapshot()?;
        snapshot.workspace["registrations"] = snapshot.school_records["registrations"].clone();
        snapshot.workspace["grades"] = snapshot.school_records["grades"].clone();
        snapshot.workspace["lmsCourses"] = snapshot.school_records["lmsCourses"].clone();
        let values = queries
            .into_iter()
            .map(|query| crate::schedule::query(&snapshot.workspace, query))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ScheduleQuerySnapshot {
            revision: snapshot.content_revision,
            values,
        })
    }

    pub fn finder_query(
        &self,
        request: crate::course_query::FinderRequest,
    ) -> Result<crate::course_query::FinderView, ReadModelError> {
        let snapshot = self.workspace.snapshot()?;
        let offerings = crate::catalog::offerings(&self.store, request.year)?;
        let mut workspace = snapshot.workspace;
        workspace["kyomuSearchOptions"] = self
            .store
            .kyomu_search_options(request.year)
            .map_err(crate::catalog::CatalogError::from)?;
        Ok(crate::course_query::finder_view(
            &workspace, offerings, request,
        )?)
    }

    pub async fn reminder_tick(
        &self,
        sink: Arc<dyn crate::schedule::NotificationSink>,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<bool, crate::schedule::ReminderError> {
        self.reminders.tick(sink, now).await
    }

    /// Run every periodic domain service once. Each result is retained so a
    /// failed refresh cannot suppress cloud journaling or reminder delivery.
    pub async fn wake(&self, services: WakeServices) -> WakeReport {
        let runtime = self.runtime.wake(services.scheduler).await;
        let cloud = services
            .cloud
            .map(|services| self.runtime.wake_cloud(services));
        let reminders = match services.reminders {
            Some(reminders) => Some(self.reminder_tick(reminders.sink, reminders.now).await),
            None => None,
        };
        WakeReport {
            runtime,
            cloud,
            reminders,
        }
    }

    pub async fn import_legacy_reminders(
        &self,
        fired: serde_json::Value,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<bool, crate::schedule::ReminderError> {
        self.reminders.import_legacy_fired(fired, now).await
    }

    pub async fn shutdown(&self) {
        self.reminders.shutdown().await;
        // Reads stop before the browser they may be signing in with.
        self.sync.shutdown().await;
        if let Ok(browser) = self.school_browser() {
            browser.shutdown().await;
        }
        self.runtime.shutdown().await;
        if let Some(events) = &self.events {
            events.clear();
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialMutationError {
    #[error(transparent)]
    Auth(#[from] SchoolAuthError),
    #[error(transparent)]
    Browser(#[from] SchoolBrowserError),
}

pub struct ReminderWake {
    pub sink: Arc<dyn crate::schedule::NotificationSink>,
    pub now: chrono::DateTime<chrono::FixedOffset>,
}

pub struct WakeServices {
    pub scheduler: SchedulerServices,
    pub cloud: Option<crate::cloud_sync::CloudServices>,
    pub reminders: Option<ReminderWake>,
}

pub struct WakeReport {
    pub runtime: Result<(), RuntimeError>,
    pub cloud: Option<Result<bool, CloudWakeError>>,
    pub reminders: Option<Result<bool, crate::schedule::ReminderError>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadModelError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Schedule(#[from] crate::schedule::ScheduleError),
    #[error(transparent)]
    Finder(#[from] crate::course_query::FinderError),
    #[error(transparent)]
    Catalog(#[from] crate::catalog::CatalogError),
    #[error("{0}")]
    InvalidInput(String),
}

#[derive(serde::Serialize)]
pub struct ScheduleQuerySnapshot {
    pub revision: String,
    pub values: Vec<crate::schedule::WorkspaceQueryResult>,
}

#[cfg(test)]
#[path = "tests/facade.rs"]
mod tests;
