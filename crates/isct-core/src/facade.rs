//! One application service owner, shared by native shells and agent adapters.
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
use std::sync::Arc;

pub struct Core {
    pub store: DataStore,
    pub runtime: Arc<CoreRuntime>,
    pub auth: Arc<SchoolAuthService>,
    pub agent: AgentService,
}

impl Core {
    pub fn new(store: DataStore, auth: Arc<SchoolAuthService>, network: PullNetwork) -> Self {
        Self {
            agent: AgentService::new(store.clone()),
            store,
            auth,
            runtime: Arc::new(CoreRuntime::new(network)),
        }
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
        self.runtime.shutdown().await;
    }
}
