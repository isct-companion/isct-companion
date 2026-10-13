use super::*;
use crate::notes::NoteStore;
use serde_json::json;

struct Fixture {
    root: PathBuf,
    backup: BackupService,
    store: DataStore,
    notes: Notes,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("core-backup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize(root.join("workspace.db")).unwrap().0;
        let notes = NoteStore::default().at(root.join("files"));
        let backup = BackupService::new(store.clone(), notes.clone());
        Self {
            root,
            backup,
            store,
            notes,
        }
    }
    fn add_note(&self) {
        let note = serde_json::from_value(json!({"id": uuid::Uuid::new_v4().to_string(), "term":"2026-3Q", "course":"CSC.T251", "section":"intro", "text":"# A preserved note", "updatedAt":"2026-10-13T10:00:00Z", "updatedBy":"fixture", "rev":1})).unwrap();
        self.notes.merge(&[note]).unwrap();
    }
    fn add_plan(&self, title: &str) {
        self.store.put_user_record("plans", json!({"title": title, "start":"2026-10-13T10:00:00+09:00", "duration":60, "exdates":[]}), "test").unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn backup_roundtrip_preserves_notes_records_identity_and_later_edits() {
    let source = Fixture::new();
    source.add_note();
    source.add_plan("original");
    let document = source.backup.snapshot().unwrap();
    BackupService::validate(&document).unwrap();
    let target = Fixture::new();
    let identity = target.backup.device_id().unwrap();
    target.backup.apply(&document).unwrap();
    assert_eq!(target.notes.export().unwrap()[0].text, "# A preserved note");
    assert_eq!(target.store.user_rows("plans").unwrap().len(), 1);
    assert_eq!(target.backup.device_id().unwrap(), identity);
    target.add_plan("edited while remote request was running");
    target.backup.apply(&document).unwrap();
    assert_eq!(target.store.user_rows("plans").unwrap().len(), 2);
}

#[test]
fn replayed_restore_generation_cannot_resurrect_removed_notes_or_records() {
    let fixture = Fixture::new();
    fixture.add_note();
    fixture.add_plan("before restore");
    let old = fixture.backup.snapshot().unwrap();
    let mut restored = old.clone();
    restored["localNotes"] = json!([]);
    restored["tables"]["plans"] = json!([]);
    restored["sync"] =
        json!({"workspaceId":"workspace", "generation":{"counter":1,"id":"restore"}});
    fixture.backup.apply(&restored).unwrap();
    fixture.backup.apply(&restored).unwrap();
    fixture.backup.apply(&old).unwrap();
    assert!(fixture.notes.export().unwrap().is_empty());
    assert!(fixture.store.user_rows("plans").unwrap().is_empty());
}

#[test]
fn invalid_notes_are_rejected_before_any_local_changes() {
    let fixture = Fixture::new();
    fixture.add_note();
    fixture.add_plan("kept");
    let before = fixture.backup.snapshot().unwrap();
    let mut invalid = before.clone();
    invalid["localNotes"][0]["term"] = json!("../../outside");
    invalid["tables"]["plans"] = json!([]);
    assert!(BackupService::validate(&invalid).is_err());
    assert!(fixture.backup.apply(&invalid).is_err());
    assert_eq!(
        cloud_merge::contents(&fixture.backup.snapshot().unwrap()),
        cloud_merge::contents(&before)
    );
}

#[test]
fn scheduled_export_replaces_a_complete_document_in_the_same_format() {
    let fixture = Fixture::new();
    fixture.add_note();
    let directory = fixture.root.join("scheduled");
    let path = fixture.backup.write_scheduled(&directory).unwrap();
    fixture.add_plan("second snapshot");
    assert_eq!(fixture.backup.write_scheduled(&directory).unwrap(), path);
    let document: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    BackupService::validate(&document).unwrap();
    assert_eq!(document["tables"]["plans"].as_array().unwrap().len(), 1);
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
