//! One backup contract for export, restore and cloud application.
pub(crate) mod retention;
use crate::data::{self, DataError, DataStore, WorkspaceChangeSource};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error(transparent)]
    Data(#[from] DataError),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Storage(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct BackupService {
    store: DataStore,
}

impl BackupService {
    pub fn new(store: DataStore) -> Self {
        Self { store }
    }

    pub fn snapshot(&self) -> Result<Value, BackupError> {
        Ok(self.store.export_document()?)
    }

    pub fn export(&self) -> Result<String, BackupError> {
        Ok(serde_json::to_string_pretty(&self.snapshot()?)?)
    }

    pub fn validate(document: &Value) -> Result<(), BackupError> {
        Ok(data::validate_sync_document(document)?)
    }

    /// Merges another copy record by record. The sync journal records
    /// pending_apply before entering this operation, so it can be replayed.
    pub fn apply(
        &self,
        document: &Value,
        source: WorkspaceChangeSource,
    ) -> Result<(), BackupError> {
        Self::validate(document)?;
        Ok(self.store.merge_cloud_data(document, source)?)
    }

    /// Makes `document` this device's newest edits (`DataStore::restore_records`).
    pub fn restore(
        &self,
        document: &Value,
        source: WorkspaceChangeSource,
    ) -> Result<(), BackupError> {
        Self::validate(document)?;
        Ok(self.store.restore_records(document, source)?)
    }

    pub fn device_id(&self) -> Result<String, BackupError> {
        self.store.load_fields(&[])?["deviceId"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| BackupError::Invalid("Device id missing".into()))
    }

    pub fn write_scheduled(&self, directory: &Path) -> Result<PathBuf, BackupError> {
        let document = self.export()?;
        if document.len() > 32 * 1024 * 1024 {
            return Err(BackupError::Invalid(
                "scheduled export exceeds the 32 MiB safety limit".into(),
            ));
        }
        std::fs::create_dir_all(directory)?;
        let destination = directory.join("workspace-latest.json");
        crate::storage::atomic_write(&destination, document.as_bytes())
            .map_err(|error| BackupError::Invalid(error.to_string()))?;
        Ok(destination)
    }
}

#[cfg(test)]
#[path = "../tests/backup.rs"]
mod tests;
