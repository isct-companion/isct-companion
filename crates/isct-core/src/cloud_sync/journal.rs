use super::{
    CloudProvider, CloudSyncError, CloudVersion, Delivery, DeliveryStatus, Direction,
    ProviderState, SyncState, Task,
};
use crate::data::cloud_merge;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, path::PathBuf, sync::Mutex};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Job {
    pub(super) task: Task,
    pub(super) document: Value,
    #[serde(default)]
    pub(super) manual: bool,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// Rows and history of providers this binary cannot run. An unsigned macOS
    /// debug build has no iCloud but shares the journal with the signed app.
    #[serde(default)]
    pub(super) parked_providers: Vec<ProviderState>,
    #[serde(default)]
    pub(super) parked_versions: Vec<CloudVersion>,
    /// Last confirmed upload per provider. The hourly automatic-upload limit
    /// counts from here, so a pull does not postpone the next upload.
    #[serde(default)]
    pub(super) uploaded_at: HashMap<CloudProvider, String>,
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
                manual: false,
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

    pub(super) fn request(
        &mut self,
        document: Value,
        direction: Direction,
    ) -> Result<(), CloudSyncError> {
        let requested = version(&document)?;
        self.enqueue(document, direction)?;
        for job in &mut self.jobs {
            if job.task.direction == direction && job.task.version == requested {
                job.manual = true;
            }
        }
        Ok(())
    }

    /// Pending snapshots describe state, not individual commands. Merge their
    /// records/tombstones and keep each provider's retry budget when replacing them.
    /// Only the serial worker calls this, before taking its execution snapshot.
    pub(super) fn coalesce_uploads(&mut self) -> Result<(), CloudSyncError> {
        let pending = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, job)| {
                job.task.direction == Direction::Upload
                    && !job
                        .task
                        .deliveries
                        .iter()
                        .any(|d| d.status == DeliveryStatus::Running)
                    && job
                        .task
                        .deliveries
                        .iter()
                        .any(|d| d.status == DeliveryStatus::Pending)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if pending.len() < 2 {
            return Ok(());
        }
        let mut combined = self.jobs[*pending
            .first()
            .ok_or_else(|| CloudSyncError::Storage("Sync task missing".into()))?]
        .clone();
        combined.task.deliveries.clear();
        for index in &pending {
            let job = &self.jobs[*index];
            combined.document = cloud_merge::merge(&combined.document, &job.document);
            combined.manual |= job.manual;
            for delivery in &job.task.deliveries {
                if delivery.status != DeliveryStatus::Pending {
                    continue;
                }
                if let Some(saved) = combined
                    .task
                    .deliveries
                    .iter_mut()
                    .find(|d| d.provider == delivery.provider)
                {
                    saved.attempts = saved.attempts.max(delivery.attempts);
                    if delivery.retry_at > saved.retry_at {
                        saved.retry_at = delivery.retry_at;
                        saved.error = delivery.error.clone();
                    }
                } else {
                    combined.task.deliveries.push(delivery.clone());
                }
            }
        }
        combined.task.version = version(&combined.document)?;
        // Keep completed receipts tied to their original snapshot and identifier.
        if self.jobs.iter().any(|job| {
            job.task.id == combined.task.id
                && job
                    .task
                    .deliveries
                    .iter()
                    .any(|d| d.status == DeliveryStatus::Completed)
        }) {
            combined.task.id = uuid::Uuid::now_v7().to_string();
        }
        for index in pending {
            self.jobs[index]
                .task
                .deliveries
                .retain(|d| d.status != DeliveryStatus::Pending);
        }
        self.jobs.retain(|job| !job.task.deliveries.is_empty());
        self.jobs.push(combined);
        Ok(())
    }

    pub(super) fn upload_due(
        &self,
        provider: CloudProvider,
        now: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        // Journals from before `uploaded_at` only know the last sync of any kind.
        self.uploaded_at
            .get(&provider)
            .map(String::as_str)
            .or_else(|| {
                self.state
                    .providers
                    .iter()
                    .find(|row| row.provider == provider)
                    .and_then(|row| row.last_synced_at.as_deref())
            })
            .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
            .is_none_or(|last| {
                now.signed_duration_since(last).num_seconds() >= super::SNAPSHOT_INTERVAL_SECONDS
            })
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

    /// Moves rows and history between the live state and the parked lists, so
    /// a build without a provider neither shows it nor forgets it: the build
    /// that has it again finds its switch, acknowledgement and history unchanged.
    fn park(&mut self, supported: &[CloudProvider]) {
        for row in std::mem::take(&mut self.parked_providers) {
            if !supported.contains(&row.provider) {
                self.parked_providers.push(row);
            } else if !self
                .state
                .providers
                .iter()
                .any(|r| r.provider == row.provider)
            {
                self.state.providers.push(row);
            }
        }
        for version in std::mem::take(&mut self.parked_versions) {
            if !supported.contains(&version.provider) {
                self.parked_versions.push(version);
            } else if !self.state.versions.contains(&version) {
                self.state.versions.push(version);
            }
        }
        let (rows, parked): (Vec<_>, Vec<_>) = std::mem::take(&mut self.state.providers)
            .into_iter()
            .partition(|row| supported.contains(&row.provider));
        self.state.providers = rows;
        self.parked_providers
            .retain(|row| !parked.iter().any(|p| p.provider == row.provider));
        self.parked_providers.extend(parked);
        let (versions, parked): (Vec<_>, Vec<_>) = std::mem::take(&mut self.state.versions)
            .into_iter()
            .partition(|version| supported.contains(&version.provider));
        self.state.versions = versions;
        self.parked_versions.extend(parked);
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
        journal.park(supported);
        // A provider absent from this binary cannot execute. Drop only its
        // transport deliveries, preserving documents/recovery files and other receipts.
        for job in &mut journal.jobs {
            job.task
                .deliveries
                .retain(|delivery| supported.contains(&delivery.provider));
        }
        journal.jobs.retain(|job| !job.task.deliveries.is_empty());
        for provider in supported {
            if !journal
                .state
                .providers
                .iter()
                .any(|row| row.provider == *provider)
            {
                journal.state.providers.push(ProviderState {
                    provider: *provider,
                    enabled: authorized.contains(provider) || *provider == CloudProvider::Icloud,
                    last_synced_at: None,
                    error: None,
                    synced_version: None,
                });
            }
        }
        journal
            .state
            .providers
            .sort_by_key(|provider| provider.provider != CloudProvider::GoogleDrive);
        persist(&path, &journal)?;
        Ok(Self {
            path,
            value: Mutex::new(journal),
        })
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
        // Idle passes must not fsync the whole journal: skip unchanged writes.
        if next != *guard {
            persist(&self.path, &next)?;
            *guard = next;
        }
        Ok(result)
    }

    pub(super) fn read<T>(&self, read: impl FnOnce(&Journal) -> T) -> Result<T, CloudSyncError> {
        self.value
            .lock()
            .map(|journal| read(&journal))
            .map_err(|_| CloudSyncError::Storage("Sync state unavailable".into()))
    }
}

fn persist(path: &std::path::Path, journal: &Journal) -> Result<(), CloudSyncError> {
    let serialized = serde_json::to_string(journal).map_err(storage)?;
    let connection = rusqlite::Connection::open(path).map_err(storage)?;
    connection
        .execute_batch("PRAGMA synchronous=FULL")
        .map_err(storage)?;
    connection
        .execute(
            "INSERT INTO journal(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
            [serialized],
        )
        .map_err(storage)?;
    Ok(())
}

pub(super) fn version(document: &Value) -> Result<String, CloudSyncError> {
    cloud_merge::version(document).map_err(|error| CloudSyncError::Compatibility(error.to_string()))
}

fn storage(error: impl ToString) -> CloudSyncError {
    CloudSyncError::Storage(error.to_string())
}
