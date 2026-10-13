use super::{
    CloudProvider, CloudSyncError, Delivery, DeliveryStatus, Direction, ProviderState, SyncState,
    Task,
};
use crate::data::cloud_merge;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, sync::Mutex};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Job {
    pub(super) task: Task,
    pub(super) document: Value,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Journal {
    #[serde(flatten)]
    pub(super) state: SyncState,
    #[serde(default)]
    pub(super) jobs: Vec<Job>,
    #[serde(default)]
    pub(super) observed: String,
    #[serde(default)]
    pub(super) workspace_id: Option<String>,
    #[serde(default)]
    pub(super) pending_apply: Option<Value>,
}

impl Journal {
    pub(super) fn observe(
        &mut self,
        document: Value,
        schedule: bool,
    ) -> Result<bool, CloudSyncError> {
        let version = version(&document)?;
        if self.observed == version {
            return Ok(false);
        }
        self.observed = version.clone();
        self.state.local_version = version;
        if schedule && self.state.automatic {
            self.enqueue(document, Direction::Upload)?;
        }
        Ok(true)
    }

    pub(super) fn enqueue(
        &mut self,
        document: Value,
        direction: Direction,
    ) -> Result<(), CloudSyncError> {
        let version = version(&document)?;
        let deliveries = self
            .state
            .providers
            .iter()
            .filter(|provider| provider.enabled)
            .filter(|provider| {
                direction == Direction::Pull || provider.synced_version.as_ref() != Some(&version)
            })
            .filter(|provider| {
                !self.jobs.iter().any(|job| {
                    job.task.direction == direction
                        && job.task.version == version
                        && job.task.deliveries.iter().any(|delivery| {
                            delivery.provider == provider.provider
                                && delivery.status != DeliveryStatus::Completed
                        })
                })
            })
            .map(|provider| Delivery {
                provider: provider.provider,
                status: DeliveryStatus::Pending,
                attempts: 0,
                retry_at: 0,
                error: None,
            })
            .collect::<Vec<_>>();
        if !deliveries.is_empty() {
            self.jobs.push(Job {
                document,
                task: Task {
                    id: uuid::Uuid::now_v7().to_string(),
                    direction,
                    version,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    deliveries,
                },
            });
        }
        Ok(())
    }

    pub(super) fn recover(&mut self) {
        self.state.syncing = false;
        for job in &mut self.jobs {
            for delivery in &mut job.task.deliveries {
                if delivery.status == DeliveryStatus::Running {
                    delivery.status = DeliveryStatus::Pending;
                    delivery.retry_at = 0;
                }
            }
        }
    }

    pub(super) fn public_state(&self) -> SyncState {
        let mut state = self.state.clone();
        state.tasks = self.jobs.iter().map(|job| job.task.clone()).collect();
        state
    }
}

pub(super) struct JournalStore {
    pub(super) path: PathBuf,
    value: Mutex<Journal>,
}

impl JournalStore {
    pub(super) fn open(
        legacy: PathBuf,
        supported: &[CloudProvider],
        authorized: &[CloudProvider],
    ) -> Result<Self, CloudSyncError> {
        let path = legacy.with_extension("sqlite");
        let connection = rusqlite::Connection::open(&path).map_err(storage)?;
        connection
            .execute_batch(
                "PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS journal (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL)",
            )
            .map_err(storage)?;
        let saved: Option<String> = connection
            .query_row("SELECT value FROM journal WHERE id=1", [], |row| row.get(0))
            .optional()
            .map_err(storage)?;
        let mut journal = if let Some(saved) = saved {
            serde_json::from_str(&saved).map_err(|error| {
                CloudSyncError::Compatibility(format!("Invalid sync journal: {error}"))
            })?
        } else if legacy.exists() {
            serde_json::from_slice(&std::fs::read(&legacy).map_err(storage)?).map_err(|error| {
                CloudSyncError::Compatibility(format!("Invalid sync configuration: {error}"))
            })?
        } else {
            Journal {
                state: SyncState {
                    providers: supported
                        .iter()
                        .copied()
                        .map(|provider| ProviderState {
                            provider,
                            enabled: authorized.contains(&provider)
                                || provider == CloudProvider::Icloud,
                            last_synced_at: None,
                            error: None,
                            synced_version: None,
                        })
                        .collect(),
                    ..SyncState::default()
                },
                ..Journal::default()
            }
        };
        journal.recover();
        journal
            .state
            .providers
            .retain(|provider| supported.contains(&provider.provider));
        journal
            .state
            .providers
            .sort_by_key(|provider| provider.provider != CloudProvider::GoogleDrive);
        let store = Self {
            path,
            value: Mutex::new(journal),
        };
        store.change(|_| Ok(()))?;
        Ok(store)
    }

    pub(super) fn change<T>(
        &self,
        update: impl FnOnce(&mut Journal) -> Result<T, CloudSyncError>,
    ) -> Result<T, CloudSyncError> {
        let mut guard = self
            .value
            .lock()
            .map_err(|_| CloudSyncError::Storage("Sync journal unavailable".into()))?;
        let mut next = guard.clone();
        let result = update(&mut next)?;
        let serialized = serde_json::to_string(&next).map_err(storage)?;
        let connection = rusqlite::Connection::open(&self.path).map_err(storage)?;
        connection
            .execute_batch("PRAGMA synchronous=FULL")
            .map_err(storage)?;
        connection
            .execute(
                "INSERT INTO journal(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
                [serialized],
            )
            .map_err(storage)?;
        *guard = next;
        Ok(result)
    }

    pub(super) fn read<T>(&self, read: impl FnOnce(&Journal) -> T) -> Result<T, CloudSyncError> {
        self.value
            .lock()
            .map(|journal| read(&journal))
            .map_err(|_| CloudSyncError::Storage("Sync state unavailable".into()))
    }
}

pub(super) fn version(document: &Value) -> Result<String, CloudSyncError> {
    cloud_merge::version(document).map_err(|error| CloudSyncError::Compatibility(error.to_string()))
}

fn storage(error: impl ToString) -> CloudSyncError {
    CloudSyncError::Storage(error.to_string())
}
