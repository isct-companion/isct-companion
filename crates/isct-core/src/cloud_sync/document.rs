//! Cloud workspace envelope, history, and optimistic merge ownership.

use super::{CloudProvider, CloudVersion};
use crate::data::cloud_merge;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{future::Future, pin::Pin};

const CLOUD_FORMAT: &str = "IsctCompanion";
const CLOUD_FORMAT_VERSION: u64 = 2;

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
        checkpoints: daily_checkpoints(remote),
    })
}

// History lives in the same object as current, so an upload cannot rotate
// history independently of the current document.
fn rotate_versions(current: Option<&CloudWorkspaceFile>) -> Vec<CloudWorkspaceFile> {
    let Some(current) = current else {
        return Vec::new();
    };
    std::iter::once(current)
        .chain(current.previous.iter())
        .take(2)
        .map(flatten_history)
        .collect()
}

fn daily_checkpoints(current: Option<&CloudWorkspaceFile>) -> Vec<CloudWorkspaceFile> {
    let Some(current) = current else {
        return Vec::new();
    };
    let mut days = std::collections::BTreeSet::new();
    std::iter::once(current)
        .chain(current.checkpoints.iter())
        .filter(|old| days.insert(old.updated_at.get(..10).unwrap_or("").to_owned()))
        .take(7)
        .map(flatten_history)
        .collect()
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
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn document(notes: Value) -> Value {
        json!({
            "schemaVersion": 6,
            "exportedAt": "2026-01-01T00:00:00Z",
            "kind": "IsctCompanion.local-backup",
            "deviceId": "local-backup",
            "sync": {"workspaceId": "workspace-a"},
            "tables": {},
            "localNotes": notes
        })
    }

    #[test]
    fn daily_recovery_survives_frequent_same_day_updates() {
        let mut file = make_workspace_file(json!({"day":0}), "Phone", Some("Phone"), None).unwrap();
        file.updated_at = "2026-10-01T00:00:00Z".into();
        for day in 2..=9 {
            file = make_workspace_file(json!({"day":day}), "Phone", Some("Phone"), Some(&file))
                .unwrap();
            file.updated_at = format!("2026-10-{day:02}T00:00:00Z");
        }
        for edit in 0..20 {
            file = make_workspace_file(json!({"edit":edit}), "Phone", Some("Phone"), Some(&file))
                .unwrap();
            file.updated_at = "2026-10-09T12:00:00Z".into();
        }
        assert_eq!(file.previous.len(), 2);
        assert_eq!(file.checkpoints.len(), 7);
        assert!(file
            .checkpoints
            .iter()
            .any(|point| point.updated_at.starts_with("2026-10-03")));
        assert!(file
            .checkpoints
            .iter()
            .all(|point| point.previous.is_empty() && point.checkpoints.is_empty()));
    }

    #[test]
    fn rotations_keep_two_flat_versions_and_restore_rotates_current() {
        let mut file =
            make_workspace_file(json!({"value":1}), "Laptop", Some("Laptop"), None).unwrap();
        for value in 2..=6 {
            file = make_workspace_file(json!({"value":value}), "Phone", Some("Phone"), Some(&file))
                .unwrap();
            assert!(file.previous.len() <= 2);
            assert!(file.previous.iter().all(|old| old.previous.is_empty()));
        }
        assert_eq!(file.previous[0].document["value"], 5);
        assert_eq!(file.previous[1].document["value"], 4);
        let restored = make_workspace_file(
            file.previous[1].document.clone(),
            "Laptop",
            Some("Laptop"),
            Some(&file),
        )
        .unwrap();
        assert_eq!(restored.document["value"], 4);
        assert_eq!(restored.previous[0].document["value"], 6);
        assert_eq!(restored.previous.len(), 2);
    }

    #[test]
    fn parses_workspace_wrapper_and_compares_only_restorable_content() {
        let file = make_workspace_file(document(json!([])), "device-a", None, None).unwrap();
        let parsed = parse_cloud_workspace(&serde_json::to_vec(&file).unwrap()).unwrap();
        let mut later = parsed.document.clone();
        later["exportedAt"] = Value::String("2026-02-01T00:00:00Z".into());
        assert!(workspace_contents_equal(&parsed.document, &later));
    }

    struct ConflictOnce {
        state: Mutex<ConflictState>,
    }

    struct ConflictState {
        remote: CloudWorkspaceFile,
        replacement: CloudWorkspaceFile,
        writes: usize,
        saved: Option<CloudWorkspaceFile>,
    }

    impl WorkspaceTransport for ConflictOnce {
        fn read<'a>(
            &'a self,
            _provider: CloudProvider,
        ) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
            Box::pin(async move {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| DocumentError::Transport("test lock failed".into()))?;
                Ok(Some(state.remote.clone()))
            })
        }

        fn write<'a>(
            &'a self,
            _provider: CloudProvider,
            file: &'a CloudWorkspaceFile,
            _expected_revision: Option<u64>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| DocumentError::Transport("test lock failed".into()))?;
                state.writes += 1;
                if state.writes == 1 {
                    state.remote = state.replacement.clone();
                    return Err(DocumentError::Conflict);
                }
                state.saved = Some(file.clone());
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn conflict_retry_merges_the_new_remote_before_writing() {
        let note = |id: &str, revision: u64| {
            json!({
                "id": id,
                "rev": revision,
                "updatedAt": "2026-01-01T00:00:00Z",
                "updatedBy": "device"
            })
        };
        let first = make_workspace_file(
            document(json!([note("first-remote", 1)])),
            "remote",
            None,
            None,
        )
        .unwrap();
        let replacement = make_workspace_file(
            document(json!([note("first-remote", 1), note("late-remote", 1)])),
            "remote",
            None,
            Some(&first),
        )
        .unwrap();
        let transport = ConflictOnce {
            state: Mutex::new(ConflictState {
                remote: first,
                replacement,
                writes: 0,
                saved: None,
            }),
        };
        let merged = merge_and_push(
            &transport,
            CloudProvider::GoogleDrive,
            &document(json!([note("local", 1)])),
            "local-device",
            Some("Laptop"),
        )
        .await
        .unwrap();

        let ids: Vec<_> = merged["localNotes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|note| note["id"].as_str())
            .collect();
        assert_eq!(ids, ["first-remote", "late-remote", "local"]);
        let state = transport.state.lock().unwrap();
        assert_eq!(state.writes, 2);
        assert_eq!(state.saved.as_ref().unwrap().document, merged);
    }

    #[tokio::test]
    async fn exhausted_remote_revision_returns_before_writing() {
        let original = document(json!([]));
        let normalized = cloud_merge::merge(&original, &original);
        let mut remote = make_workspace_file(normalized.clone(), "remote", None, None).unwrap();
        remote.revision = u64::MAX;
        let transport = ConflictOnce {
            state: Mutex::new(ConflictState {
                replacement: remote.clone(),
                remote,
                writes: 0,
                saved: None,
            }),
        };

        let unchanged = merge_and_push(
            &transport,
            CloudProvider::GoogleDrive,
            &normalized,
            "local-device",
            Some("Laptop"),
        )
        .await
        .unwrap();
        assert_eq!(unchanged, normalized);
        assert_eq!(transport.state.lock().unwrap().writes, 0);

        let error = merge_and_push(
            &transport,
            CloudProvider::GoogleDrive,
            &document(json!([{
                "id": "local",
                "rev": 1,
                "updatedAt": "2026-01-01T00:00:00Z",
                "updatedBy": "device"
            }])),
            "local-device",
            Some("Laptop"),
        )
        .await
        .unwrap_err();

        assert_eq!(error, DocumentError::RevisionExhausted);
        assert_eq!(transport.state.lock().unwrap().writes, 0);
    }
}
