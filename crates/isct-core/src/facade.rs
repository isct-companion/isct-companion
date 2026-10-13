//! One application service owner, shared by native shells and agent adapters.
use crate::school_browser::{SchoolBrowser, SchoolBrowserEngine, SchoolBrowserError};
use crate::{
    agent::AgentService,
    data::{DataError, DataStore},
    ports::EventSink,
    pull_coordinator::PullNetwork,
    reference_refresh::ReferenceService,
    runtime::CoreRuntime,
    scheduler::{ScheduledExporter, SchedulerServices},
    school_auth::SchoolAuthService,
    school_service::{SchoolExecutor, SchoolService},
};
use std::sync::{Arc, Mutex};

pub struct Core {
    pub store: DataStore,
    pub runtime: Arc<CoreRuntime>,
    pub auth: Arc<SchoolAuthService>,
    pub agent: AgentService,
    school_browser: Mutex<Option<Arc<SchoolBrowserEngine>>>,
}

impl Core {
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>, network: PullNetwork) -> Self {
        Self {
            agent: AgentService::new(store.clone()),
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

    pub fn school_records(&self) -> Result<serde_json::Value, DataError> {
        let mut records = serde_json::Map::new();
        for (key, table) in [
            ("lmsCourses", "lms_courses"),
            ("registrations", "registrations"),
            ("registrationStates", "registration_states"),
            ("grades", "grades"),
            ("gpaByTerm", "gpa_by_term"),
            ("requirements", "requirement_progress"),
        ] {
            let rows = self
                .store
                .table_rows(table)?
                .into_iter()
                .filter(|row| row.get("deletedAt").is_none_or(serde_json::Value::is_null))
                .collect();
            records.insert(key.into(), serde_json::Value::Array(rows));
        }
        records.insert("lastRuns".into(), self.store.last_import_runs()?);
        Ok(serde_json::Value::Object(records))
    }

    pub async fn shutdown(&self) {
        if let Ok(browser) = self.school_browser() {
            browser.shutdown().await;
        }
        self.runtime.shutdown().await;
    }
}
