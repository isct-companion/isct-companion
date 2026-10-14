//! One application service owner, shared by native shells and agent adapters.
use crate::school_browser::{SchoolBrowser, SchoolBrowserEngine, SchoolBrowserError};
use crate::{
    agent::AgentService,
    data::DataStore,
    ports::EventSink,
    pull_coordinator::PullNetwork,
    reference_refresh::ReferenceService,
    runtime::CoreRuntime,
    scheduler::{ScheduledExporter, SchedulerServices},
    school_auth::SchoolAuthService,
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
}

impl Core {
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>, network: PullNetwork) -> Self {
        Self {
            agent: AgentService::new(store.clone()),
            workspace: WorkspaceService::new(store.clone()),
            reminders: crate::schedule::ReminderService::new(store.clone()),
            school_browser: Mutex::new(None),
            store,
            auth,
            runtime: Arc::new(CoreRuntime::new(network)),
        }
    }

    pub fn attach_school_browser(
        &self,
        events: Arc<dyn EventSink>,
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

    /// Ports are scoped to operations, so completed work releases native handles.
    pub fn services(
        &self,
        events: Arc<dyn EventSink>,
        executor: Arc<dyn SchoolExecutor>,
        exporter: Arc<dyn ScheduledExporter>,
    ) -> SchedulerServices {
        SchedulerServices {
            references: ReferenceService::new(
                self.store.clone(),
                self.runtime.coordinator.clone(),
                events.clone(),
            ),
            school: SchoolService::new(
                self.runtime.coordinator.clone(),
                self.auth.clone(),
                events.clone(),
            ),
            refresh: self.runtime.refresh.clone(),
            store: self.store.clone(),
            events,
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
    }
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
