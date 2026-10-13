//! Desktop delivery adapters for shared domain services.
use crate::{data::DataStore, pull_coordinator::PullCoordinator};
use isct_core::{
    events::CoreEvent,
    ports::{EventError, EventSink},
    reference_refresh::ReferenceService,
};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

struct DesktopEvents(AppHandle);

impl EventSink for DesktopEvents {
    fn emit(&self, event: CoreEvent) -> Result<(), EventError> {
        self.0
            .emit(event.name(), event.payload())
            .map_err(|error| EventError::Delivery(error.to_string()))
    }
}

// Construct an operation's adapters without storing an AppHandle-backed service
// in Tauri State. The database and coordinator remain the shared instances.
pub(crate) fn references(app: &AppHandle) -> ReferenceService {
    ReferenceService::new(
        app.state::<DataStore>().inner().clone(),
        app.state::<PullCoordinator>().inner().clone(),
        Arc::new(DesktopEvents(app.clone())),
    )
}
