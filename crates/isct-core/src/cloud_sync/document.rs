//! Cloud workspace envelope, history, and optimistic merge ownership.

use super::{CloudFuture, CloudProvider, CloudSyncError, CloudTransport, CloudVersion};
use crate::data::cloud_merge;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Mutex, MutexGuard},
};

const CLOUD_FORMAT: &str = "IsctCompanion";
const CLOUD_FORMAT_VERSION: u64 = 2;
/// Every provider stores the workspace envelope under this one name.
pub const CLOUD_FILE_NAME: &str = "IsctCompanion.json";
pub const MAX_DOCUMENT_BYTES: usize = 25 * 1024 * 1024;
/// Where Apple devices keep the workspace: one record in this container's
/// private database, its envelope in an asset field. The Mac and iOS apps must
/// both use these, so they are named once here.
pub const CLOUDKIT_CONTAINER: &str = "iCloud.app.isctcompanion";
pub const CLOUDKIT_RECORD_TYPE: &str = "IsctCompanionBackup";
pub const CLOUDKIT_ASSET_KEY: &str = "payload";

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DocumentError>> + Send + 'a>>;

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum DocumentError {
    #[error("Cloud workspace is not valid JSON.")]
    InvalidJson,
    #[error("Cloud workspace format is invalid or unsupported.")]
    InvalidFormat,
    #[error("This cloud account contains a different workspace.")]
    DifferentWorkspace,
    #[error("Cloud copy changed; retrying sync.")]
    Conflict,
    #[error("Cloud copy keeps changing; sync will retry automatically.")]
    ConflictExhausted,
    #[error("Cloud workspace revision is exhausted.")]
    RevisionExhausted,
    #[error("Cloud version no longer exists.")]
    VersionMissing,
    #[error("Cloud workspace exceeds the size limit.")]
    TooLarge,
    #[error("{0}")]
    Transport(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudWorkspaceFile {
    pub format: String,
    pub format_version: u64,
    pub revision: u64,
    #[serde(default)]
    pub version: String,
    pub updated_at: String,
    pub updated_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip)]
    pub siblings: Vec<Value>,
    pub document: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous: Vec<CloudWorkspaceFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<CloudWorkspaceFile>,
}

impl CloudWorkspaceFile {
    pub fn merged_document(&self) -> Value {
        self.siblings
            .iter()
            .fold(self.document.clone(), |a, b| cloud_merge::merge(&a, b))
    }

    pub fn version_document(&self, revision: u64) -> Option<&Value> {
        self.previous
            .iter()
            .chain(self.checkpoints.iter())
            .find(|old| old.revision == revision)
            .map(|old| &old.document)
    }
}

pub trait WorkspaceTransport: Send + Sync {
    fn read<'a>(&'a self, provider: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>>;

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> BoxFuture<'a, ()>;

    fn disconnect(&self, provider: CloudProvider) -> Result<(), DocumentError>;
}

/// The queue's `CloudTransport` over any workspace transport, so every shell
/// shares one envelope parse, history listing and merge-and-push loop.
pub struct DocumentCloud<T> {
    transport: T,
    device_name: Option<String>,
}

impl<T: WorkspaceTransport> DocumentCloud<T> {
    pub fn new(transport: T, device_name: Option<String>) -> Self {
        Self {
            transport,
            device_name,
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }
}

fn transport_error(error: DocumentError) -> CloudSyncError {
    CloudSyncError::Transport(error.to_string())
}

impl<T: WorkspaceTransport> CloudTransport for DocumentCloud<T> {
    fn disconnect(&self, provider: CloudProvider) -> Result<(), CloudSyncError> {
        self.transport.disconnect(provider).map_err(transport_error)
    }

    fn read_workspace<'a>(&'a self, provider: CloudProvider) -> CloudFuture<'a, Option<Value>> {
        Box::pin(async move {
            Ok(self
                .transport
                .read(provider)
                .await
                .map_err(transport_error)?
                .map(|file| file.merged_document()))
        })
    }

    /// Re-read before committing: another device may have written since the
    /// multi-provider pull phase. Conditional writes retain that guarantee.
    fn merge_and_push<'a>(
        &'a self,
        provider: CloudProvider,
        document: &'a Value,
        device: &'a str,
    ) -> CloudFuture<'a, Value> {
        Box::pin(async move {
            // Version lists name the writer; an unnamed device is labelled by
            // its OS and the start of its device id.
            let device_name = self.device_name.clone().unwrap_or_else(|| {
                format!(
                    "{} · {}",
                    std::env::consts::OS,
                    device.chars().take(8).collect::<String>()
                )
            });
            merge_and_push(
                &self.transport,
                provider,
                document,
                device,
                Some(&device_name),
            )
            .await
            .map_err(transport_error)
        })
    }

    fn versions<'a>(&'a self, provider: CloudProvider) -> CloudFuture<'a, Vec<CloudVersion>> {
        Box::pin(async move {
            Ok(self
                .transport
                .read(provider)
                .await
                .map_err(transport_error)?
                .map(|file| versions(provider, &file))
                .unwrap_or_default())
        })
    }

    fn version_document<'a>(
        &'a self,
        provider: CloudProvider,
        revision: u64,
    ) -> CloudFuture<'a, Value> {
        Box::pin(async move {
            self.transport
                .read(provider)
                .await
                .map_err(transport_error)?
                .and_then(|file| file.version_document(revision).cloned())
                .ok_or_else(|| transport_error(DocumentError::VersionMissing))
        })
    }
}

/// The stored workspace object and the provider's opaque change tag for it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredObject {
    pub bytes: Vec<u8>,
    pub tag: String,
}

/// A provider that keeps the workspace as one object with a change tag (a
/// CloudKit record's change tag, for example). Implementations move bytes
/// only; parsing, size limits and revision checks stay in `TaggedWorkspace`.
pub trait ObjectStore: Send + Sync {
    fn read<'a>(&'a self, provider: CloudProvider) -> BoxFuture<'a, Option<StoredObject>>;

    /// `expected` is the tag of the object last read, or `None` when none
    /// existed. Return `DocumentError::Conflict` if the stored object differs.
    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        bytes: Vec<u8>,
        expected: Option<String>,
    ) -> BoxFuture<'a, ()>;

    fn disconnect(&self, provider: CloudProvider) -> Result<(), DocumentError>;
}

/// Adapts an `ObjectStore` to envelope revisions: a write is conditional on
/// the change tag that was read together with the expected revision.
pub struct TaggedWorkspace<S> {
    store: S,
    read_tags: Mutex<ReadTags>,
}

/// The envelope revision and change tag of each provider's last read.
type ReadTags = HashMap<CloudProvider, (u64, String)>;

impl<S: ObjectStore> TaggedWorkspace<S> {
    pub fn new(store: S) -> Self {
        Self {
            store,
            read_tags: Mutex::new(HashMap::new()),
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    fn tags(&self) -> Result<MutexGuard<'_, ReadTags>, DocumentError> {
        self.read_tags
            .lock()
            .map_err(|_| DocumentError::Transport("Cloud tag cache is unavailable.".into()))
    }
}

impl<S: ObjectStore> WorkspaceTransport for TaggedWorkspace<S> {
    fn read<'a>(&'a self, provider: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
        Box::pin(async move {
            let Some(object) = self.store.read(provider).await? else {
                self.tags()?.remove(&provider);
                return Ok(None);
            };
            if object.bytes.len() > MAX_DOCUMENT_BYTES {
                return Err(DocumentError::TooLarge);
            }
            let file = parse_cloud_workspace(&object.bytes)?;
            self.tags()?.insert(provider, (file.revision, object.tag));
            Ok(Some(file))
        })
    }

    fn write<'a>(
        &'a self,
        provider: CloudProvider,
        file: &'a CloudWorkspaceFile,
        expected_revision: Option<u64>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let expected = match expected_revision {
                None => None,
                Some(revision) => match self.tags()?.get(&provider) {
                    Some((read, tag)) if *read == revision => Some(tag.clone()),
                    // A newer read replaced the tag; the caller re-reads.
                    _ => return Err(DocumentError::Conflict),
                },
            };
            let bytes =
                serde_json::to_vec(file).map_err(|e| DocumentError::Transport(e.to_string()))?;
            if bytes.len() > MAX_DOCUMENT_BYTES {
                return Err(DocumentError::TooLarge);
            }
            self.store.write(provider, bytes, expected).await
        })
    }

    fn disconnect(&self, provider: CloudProvider) -> Result<(), DocumentError> {
        self.tags()?.remove(&provider);
        self.store.disconnect(provider)
    }
}

pub fn parse_cloud_workspace(bytes: &[u8]) -> Result<CloudWorkspaceFile, DocumentError> {
    let file: CloudWorkspaceFile =
        serde_json::from_slice(bytes).map_err(|_| DocumentError::InvalidJson)?;
    if file.format != CLOUD_FORMAT
        || !(1..=CLOUD_FORMAT_VERSION).contains(&file.format_version)
        || file.revision == 0
        || file.updated_by.trim().is_empty()
        || chrono::DateTime::parse_from_rfc3339(&file.updated_at).is_err()
    {
        return Err(DocumentError::InvalidFormat);
    }
    validate_workspace_document(&file.document)?;
    Ok(file)
}

pub fn versions(provider: CloudProvider, file: &CloudWorkspaceFile) -> Vec<CloudVersion> {
    let mut seen = std::collections::BTreeSet::new();
    file.previous
        .iter()
        .chain(file.checkpoints.iter())
        .filter(|old| seen.insert(old.revision))
        .map(|old| CloudVersion {
            id: format!("{}:{}", provider_key(provider), old.revision),
            provider,
            version: cloud_merge::version(&old.document).unwrap_or_default(),
            saved_at: old.updated_at.clone(),
            device: old
                .device_name
                .clone()
                .unwrap_or_else(|| old.updated_by.clone()),
        })
        .collect()
}

/// Re-read before committing so a conditional-write conflict always merges
/// the newest remote records before the next attempt.
pub async fn merge_and_push<T: WorkspaceTransport + ?Sized>(
    transport: &T,
    provider: CloudProvider,
    document: &Value,
    device: &str,
    device_name: Option<&str>,
) -> Result<Value, DocumentError> {
    for _ in 0..3 {
        let remote = transport.read(provider).await?;
        if let Some(remote) = &remote {
            if let (Some(local_id), Some(remote_id)) = (
                document
                    .pointer("/sync/workspaceId")
                    .and_then(Value::as_str),
                remote
                    .document
                    .pointer("/sync/workspaceId")
                    .and_then(Value::as_str),
            ) {
                if local_id != remote_id {
                    return Err(DocumentError::DifferentWorkspace);
                }
            }
        }
        let merged = remote
            .as_ref()
            .map(|remote| cloud_merge::merge(document, &remote.merged_document()))
            .unwrap_or_else(|| document.clone());
        if remote
            .as_ref()
            .is_some_and(|remote| workspace_contents_equal(&remote.document, &merged))
        {
            return Ok(merged);
        }
        let file = make_workspace_file(merged.clone(), device, device_name, remote.as_ref())?;
        match transport
            .write(
                provider,
                &file,
                remote.as_ref().map(|remote| remote.revision),
            )
            .await
        {
            Ok(()) => return Ok(merged),
            Err(DocumentError::Conflict) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(DocumentError::ConflictExhausted)
}

fn validate_workspace_document(document: &Value) -> Result<(), DocumentError> {
    if document.get("schemaVersion").and_then(Value::as_u64) != Some(6)
        || document.get("kind").and_then(Value::as_str) != Some("IsctCompanion.local-backup")
        || !document.get("tables").is_some_and(Value::is_object)
        || !document.get("localNotes").is_some_and(Value::is_array)
    {
        return Err(DocumentError::InvalidFormat);
    }
    Ok(())
}

fn make_workspace_file(
    document: Value,
    device: &str,
    device_name: Option<&str>,
    remote: Option<&CloudWorkspaceFile>,
) -> Result<CloudWorkspaceFile, DocumentError> {
    let revision = remote
        .map(|remote| {
            remote
                .revision
                .checked_add(1)
                .ok_or(DocumentError::RevisionExhausted)
        })
        .transpose()?
        .unwrap_or(1);
    Ok(CloudWorkspaceFile {
        format: CLOUD_FORMAT.into(),
        format_version: CLOUD_FORMAT_VERSION,
        revision,
        version: cloud_merge::version(&document).unwrap_or_default(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        updated_by: device.into(),
        device_name: device_name.map(str::to_owned),
        siblings: Vec::new(),
        document,
        previous: rotate_versions(remote),
        checkpoints: Vec::new(),
    })
}

// History lives in the same object as current, so an upload cannot rotate
// history independently of the current document.
fn rotate_versions(current: Option<&CloudWorkspaceFile>) -> Vec<CloudWorkspaceFile> {
    let Some(current) = current else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut history = std::iter::once(current)
        .chain(current.previous.iter())
        .chain(current.checkpoints.iter())
        .filter(|old| seen.insert(old.revision))
        .map(flatten_history)
        .collect::<Vec<_>>();
    let now = chrono::Utc::now();
    let new_backup = now.to_rfc3339();
    // The new current document and this retained history are one atomic write.
    let expired = crate::backup::retention::expired(
        std::iter::once(new_backup.as_str())
            .chain(history.iter().map(|old| old.updated_at.as_str())),
        now,
    )
    .into_iter()
    .collect::<std::collections::HashSet<_>>();
    // Keep the previous current document first, even under clock skew: native
    // conditional writers also compare it with the remote document they read.
    history = history
        .into_iter()
        .enumerate()
        .filter(|(index, _)| *index == 0 || !expired.contains(&(index + 1)))
        .map(|(_, old)| old)
        .collect();
    history
}

fn flatten_history(old: &CloudWorkspaceFile) -> CloudWorkspaceFile {
    let mut old = old.clone();
    old.previous.clear();
    old.checkpoints.clear();
    old
}

pub fn workspace_contents_equal(left: &Value, right: &Value) -> bool {
    cloud_merge::contents(left) == cloud_merge::contents(right)
        && left.get("sync") == right.get("sync")
}

fn provider_key(provider: CloudProvider) -> &'static str {
    match provider {
        CloudProvider::GoogleDrive => "google-drive",
        CloudProvider::Onedrive => "onedrive",
        CloudProvider::Icloud => "icloud",
    }
}

#[cfg(test)]
#[path = "../tests/cloud_sync__document.rs"]
mod tests;
