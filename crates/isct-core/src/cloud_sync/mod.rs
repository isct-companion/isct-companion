//! Durable cloud synchronization state and queue ownership.
pub mod document;
mod journal;

use crate::{
    backup::{BackupError, BackupService},
    data::cloud_merge,
    events::{CoreEvent, WorkspaceChangeSource},
    ports::EventSink,
    runtime_tasks::{RuntimeTaskError, RuntimeTasks},
};
use journal::{version, Job, Journal, JournalStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const REMOTE_TIMEOUT: Duration = Duration::from_secs(45);
const TICK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudProvider {
    GoogleDrive,
    Onedrive,
    Icloud,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderState {
    pub provider: CloudProvider,
    pub enabled: bool,
    pub last_synced_at: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub synced_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVersion {
    pub id: String,
    pub provider: CloudProvider,
    pub saved_at: String,
    pub device: String,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPoint {
    pub id: String,
    pub version: String,
    pub saved_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Pull,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub provider: CloudProvider,
    pub status: DeliveryStatus,
    pub attempts: u32,
    pub retry_at: i64,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryStatus {
    Pending,
    Running,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub direction: Direction,
    pub version: String,
    pub created_at: String,
    pub deliveries: Vec<Delivery>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub syncing: bool,
    #[serde(default)]
    pub automatic: bool,
    #[serde(default)]
    pub local_version: String,
    pub providers: Vec<ProviderState>,
    pub versions: Vec<CloudVersion>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub recovery_points: Vec<RecoveryPoint>,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CloudSyncError {
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Compatibility(String),
    #[error("{0}")]
    Transport(String),
    #[error("{0}")]
    Conflict(String),
    #[error("Sync interrupted or timed out; the saved task will retry.")]
    Interrupted,
    #[error("The cloud sync service has stopped.")]
    Stopped,
    #[error("Cloud sync worker failed; the saved task will retry.")]
    Task(#[source] RuntimeTaskError),
    #[error("Cloud storage worker failed; the saved task will retry.")]
    StorageTask(#[source] RuntimeTaskError),
}

impl From<RuntimeTaskError> for CloudSyncError {
    fn from(error: RuntimeTaskError) -> Self {
        match error {
            RuntimeTaskError::Stopped => Self::Stopped,
            other => Self::Task(other),
        }
    }
}

pub type CloudFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, CloudSyncError>> + Send + 'a>>;

pub trait CloudTransport: Send + Sync {
    fn disconnect(&self, provider: CloudProvider) -> Result<(), CloudSyncError>;
    fn read_workspace<'a>(&'a self, provider: CloudProvider) -> CloudFuture<'a, Option<Value>>;
    fn merge_and_push<'a>(
        &'a self,
        provider: CloudProvider,
        document: &'a Value,
        device: &'a str,
    ) -> CloudFuture<'a, Value>;
    fn versions<'a>(&'a self, provider: CloudProvider) -> CloudFuture<'a, Vec<CloudVersion>>;
    fn version_document<'a>(
        &'a self,
        provider: CloudProvider,
        revision: u64,
    ) -> CloudFuture<'a, Value>;
}

#[derive(Clone)]
pub struct CloudServices {
    pub backup: BackupService,
    pub transport: Arc<dyn CloudTransport>,
    pub events: Arc<dyn EventSink>,
}

struct Inner {
    journal: JournalStore,
    run: tokio::sync::Mutex<()>,
    tasks: RuntimeTasks,
    tick_pass: Arc<tokio::sync::Semaphore>,
    next_tick: Mutex<Instant>,
}

#[derive(Clone)]
pub struct CloudSync(Arc<Inner>);

impl CloudSync {
    pub fn new(
        legacy: PathBuf,
        supported: &[CloudProvider],
        authorized: &[CloudProvider],
    ) -> Result<Self, CloudSyncError> {
        Ok(Self(Arc::new(Inner {
            journal: JournalStore::open(legacy, supported, authorized)?,
            run: tokio::sync::Mutex::new(()),
            tasks: RuntimeTasks::default(),
            tick_pass: Arc::new(tokio::sync::Semaphore::new(1)),
            next_tick: Mutex::new(Instant::now()),
        })))
    }

    pub fn state(&self) -> Result<SyncState, CloudSyncError> {
        self.0.journal.read(Journal::public_state)
    }

    /// Admit one periodic queue/capture pass. The shell may call this often;
    /// Core owns the five-second cadence and serial run guard.
    pub fn tick(&self, services: CloudServices) -> Result<bool, CloudSyncError> {
        if self.0.tasks.is_closed() {
            return Err(CloudSyncError::Stopped);
        }
        let Ok(permit) = self.0.tick_pass.clone().try_acquire_owned() else {
            return Ok(false);
        };
        let now = Instant::now();
        {
            let mut next = self
                .0
                .next_tick
                .lock()
                .map_err(|_| CloudSyncError::Storage("Sync clock unavailable".into()))?;
            if now < *next {
                return Ok(false);
            }
            *next = now + TICK_INTERVAL;
        }
        let service = self.clone();
        if !self.0.tasks.spawn(async move {
            let _permit = permit;
            let _ = service.resume_inner(&services).await;
            if !service.0.tasks.is_closed() {
                let _ = service.capture_inner(&services, true).await;
            }
        }) {
            return Err(CloudSyncError::Stopped);
        }
        Ok(true)
    }

    pub async fn changed(&self, services: CloudServices) -> Result<(), CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move {
                service.capture_inner(&services, true).await?;
                if let Ok(mut next) = service.0.next_tick.lock() {
                    *next = Instant::now();
                }
                Ok(())
            })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub async fn provider_result(
        &self,
        services: CloudServices,
        provider: CloudProvider,
        enabled: bool,
        error: Option<String>,
    ) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            // The journal serializes this change; an in-flight queue pass
            // must not delay reporting a failed connection.
            .run(async move { service.provider_result_inner(&services, provider, enabled, error) })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub async fn set_enabled(
        &self,
        services: CloudServices,
        provider: CloudProvider,
        enabled: bool,
    ) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move {
                service
                    .set_enabled_inner(&services, provider, enabled)
                    .await
            })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub async fn set_automatic(
        &self,
        services: CloudServices,
        automatic: bool,
    ) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move {
                service.0.journal.change(|journal| {
                    journal.state.automatic = automatic;
                    journal.observed.clear();
                    Ok(())
                })?;
                service.capture_inner(&services, true).await?;
                service.state()
            })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub async fn sync(
        &self,
        services: CloudServices,
        direction: Direction,
    ) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move {
                let document = snapshot(&services.backup).await?;
                service.0.journal.change(|journal| {
                    for job in &mut journal.jobs {
                        for delivery in &mut job.task.deliveries {
                            delivery.retry_at = 0;
                        }
                    }
                    journal.enqueue(document, direction)
                })?;
                service.publish(&services)?;
                service.resume_inner(&services).await
            })
            .await
            .map_err(CloudSyncError::from)?
    }

    /// Replay pending work after startup or a native lifecycle wake.
    pub async fn resume(&self, services: CloudServices) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move { service.resume_inner(&services).await })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub async fn prepare_reset(&self, services: CloudServices) -> Result<(), CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move { service.prepare_reset_inner(&services).await })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub async fn restore(
        &self,
        services: CloudServices,
        id: String,
    ) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move { service.restore_inner(&services, &id).await })
            .await
            .map_err(CloudSyncError::from)?
    }

    /// Import an already decoded backup through the same durable restore path
    /// used by cloud history. The owned document survives caller cancellation.
    pub async fn restore_document(
        &self,
        services: CloudServices,
        document: Value,
    ) -> Result<SyncState, CloudSyncError> {
        let service = self.clone();
        self.0
            .tasks
            .run(async move { service.restore_document_inner(&services, document).await })
            .await
            .map_err(CloudSyncError::from)?
    }

    pub fn recovery_document(&self, id: &str) -> Result<String, CloudSyncError> {
        let point = self
            .state()?
            .recovery_points
            .into_iter()
            .find(|point| point.id == id)
            .ok_or_else(|| CloudSyncError::Compatibility("Recovery point not found".into()))?;
        if !point.version.starts_with("v1-")
            || point.version.len() != 67
            || !point.version[3..]
                .bytes()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(CloudSyncError::Compatibility(
                "Invalid recovery version".into(),
            ));
        }
        std::fs::read_to_string(
            self.0
                .journal
                .path
                .with_file_name("recovery")
                .join(format!("{}.json", point.version)),
        )
        .map_err(storage_error)
    }

    pub async fn shutdown(&self) {
        self.0.tasks.close();
        self.0.tasks.wait().await;
        let _ = self.0.journal.change(|journal| {
            journal.state.syncing = false;
            for job in &mut journal.jobs {
                for delivery in &mut job.task.deliveries {
                    if delivery.status == DeliveryStatus::Running {
                        delivery.status = DeliveryStatus::Pending;
                        delivery.retry_at = 0;
                    }
                }
            }
            Ok(())
        });
    }

    // Events are hints; the UI re-reads `cloud_sync_state`, and the sink
    // reports its own delivery failures.
    fn publish(&self, services: &CloudServices) -> Result<(), CloudSyncError> {
        let state = self.state()?;
        let _ = services.events.emit(CoreEvent::CloudSyncState(state));
        Ok(())
    }

    fn publish_workspace(&self, services: &CloudServices, source: WorkspaceChangeSource) {
        let _ = services.events.emit(CoreEvent::WorkspaceChanged {
            source,
            revision: None,
        });
    }

    fn provider_result_inner(
        &self,
        services: &CloudServices,
        provider: CloudProvider,
        enabled: bool,
        error: Option<String>,
    ) -> Result<SyncState, CloudSyncError> {
        self.0.journal.change(|journal| {
            let row = journal
                .state
                .providers
                .iter_mut()
                .find(|row| row.provider == provider)
                .ok_or_else(|| CloudSyncError::Compatibility("Unsupported sync provider".into()))?;
            row.enabled = enabled;
            row.error = error;
            Ok(())
        })?;
        self.publish(services)?;
        self.state()
    }

    async fn set_enabled_inner(
        &self,
        services: &CloudServices,
        provider: CloudProvider,
        enabled: bool,
    ) -> Result<SyncState, CloudSyncError> {
        if !self
            .state()?
            .providers
            .iter()
            .any(|row| row.provider == provider)
        {
            return Err(CloudSyncError::Compatibility(
                "Unsupported sync provider".into(),
            ));
        }
        let _run = self.0.run.lock().await;
        if !enabled {
            services.transport.disconnect(provider)?;
        }
        self.0.journal.change(|journal| {
            if !enabled {
                journal
                    .state
                    .versions
                    .retain(|version| version.provider != provider);
                for job in &mut journal.jobs {
                    job.task
                        .deliveries
                        .retain(|delivery| delivery.provider != provider);
                }
            }
            let row = journal
                .state
                .providers
                .iter_mut()
                .find(|row| row.provider == provider)
                .ok_or_else(|| CloudSyncError::Compatibility("Unsupported sync provider".into()))?;
            row.enabled = enabled;
            row.error = None;
            Ok(())
        })?;
        if enabled && self.state()?.automatic {
            let document = snapshot(&services.backup).await?;
            self.0
                .journal
                .change(|journal| journal.enqueue(document, Direction::Upload))?;
        }
        self.publish(services)?;
        self.state()
    }

    async fn capture_inner(
        &self,
        services: &CloudServices,
        schedule: bool,
    ) -> Result<(), CloudSyncError> {
        let service = self.clone();
        let backup = services.backup.clone();
        let changed = blocking(move || service.capture_blocking(&backup, schedule)).await?;
        if changed {
            self.publish(services)?;
        }
        Ok(())
    }

    fn capture_blocking(
        &self,
        backup: &BackupService,
        schedule: bool,
    ) -> Result<bool, CloudSyncError> {
        if self
            .0
            .journal
            .read(|journal| journal.pending_apply.is_some())?
        {
            return Ok(false);
        }
        let document = backup.snapshot().map_err(backup_storage)?;
        let current = version(&document)?;
        if self.0.journal.read(|journal| journal.observed == current)? {
            return Ok(false);
        }
        self.0.journal.change(|journal| {
            if journal.pending_apply.is_some() {
                Ok(false)
            } else {
                journal.observe(document, schedule)
            }
        })
    }

    async fn resume_inner(&self, services: &CloudServices) -> Result<SyncState, CloudSyncError> {
        if self.0.tasks.is_closed() {
            return Err(CloudSyncError::Stopped);
        }
        let Ok(_run) = self.0.run.try_lock() else {
            return self.state();
        };
        let before = self.state()?;
        let worker = self.clone();
        let worker_services = services.clone();
        let result =
            match tokio::spawn(async move { worker.run_queue(&worker_services).await }).await {
                Ok(result) => result,
                Err(error) => Err(CloudSyncError::Task(error.into())),
            };
        let settled = self.0.journal.change(|journal| {
            journal.state.syncing = false;
            for job in &mut journal.jobs {
                for delivery in &mut job.task.deliveries {
                    if delivery.status == DeliveryStatus::Running {
                        delivery.status = DeliveryStatus::Pending;
                        delivery.retry_at = 0;
                    }
                }
            }
            Ok(())
        });
        // An idle pass writes nothing (no-op journal changes are skipped) and
        // publishes nothing; work in the pass already published its steps.
        let after = self.state()?;
        if settled.is_ok() && after != before {
            let _ = self.publish(services);
        }
        result?;
        settled?;
        Ok(after)
    }

    async fn run_queue(&self, services: &CloudServices) -> Result<(), CloudSyncError> {
        self.finish_apply(services).await?;
        let jobs = self.0.journal.read(|journal| journal.jobs.clone())?;
        for mut job in jobs {
            if self.0.tasks.is_closed() {
                return Err(CloudSyncError::Stopped);
            }
            let now = chrono::Utc::now().timestamp();
            if !job.task.deliveries.iter().any(|delivery| {
                delivery.status != DeliveryStatus::Completed && delivery.retry_at <= now
            }) {
                continue;
            }
            self.0.journal.change(|journal| {
                journal.state.syncing = true;
                Ok(())
            })?;
            self.publish(services)?;
            if job.task.direction == Direction::Upload {
                let providers = self
                    .state()?
                    .providers
                    .into_iter()
                    .filter(|provider| provider.enabled)
                    .collect::<Vec<_>>();
                for provider in providers {
                    if let Ok(Some(remote)) = self
                        .remote(
                            services.transport.read_workspace(provider.provider),
                            CloudSyncError::Interrupted,
                        )
                        .await
                    {
                        if self.bind_workspace(&remote).is_ok() {
                            job.document = cloud_merge::merge(&job.document, &remote);
                        }
                    }
                }
                job.task.version = version(&job.document)?;
                self.0.journal.change(|journal| {
                    let saved = journal
                        .jobs
                        .iter_mut()
                        .find(|saved| saved.task.id == job.task.id)
                        .ok_or_else(|| CloudSyncError::Storage("Sync task missing".into()))?;
                    saved.document = job.document.clone();
                    saved.task.version = job.task.version.clone();
                    Ok(())
                })?;
            }
            let mut convergence = job.document.clone();
            let mut delivered = false;
            for delivery in job.task.deliveries.clone() {
                if self.0.tasks.is_closed() {
                    return Err(CloudSyncError::Stopped);
                }
                if delivery.status == DeliveryStatus::Completed
                    || delivery.retry_at > chrono::Utc::now().timestamp()
                    || !self
                        .state()?
                        .providers
                        .iter()
                        .any(|provider| provider.provider == delivery.provider && provider.enabled)
                {
                    continue;
                }
                self.0.journal.change(|journal| {
                    journal.state.syncing = true;
                    let delivery = delivery_mut(journal, &job.task.id, delivery.provider)?;
                    delivery.status = DeliveryStatus::Running;
                    delivery.attempts = delivery.attempts.saturating_add(1);
                    delivery.error = None;
                    Ok(())
                })?;
                self.publish(services)?;
                let outcome = self.deliver(services, &job, delivery.provider).await;
                if let Ok((document, _)) = &outcome {
                    delivered = true;
                    convergence = cloud_merge::merge(&convergence, document);
                }
                let stopped = matches!(&outcome, Err(CloudSyncError::Stopped));
                self.record_delivery(&job.task.id, delivery.provider, outcome)?;
                self.publish(services)?;
                if stopped {
                    return Err(CloudSyncError::Stopped);
                }
            }
            if delivered && job.task.direction == Direction::Upload {
                self.0
                    .journal
                    .change(|journal| journal.enqueue(convergence, Direction::Upload))?;
                if self.state()?.automatic {
                    let latest = snapshot(&services.backup).await?;
                    self.0
                        .journal
                        .change(|journal| journal.enqueue(latest, Direction::Upload))?;
                }
            }
        }
        self.0.journal.change(|journal| {
            let mut completed = 0;
            journal.jobs.reverse();
            journal.jobs.retain(|job| {
                if job
                    .task
                    .deliveries
                    .iter()
                    .all(|delivery| delivery.status == DeliveryStatus::Completed)
                {
                    completed += 1;
                    completed <= 10
                } else {
                    true
                }
            });
            journal.jobs.reverse();
            journal
                .state
                .versions
                .sort_by(|left, right| right.saved_at.cmp(&left.saved_at));
            Ok(())
        })
    }

    fn record_delivery(
        &self,
        task: &str,
        provider: CloudProvider,
        result: Result<(Value, Vec<CloudVersion>), CloudSyncError>,
    ) -> Result<(), CloudSyncError> {
        self.0.journal.change(|journal| {
            journal.state.syncing = false;
            let delivery = delivery_mut(journal, task, provider)?;
            match &result {
                Ok(_) => {
                    delivery.status = DeliveryStatus::Completed;
                    delivery.error = None;
                }
                Err(CloudSyncError::Stopped) => {
                    delivery.status = DeliveryStatus::Pending;
                    delivery.error = None;
                    delivery.retry_at = 0;
                }
                Err(error) => {
                    delivery.status = DeliveryStatus::Pending;
                    delivery.error = Some(error.to_string());
                    let exponent = delivery.attempts.clamp(1, 4) - 1;
                    delivery.retry_at =
                        chrono::Utc::now().timestamp() + (30_i64 * 2_i64.pow(exponent)).min(300);
                }
            }
            let provider_state = journal
                .state
                .providers
                .iter_mut()
                .find(|state| state.provider == provider)
                .ok_or_else(|| CloudSyncError::Storage("Unsupported provider".into()))?;
            match result {
                Ok((document, versions)) => {
                    provider_state.error = None;
                    provider_state.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
                    provider_state.synced_version = Some(version(&document)?);
                    journal
                        .state
                        .versions
                        .retain(|version| version.provider != provider);
                    journal.state.versions.extend(versions);
                }
                Err(CloudSyncError::Stopped) => {}
                Err(error) => provider_state.error = Some(error.to_string()),
            }
            Ok(())
        })
    }

    async fn deliver(
        &self,
        services: &CloudServices,
        job: &Job,
        provider: CloudProvider,
    ) -> Result<(Value, Vec<CloudVersion>), CloudSyncError> {
        let remote = self
            .remote(
                services.transport.read_workspace(provider),
                CloudSyncError::Interrupted,
            )
            .await?;
        let remote_id = remote
            .as_ref()
            .and_then(|document| document.pointer("/sync/workspaceId"))
            .and_then(Value::as_str);
        let workspace_id = self.0.journal.change(|journal| {
            if let (Some(local), Some(remote)) = (&journal.workspace_id, remote_id) {
                if local != remote {
                    return Err(workspace_conflict());
                }
            }
            Ok(journal
                .workspace_id
                .get_or_insert_with(|| {
                    remote_id
                        .map(str::to_owned)
                        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string())
                })
                .clone())
        })?;
        let mut requested = job.document.clone();
        requested["sync"]["workspaceId"] = json!(workspace_id);
        let document = if job.task.direction == Direction::Upload {
            let backup = services.backup.clone();
            let device = blocking(move || backup.device_id().map_err(backup_storage)).await?;
            self.remote(
                services
                    .transport
                    .merge_and_push(provider, &requested, &device),
                CloudSyncError::Interrupted,
            )
            .await?
        } else {
            remote.ok_or_else(|| CloudSyncError::Transport("No cloud copy exists yet.".into()))?
        };
        // Re-read after every remote await. Applying a stale snapshot must not
        // overwrite local edits made while the provider was in flight.
        let latest = snapshot(&services.backup).await?;
        let merged = cloud_merge::merge(&latest, &document);
        if cloud_merge::contents(&latest) != cloud_merge::contents(&merged) {
            self.checkpoint(&latest).await?;
        }
        self.apply_document(services, &merged).await?;
        self.capture_inner(services, false).await?;
        self.publish_workspace(services, WorkspaceChangeSource::CloudSync);
        let versions = self
            .remote(
                services.transport.versions(provider),
                CloudSyncError::Interrupted,
            )
            .await?;
        Ok((document, versions))
    }

    async fn remote<T>(
        &self,
        work: CloudFuture<'_, T>,
        timeout_error: CloudSyncError,
    ) -> Result<T, CloudSyncError> {
        tokio::select! {
            _ = self.0.tasks.closed() => Err(CloudSyncError::Stopped),
            result = tokio::time::timeout(REMOTE_TIMEOUT, work) => {
                result.unwrap_or(Err(timeout_error))
            }
        }
    }

    fn bind_workspace(&self, document: &Value) -> Result<(), CloudSyncError> {
        let remote_id = document
            .pointer("/sync/workspaceId")
            .and_then(Value::as_str);
        self.0.journal.change(|journal| {
            if let (Some(local), Some(remote)) = (&journal.workspace_id, remote_id) {
                if local != remote {
                    return Err(workspace_conflict());
                }
            }
            if journal.workspace_id.is_none() {
                journal.workspace_id = remote_id.map(str::to_owned);
            }
            Ok(())
        })
    }

    async fn apply_document(
        &self,
        services: &CloudServices,
        document: &Value,
    ) -> Result<(), CloudSyncError> {
        let validated = document.clone();
        blocking(move || BackupService::validate(&validated).map_err(backup_compatibility)).await?;
        self.0.journal.change(|journal| {
            journal.pending_apply = Some(document.clone());
            Ok(())
        })?;
        self.finish_apply(services).await
    }

    async fn finish_apply(&self, services: &CloudServices) -> Result<(), CloudSyncError> {
        let pending = self
            .0
            .journal
            .read(|journal| journal.pending_apply.clone())?;
        if let Some(document) = pending {
            let backup = services.backup.clone();
            blocking(move || backup.apply(&document).map_err(backup_storage)).await?;
            self.0.journal.change(|journal| {
                journal.pending_apply = None;
                Ok(())
            })?;
        }
        Ok(())
    }

    async fn checkpoint(&self, document: &Value) -> Result<(), CloudSyncError> {
        let service = self.clone();
        let document = document.clone();
        blocking(move || service.checkpoint_blocking(&document)).await
    }

    fn checkpoint_blocking(&self, document: &Value) -> Result<(), CloudSyncError> {
        let directory = self.0.journal.path.with_file_name("recovery");
        std::fs::create_dir_all(&directory).map_err(storage_error)?;
        let document_version = version(document)?;
        let path = directory.join(format!("{document_version}.json"));
        if !path.exists() {
            crate::notes::atomic_write(&path, document.to_string().as_bytes())
                .map_err(storage_error)?;
        }
        self.0.journal.change(|journal| {
            if !journal
                .state
                .recovery_points
                .iter()
                .any(|point| point.version == document_version)
            {
                journal.state.recovery_points.insert(
                    0,
                    RecoveryPoint {
                        id: format!("local:{document_version}"),
                        version: document_version,
                        saved_at: chrono::Utc::now().to_rfc3339(),
                    },
                );
            }
            Ok(())
        })
    }

    async fn prepare_reset_inner(&self, services: &CloudServices) -> Result<(), CloudSyncError> {
        let _run = self.0.run.lock().await;
        self.finish_apply(services).await?;
        let current = snapshot(&services.backup).await?;
        self.checkpoint(&current).await?;
        self.0.journal.change(|journal| {
            journal.state.automatic = false;
            journal.state.syncing = false;
            journal.jobs.clear();
            journal.state.versions.clear();
            journal.observed.clear();
            journal.workspace_id = None;
            for provider in &mut journal.state.providers {
                provider.enabled = false;
                provider.synced_version = None;
                provider.error = None;
                provider.last_synced_at = None;
            }
            Ok(())
        })?;
        self.publish(services)
    }

    async fn restore_inner(
        &self,
        services: &CloudServices,
        id: &str,
    ) -> Result<SyncState, CloudSyncError> {
        let old = if id.starts_with("local:") {
            let service = self.clone();
            let id = id.to_owned();
            blocking(move || {
                serde_json::from_str(&service.recovery_document(&id)?).map_err(|error| {
                    CloudSyncError::Compatibility(format!("Invalid recovery file: {error}"))
                })
            })
            .await?
        } else {
            let cloud_version = self
                .state()?
                .versions
                .into_iter()
                .find(|version| version.id == id)
                .ok_or_else(|| {
                    CloudSyncError::Compatibility("Cloud version no longer exists".into())
                })?;
            let revision = id
                .rsplit(':')
                .next()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| CloudSyncError::Compatibility("Invalid cloud version".into()))?;
            self.remote(
                services
                    .transport
                    .version_document(cloud_version.provider, revision),
                CloudSyncError::Interrupted,
            )
            .await?
        };
        self.restore_document_inner(services, old).await
    }

    async fn restore_document_inner(
        &self,
        services: &CloudServices,
        old: Value,
    ) -> Result<SyncState, CloudSyncError> {
        let validated = old.clone();
        blocking(move || BackupService::validate(&validated).map_err(backup_compatibility)).await?;
        let _run = self.0.run.lock().await;
        self.finish_apply(services).await?;
        if self.state()?.tasks.iter().any(|task| {
            task.deliveries
                .iter()
                .any(|delivery| delivery.status != DeliveryStatus::Completed)
        }) {
            return Err(CloudSyncError::Conflict("Finish pending sync tasks or disconnect the unavailable provider before restoring.".into()));
        }
        let current = snapshot(&services.backup).await?;
        self.checkpoint(&current).await?;
        let mut restored = cloud_merge::merge(&old, &old);
        restored["sync"] = current["sync"].clone();
        let mut generation = cloud_merge::generation(&current).0;
        for provider in self
            .state()?
            .providers
            .into_iter()
            .filter(|provider| provider.enabled)
        {
            if let Some(remote) = self
                .remote(
                    services.transport.read_workspace(provider.provider),
                    CloudSyncError::Interrupted,
                )
                .await?
            {
                self.bind_workspace(&remote)?;
                generation = generation.max(cloud_merge::generation(&remote).0);
            }
        }
        let next_generation = generation.checked_add(1).ok_or_else(|| {
            CloudSyncError::Compatibility("Cloud generation is too large.".into())
        })?;
        restored["sync"]["generation"] =
            json!({"counter":next_generation,"id":uuid::Uuid::now_v7().to_string()});
        self.0
            .journal
            .change(|journal| journal.enqueue(restored.clone(), Direction::Upload))?;
        self.apply_document(services, &restored).await?;
        self.capture_inner(services, false).await?;
        self.publish_workspace(services, WorkspaceChangeSource::CloudRestore);
        self.publish(services)?;
        drop(_run);
        self.resume_inner(services).await
    }
}

fn delivery_mut<'a>(
    journal: &'a mut Journal,
    task: &str,
    provider: CloudProvider,
) -> Result<&'a mut Delivery, CloudSyncError> {
    journal
        .jobs
        .iter_mut()
        .find(|job| job.task.id == task)
        .and_then(|job| {
            job.task
                .deliveries
                .iter_mut()
                .find(|delivery| delivery.provider == provider)
        })
        .ok_or_else(|| CloudSyncError::Storage("Sync task missing".into()))
}

fn workspace_conflict() -> CloudSyncError {
    CloudSyncError::Conflict(
        "This cloud account contains a different workspace. Disconnect it before continuing."
            .into(),
    )
}

fn storage_error(error: impl ToString) -> CloudSyncError {
    CloudSyncError::Storage(error.to_string())
}

fn backup_storage(error: BackupError) -> CloudSyncError {
    CloudSyncError::Storage(error.to_string())
}

fn backup_compatibility(error: BackupError) -> CloudSyncError {
    CloudSyncError::Compatibility(error.to_string())
}

async fn snapshot(backup: &BackupService) -> Result<Value, CloudSyncError> {
    let backup = backup.clone();
    blocking(move || backup.snapshot().map_err(backup_storage)).await
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CloudSyncError> + Send + 'static,
) -> Result<T, CloudSyncError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| CloudSyncError::StorageTask(error.into()))?
}

#[cfg(test)]
#[path = "../tests/cloud_sync.rs"]
mod tests;
