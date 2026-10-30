use super::*;
use crate::data::WorkspaceChangeSource;
use serde_json::json;

struct Fixture {
    root: PathBuf,
    backup: BackupService,
    store: DataStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("core-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize_dir(root.clone()).unwrap().0;
        let backup = BackupService::new(store.clone());
        Self {
            root,
            backup,
            store,
        }
    }
    fn add_event(&self, title: &str) -> String {
        let saved = self.store.put_user_record("personal_events", json!({"title": title, "kind": "event", "termId": "2026-3Q", "room": "", "days": [1], "startTime": "10:00", "endTime": "11:00"})).unwrap();
        saved["id"].as_str().unwrap().to_owned()
    }
    /// Titles of the events that are not deleted.
    fn events(&self) -> Vec<String> {
        let mut titles = self
            .store
            .user_rows("personal_events")
            .unwrap()
            .into_iter()
            .filter(|row| row["deletedAt"].is_null())
            .map(|row| row["title"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        titles.sort();
        titles
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn backup_roundtrip_preserves_records_identity_and_later_edits() {
    let source = Fixture::new();
    source.add_event("original");
    let document = source.backup.snapshot().unwrap();
    BackupService::validate(&document).unwrap();
    assert!(document.get("localNotes").is_none());
    let target = Fixture::new();
    let identity = target.backup.device_id().unwrap();
    target
        .backup
        .apply(&document, WorkspaceChangeSource::CloudSync)
        .unwrap();
    assert_eq!(target.events(), ["original"]);
    assert_eq!(target.backup.device_id().unwrap(), identity);
    target.add_event("edited while remote request was running");
    target
        .backup
        .apply(&document, WorkspaceChangeSource::CloudSync)
        .unwrap();
    assert_eq!(target.events().len(), 2);
}

/// A restore is the newest edit: it wins over the deletion and the addition
/// it undoes, on this device and on one that merges it, while that device's
/// own offline edit to another record survives.
#[test]
fn restore_rewrites_old_records_as_new_edits_that_other_devices_merge() {
    let laptop = Fixture::new();
    laptop.add_event("kept");
    let deleted = laptop.add_event("deleted by mistake");
    let old = laptop.backup.snapshot().unwrap();
    laptop
        .store
        .delete_user_record("personal_events", &deleted)
        .unwrap();
    laptop.add_event("added after");
    let phone = Fixture::new();
    phone
        .backup
        .apply(
            &laptop.backup.snapshot().unwrap(),
            WorkspaceChangeSource::CloudSync,
        )
        .unwrap();
    phone.add_event("offline on the phone");

    laptop
        .backup
        .restore(&old, WorkspaceChangeSource::CloudRestore)
        .unwrap();
    assert_eq!(laptop.events(), ["deleted by mistake", "kept"]);
    let restored = laptop.backup.snapshot().unwrap();
    // Restoring the same copy again changes nothing.
    laptop
        .backup
        .restore(&old, WorkspaceChangeSource::CloudRestore)
        .unwrap();
    assert_eq!(
        crate::data::cloud_merge::version(&laptop.backup.snapshot().unwrap()).unwrap(),
        crate::data::cloud_merge::version(&restored).unwrap()
    );

    phone
        .backup
        .apply(&restored, WorkspaceChangeSource::CloudSync)
        .unwrap();
    assert_eq!(
        phone.events(),
        ["deleted by mistake", "kept", "offline on the phone"]
    );
    // The old copy arriving late cannot undo the restore.
    phone
        .backup
        .apply(&old, WorkspaceChangeSource::CloudSync)
        .unwrap();
    assert_eq!(
        phone.events(),
        ["deleted by mistake", "kept", "offline on the phone"]
    );
}

#[test]
fn invalid_records_are_rejected_before_any_local_changes() {
    let fixture = Fixture::new();
    fixture.add_event("kept");
    let before = fixture.backup.snapshot().unwrap();
    let mut invalid = before.clone();
    invalid["tables"]["personal_events"][0]["rev"] = json!("not a revision");
    assert!(BackupService::validate(&invalid).is_err());
    assert!(fixture
        .backup
        .apply(&invalid, WorkspaceChangeSource::CloudSync)
        .is_err());
    assert!(fixture
        .backup
        .restore(&invalid, WorkspaceChangeSource::CloudRestore)
        .is_err());
    assert_eq!(fixture.events(), ["kept"]);
}

#[test]
fn scheduled_export_replaces_a_complete_document_in_the_same_format() {
    let fixture = Fixture::new();
    let directory = fixture.root.join("scheduled");
    let path = fixture.backup.write_scheduled(&directory).unwrap();
    fixture.add_event("second snapshot");
    assert_eq!(fixture.backup.write_scheduled(&directory).unwrap(), path);
    let document: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    BackupService::validate(&document).unwrap();
    assert_eq!(
        document["tables"]["personal_events"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
