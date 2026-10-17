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
fn history_keeps_recent_edits_and_expires_old_versions_only_with_four_newer_copies() {
    let now = chrono::Utc::now();
    let mut file = make_workspace_file(json!({"old":true}), "Phone", Some("Phone"), None).unwrap();
    file.updated_at = (now - chrono::Duration::days(60)).to_rfc3339();
    for edit in 1..=20 {
        file =
            make_workspace_file(json!({"edit":edit}), "Phone", Some("Phone"), Some(&file)).unwrap();
        if edit < 4 {
            assert!(file.previous.iter().any(|old| old.document["old"] == true));
        }
    }
    assert_eq!(file.previous.len(), 19);
    assert!(!file.previous.iter().any(|old| old.document["old"] == true));
    assert!(file
        .previous
        .iter()
        .all(|old| old.previous.is_empty() && old.checkpoints.is_empty()));
}

#[test]
fn retained_history_stays_flat_and_restore_rotates_current() {
    let mut file = make_workspace_file(json!({"value":1}), "Laptop", Some("Laptop"), None).unwrap();
    for value in 2..=6 {
        file = make_workspace_file(json!({"value":value}), "Phone", Some("Phone"), Some(&file))
            .unwrap();
        assert_eq!(file.previous.len(), (value - 1) as usize);
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
    assert_eq!(restored.previous.len(), 6);
    let mut skewed = restored.clone();
    skewed.updated_at = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    let next = make_workspace_file(json!({"value":7}), "Laptop", None, Some(&skewed)).unwrap();
    assert_eq!(next.previous[0].document, skewed.document); // conditional-write baseline survives clock skew
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
    fn read<'a>(&'a self, _provider: CloudProvider) -> BoxFuture<'a, Option<CloudWorkspaceFile>> {
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
