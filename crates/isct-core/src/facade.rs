//! One application service owner, shared by native shells and agent adapters.
use crate::school_browser::{SchoolBrowser, SchoolBrowserEngine, SchoolBrowserError};
use crate::{
    agent::AgentService,
    data::{CommitObserver, DataStore, WorkspaceChangeSource},
    ports::EventSink,
    pull_coordinator::PullNetwork,
    reference_refresh::ReferenceService,
    runtime::{CloudWakeError, CoreRuntime, RuntimeError},
    scheduler::{ScheduledExporter, SchedulerServices},
    school_auth::{SchoolAuthError, SchoolAuthProfile, SchoolAuthService, SchoolCredentials},
    school_service::{SchoolExecutor, SchoolService},
    workspace::{WorkspaceError, WorkspaceService},
};
use std::sync::{Arc, Mutex};

pub struct Core {
    pub store: DataStore,
    pub runtime: Arc<CoreRuntime>,
    pub auth: Arc<SchoolAuthService>,
    pub agent: AgentService,
    pub workspace: WorkspaceService,
    pub reminders: crate::schedule::ReminderService,
    school_browser: Mutex<Option<Arc<SchoolBrowserEngine>>>,
    events: Option<Arc<CoreEvents>>,
}

struct CoreEvents(Mutex<Option<Arc<dyn EventSink>>>);

struct NoopEvents;

impl EventSink for NoopEvents {
    fn emit(&self, _event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        Ok(())
    }
}

impl EventSink for CoreEvents {
    fn emit(&self, event: crate::events::CoreEvent) -> Result<(), crate::ports::EventError> {
        let sink = self
            .0
            .lock()
            .map_err(|_| crate::ports::EventError::Delivery("Event bridge is unavailable.".into()))?
            .clone();
        match sink {
            Some(sink) => sink.emit(event),
            None => Err(crate::ports::EventError::Delivery(
                "Event bridge has stopped.".into(),
            )),
        }
    }
}

/// Every committed workspace write reaches shells and cloud sync from here,
/// whichever service, executor or agent tool made it.
struct StoreChanges {
    events: Option<Arc<CoreEvents>>,
    runtime: Arc<CoreRuntime>,
}

impl CommitObserver for StoreChanges {
    fn committed(&self, source: WorkspaceChangeSource, revision: Option<String>) {
        if let Some(events) = &self.events {
            // Events are hints with snapshot recovery; a failed delivery does
            // not undo the committed write.
            let _ = events.emit(crate::events::CoreEvent::WorkspaceChanged { source, revision });
        }
        self.runtime.request_cloud_capture();
    }
}

impl CoreEvents {
    fn clear(&self) {
        if let Ok(mut sink) = self.0.lock() {
            *sink = None;
        }
    }
}

impl Core {
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>, network: PullNetwork) -> Self {
        Self::build(store, auth, network, None)
    }

    pub fn with_events(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        network: PullNetwork,
        events: Arc<dyn EventSink>,
    ) -> Self {
        Self::build(store, auth, network, Some(events))
    }

    fn build(
        store: DataStore,
        auth: Arc<SchoolAuthService>,
        network: PullNetwork,
        events: Option<Arc<dyn EventSink>>,
    ) -> Self {
        let events = events.map(|events| Arc::new(CoreEvents(Mutex::new(Some(events)))));
        let runtime = Arc::new(CoreRuntime::new(network));
        store.observe(Arc::new(StoreChanges {
            events: events.clone(),
            runtime: runtime.clone(),
        }));
        Self {
            agent: AgentService::new(store.clone()),
            workspace: WorkspaceService::new(store.clone()),
            reminders: crate::schedule::ReminderService::new(store.clone()),
            school_browser: Mutex::new(None),
            store,
            auth,
            runtime,
            events,
        }
    }

    /// Start Core-owned event bridges on the shell's runtime. Cores built
    /// without an event sink need no bridge, which keeps unit tests headless.
    pub fn start(
        &self,
        handle: &tokio::runtime::Handle,
    ) -> Result<(), crate::runtime_tasks::RuntimeTaskError> {
        match &self.events {
            Some(events) => self.runtime.start_event_forwarding(events.clone(), handle),
            None => Ok(()),
        }
    }

    pub fn attach_school_browser(
        &self,
        browser: Arc<dyn SchoolBrowser>,
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
        let engine = SchoolBrowserEngine::new(
            self.store.clone(),
            self.auth.clone(),
            events,
            browser,
            self.runtime.refresh.clone(),
        )?;
        *owner = Some(engine.clone());
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
    pub fn services(
        &self,
        executor: Arc<dyn SchoolExecutor>,
        exporter: Arc<dyn ScheduledExporter>,
    ) -> SchedulerServices {
        SchedulerServices {
            references: ReferenceService::new(self.store.clone(), self.runtime.coordinator.clone()),
            school: SchoolService::new(self.runtime.coordinator.clone(), self.auth.clone()),
            refresh: self.runtime.refresh.clone(),
            store: self.store.clone(),
            executor,
            exporter,
        }
    }

    pub fn school_records(&self) -> Result<serde_json::Value, WorkspaceError> {
        Ok(self.workspace.snapshot()?.school_records)
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
        Ok(crate::course_query::finder_view(
            &snapshot.workspace,
            offerings,
            request,
        )?)
    }

    pub fn lms_merge_preview(
        &self,
        scope: &str,
        term: &str,
    ) -> Result<serde_json::Value, ReadModelError> {
        let snapshot = self.workspace.snapshot()?;
        let offerings = self
            .store
            .catalog_search("")
            .map_err(crate::catalog::CatalogError::from)?;
        Ok(crate::workspace::lms_merge_preview(
            &snapshot.workspace,
            &snapshot.school_records,
            scope,
            term,
            &offerings,
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
        let runtime = self
            .runtime
            .wake(services.scheduler, services.school_visible)
            .await;
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
    pub school_visible: bool,
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
