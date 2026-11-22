use super::{
    backup_compatibility, backup_storage, blocking, snapshot, BackupService, CloudServices,
    CloudSync, CloudSyncError, Direction, SyncState,
};
use crate::data::WorkspaceChangeSource;
use serde_json::Value;

impl CloudSync {
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
            .run(async move {
                service
                    .restore_document_inner(&services, document, None)
                    .await
            })
            .await
            .map_err(CloudSyncError::from)?
    }

    async fn restore_inner(
        &self,
        services: &CloudServices,
        id: &str,
    ) -> Result<SyncState, CloudSyncError> {
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
        let (old, head) = self
            .remote(
                services
                    .transport
                    .restore_documents(cloud_version.provider, revision),
                CloudSyncError::Interrupted,
            )
            .await?;
        self.restore_document_inner(services, old, Some(head)).await
    }

    /// A restore is this device's newest edit (`BackupService::restore`). It
    /// goes up at once, as an explicit upload, and every other device merges
    /// it record by record, keeping what they changed meanwhile elsewhere.
    async fn restore_document_inner(
        &self,
        services: &CloudServices,
        old: Value,
        head: Option<Value>,
    ) -> Result<SyncState, CloudSyncError> {
        let (old, head) = blocking(move || {
            BackupService::validate(&old).map_err(backup_compatibility)?;
            if let Some(head) = &head {
                BackupService::validate(head).map_err(backup_compatibility)?;
            }
            Ok((old, head))
        })
        .await?;
        let _run = self.0.run.lock().await;
        self.finish_apply(services).await?;
        let backup = services.backup.clone();
        blocking(move || {
            let source = WorkspaceChangeSource::CloudRestore;
            match head {
                Some(head) => backup.restore_from_head(&old, &head, source),
                None => backup.restore(&old, source),
            }
            .map_err(backup_storage)
        })
        .await?;
        let restored = snapshot(&services.backup).await?;
        self.0
            .journal
            .change(|journal| journal.request(restored, Direction::Upload))?;
        self.capture_inner(services, false).await?;
        self.publish(services)?;
        drop(_run);
        self.resume_inner(services).await
    }
}
