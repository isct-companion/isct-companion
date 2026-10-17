//! One backup contract for export, recovery and cloud application.
pub(crate) mod retention;
use crate::{
    data::{self, cloud_merge, DataError, DataStore},
    notes::{LocalNoteBackup, NoteError, Notes},
};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error(transparent)]
    Data(#[from] DataError),
    #[error(transparent)]
    Notes(#[from] NoteError),
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
    notes: Notes,
}

impl BackupService {
    pub fn new(store: DataStore, notes: Notes) -> Self {
        Self { store, notes }
    }

    pub fn snapshot(&self) -> Result<Value, BackupError> {
        self.notes.transaction(|notes| {
            Ok(self
                .store
                .export_document(serde_json::to_value(notes.export()?)?)?)
        })
    }

    pub fn export(&self) -> Result<String, BackupError> {
        Ok(serde_json::to_string_pretty(&self.snapshot()?)?)
    }

    pub fn validate(document: &Value) -> Result<(), BackupError> {
        data::validate_sync_document(document)?;
        let notes: Vec<LocalNoteBackup> = serde_json::from_value(document["localNotes"].clone())?;
        crate::notes::validate_notes(&notes)?;
        Ok(())
    }

    /// The sync journal records pending_apply before entering this operation.
    /// Replay is idempotent across the filesystem and SQLite commit boundary.
    pub fn apply(&self, document: &Value) -> Result<(), BackupError> {
        Self::validate(document)?;
        self.notes.transaction(|notes| {
            let local = self
                .store
                .export_document(serde_json::to_value(notes.export()?)?)?;
            let merged = cloud_merge::merge(&local, document);
            let incoming: Vec<LocalNoteBackup> =
                serde_json::from_value(merged["localNotes"].clone())?;
            if cloud_merge::generation(&merged) > cloud_merge::generation(&local) {
                notes.restore(&incoming)?;
            } else {
                notes.merge(&incoming)?;
            }
            // DataStore rechecks current rows under an IMMEDIATE transaction.
            self.store.merge_cloud_data(&merged)?;
            Ok(())
        })
    }

    pub fn device_id(&self) -> Result<String, BackupError> {
        self.store.load_workspace()?["deviceId"]
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
        crate::notes::atomic_write(&destination, document.as_bytes())?;
        Ok(destination)
    }
}

#[cfg(test)]
#[path = "../tests/backup.rs"]
mod tests;
