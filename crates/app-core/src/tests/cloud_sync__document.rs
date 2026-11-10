use super::*;
use chrono::DurationRound;
use serde_json::json;
use std::sync::Mutex;

/// A complete workspace document whose personal events are `events`.
fn document(events: Value) -> Value {
    let partial =
        json!({"exportedAt": "2026-01-01T00:00:00Z", "tables": {"personal_events": events}});
    cloud_merge::merge(&partial, &partial)
}

fn event_ids(document: &Value) -> Vec<&str> {
    document["tables"]["personal_events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["id"].as_str())
        .collect()
}

fn days_ago(file: &mut CloudWorkspaceFile, days: i64) {
    file.updated_at = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
}

#[test]
fn rapid_edits_thin_out_and_versions_older_than_30_days_go_once_four_are_newer() {
    let mut file = make_workspace_file(json!({"old":true}), "Phone", Some("Phone"), None).unwrap();
    days_ago(&mut file, 60);
    // Twenty edits within one hour two days ago: one copy stands for that
    // day, besides the four newest.
    let base = (chrono::Utc::now() - chrono::Duration::days(2))
        .duration_trunc(chrono::Duration::hours(1))
        .unwrap();
    for edit in 1..=20 {
        file =
            make_workspace_file(json!({"edit":edit}), "Phone", Some("Phone"), Some(&file)).unwrap();
        if edit < 4 {
            assert!(file.previous.iter().any(|old| old.document["old"] == true));
        }
        file.updated_at = (base + chrono::Duration::seconds(edit)).to_rfc3339();
    }
    let kept = file
        .previous
        .iter()
        .map(|old| old.document["edit"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(kept, [19, 18, 17]);
    assert!(file
        .previous
        .iter()
        .all(|old| old.previous.is_empty() && old.checkpoints.is_empty()));
}

#[test]
fn retained_history_stays_flat_and_restore_rotates_current() {
    let mut file = make_workspace_file(json!({"value":1}), "Laptop", Some("Laptop"), None).unwrap();
    days_ago(&mut file, 10);
    for value in 2..=6 {
        file = make_workspace_file(json!({"value":value}), "Phone", Some("Phone"), Some(&file))
            .unwrap();
        days_ago(&mut file, 10 - value);
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
    days_ago(&mut skewed, 90);
    let next = make_workspace_file(json!({"value":7}), "Laptop", None, Some(&skewed)).unwrap();
    assert_eq!(next.previous[0].document, skewed.document); // conditional-write baseline survives clock skew
}

#[test]
fn an_envelope_that_would_not_fit_drops_its_oldest_history_first() {
    let large = "x".repeat(MAX_DOCUMENT_BYTES / 3);
    let mut file =
        make_workspace_file(json!({"value":0,"pad":large}), "Laptop", None, None).unwrap();
    for value in 1..=3 {
        days_ago(&mut file, 4 - value);
        file = make_workspace_file(
            json!({"value":value,"pad":large}),
            "Laptop",
            None,
            Some(&file),
        )
        .unwrap();
    }
    assert!(serde_json::to_vec(&file).unwrap().len() <= MAX_DOCUMENT_BYTES);
    let kept = file
        .previous
        .iter()
        .map(|old| old.document["value"].as_i64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(kept, [2], "the copy just replaced stays, older ones go");
}

#[test]
fn parses_workspace_wrapper_and_compares_only_restorable_content() {
    let file = make_workspace_file(document(json!([])), "device-a", None, None).unwrap();
    assert_eq!(
        parse_cloud_workspace(&serde_json::to_vec(&file).unwrap())
            .unwrap()
            .document["schemaVersion"],
        7
    );
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

    fn disconnect(&self, _provider: CloudProvider) -> Result<(), DocumentError> {
        Ok(())
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

    assert_eq!(event_ids(&merged), ["first-remote", "late-remote", "local"]);
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

/// An object store that keeps one object per provider and bumps its tag on
/// every accepted write, like CloudKit's record change tag.
#[derive(Default)]
struct Objects {
    stored: Mutex<Option<StoredObject>>,
    writes: Mutex<Vec<Option<String>>>,
}

impl ObjectStore for Objects {
    fn read<'a>(&'a self, _provider: CloudProvider) -> BoxFuture<'a, Option<StoredObject>> {
        Box::pin(async move { Ok(self.stored.lock().unwrap().clone()) })
    }

    fn write<'a>(
        &'a self,
        _provider: CloudProvider,
        bytes: Vec<u8>,
        expected: Option<String>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.writes.lock().unwrap().push(expected.clone());
            let mut stored = self.stored.lock().unwrap();
            if stored.as_ref().map(|object| object.tag.clone()) != expected {
                return Err(DocumentError::Conflict);
            }
            let tag = format!("tag-{}", self.writes.lock().unwrap().len());
            *stored = Some(StoredObject { bytes, tag });
            Ok(())
        })
    }

    fn disconnect(&self, _provider: CloudProvider) -> Result<(), DocumentError> {
        Ok(())
    }
}

#[tokio::test]
async fn tagged_writes_carry_the_tag_read_with_the_expected_revision() {
    let note = |id: &str| json!({"id": id, "rev": 1, "updatedAt": "2026-01-01T00:00:00Z", "updatedBy": "device"});
    let cloud = DocumentCloud::new(
        TaggedWorkspace::new(Objects::default()),
        Some("iPhone".into()),
    );
    let provider = CloudProvider::Icloud;

    // First copy: created only if nothing exists, then a second device's
    // edit is conditional on the tag of the copy it read.
    cloud
        .merge_and_push(provider, &document(json!([note("mac")])), "mac")
        .await
        .unwrap();
    let merged = cloud
        .merge_and_push(provider, &document(json!([note("phone")])), "phone")
        .await
        .unwrap();
    assert_eq!(event_ids(&merged), ["mac", "phone"]);
    let store = cloud.transport().store();
    assert_eq!(*store.writes.lock().unwrap(), [None, Some("tag-1".into())]);
    let versions = cloud.versions(provider).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].device, "iPhone");

    // A write whose revision was not the last one read never reaches the
    // store: another device may have replaced the object since.
    let workspace = cloud.transport();
    let current = workspace.read(provider).await.unwrap().unwrap();
    let stale = current.revision - 1;
    assert_eq!(
        workspace.write(provider, &current, Some(stale)).await,
        Err(DocumentError::Conflict)
    );
    assert_eq!(store.writes.lock().unwrap().len(), 2);
}
