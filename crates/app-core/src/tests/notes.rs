use super::*;

fn service(name: &str) -> Notes {
    NoteStore::default()
        .at(std::env::temp_dir().join(format!("isct-notes-{name}-{}", uuid::Uuid::new_v4())))
}

fn context() -> CourseFolder {
    CourseFolder {
        term: "2026-3Q".into(),
        course: "CSC.T251".into(),
        section: "section-91".into(),
    }
}

#[test]
fn save_read_move_list_and_backup_roundtrip_preserve_wire_format() {
    let notes = service("roundtrip");
    let saved = notes
        .save(
            context(),
            String::new(),
            "# 読書ノート\n".into(),
            "device-a",
        )
        .unwrap();
    let read = notes.read(context(), saved.id.clone()).unwrap();
    assert_eq!(read.name, "読書ノート");
    assert_eq!(read.text.as_deref(), Some("# 読書ノート\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = notes.note_path(&context(), &saved.id).unwrap();
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            path.with_extension("sync.json")
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(
        notes
            .list(context(), vec!["section-91".into()])
            .unwrap()
            .len(),
        1
    );

    notes
        .move_note(context(), saved.id.clone(), false, "device-a")
        .unwrap();
    let backup = notes.export().unwrap();
    assert_eq!(backup.len(), 1);
    assert!(backup[0].deleted);
    assert_eq!(backup[0].updated_by, "device-a");
    assert_eq!(backup[0].rev, 2);

    let restored = service("restored");
    restored.restore(&backup).unwrap();
    assert_eq!(restored.export().unwrap(), backup);
}

#[test]
fn validation_rejects_limits_before_touching_storage() {
    let notes = service("limits");
    assert!(matches!(
        notes.save(
            context(),
            String::new(),
            "x".repeat(MAX_NOTE_BYTES as usize + 1),
            "device-a"
        ),
        Err(NoteError::TooLarge)
    ));
    let oversized = LocalNoteBackup {
        id: uuid::Uuid::new_v4().to_string(),
        term: context().term,
        course: context().course,
        section: context().section,
        text: "x".repeat(MAX_NOTE_BYTES as usize + 1),
        deleted: false,
        updated_at: String::new(),
        updated_by: String::new(),
        rev: 0,
        deleted_at: None,
    };
    assert!(validate_notes(&[oversized]).is_err());
    assert!(!notes.root.exists());
}

#[test]
fn export_does_not_truncate_a_large_section_and_rejects_the_real_limit() {
    let notes = service("export-count");
    let directory = storage_dir(
        &crate::storage::material_root(&notes.root),
        &["2026-3Q", "CSC.T251", "section-91", "notes"],
    )
    .unwrap();
    let mut ids = Vec::new();
    for _ in 0..501 {
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(directory.join(format!("{id}.md")), "note").unwrap();
        ids.push(id);
    }
    let exported = notes.export().unwrap();
    assert_eq!(exported.len(), 501);
    assert!(ids
        .iter()
        .all(|id| exported.iter().any(|note| &note.id == id)));

    for _ in 501..=MAX_BACKUP_NOTES {
        let id = uuid::Uuid::new_v4();
        fs::write(directory.join(format!("{id}.md")), "note").unwrap();
    }
    assert!(matches!(
        notes.export(),
        Err(NoteError::Input(message))
            if message == "Notes exceed the local backup safety limit"
    ));
}

#[test]
fn shared_lock_serializes_versions_across_service_values() {
    let root = std::env::temp_dir().join(format!("isct-notes-lock-{}", uuid::Uuid::new_v4()));
    let store = NoteStore::default();
    let first = store.at(root.clone());
    let second = store.at(root.clone());
    let id = uuid::Uuid::new_v4().to_string();
    let one = {
        let id = id.clone();
        std::thread::spawn(move || first.save(context(), id, "one".into(), "device-a"))
    };
    let two = {
        let id = id.clone();
        std::thread::spawn(move || second.save(context(), id, "two".into(), "device-b"))
    };
    one.join().unwrap().unwrap();
    two.join().unwrap().unwrap();
    // Both writers advanced the same sidecar instead of racing at rev 1.
    let backup = store.at(root).export().unwrap();
    assert_eq!(backup.len(), 1);
    assert_eq!(backup[0].id, id);
    assert_eq!(backup[0].rev, 2);
}

#[test]
fn merge_updates_revision_even_when_note_contents_are_unchanged() {
    let notes = service("merge-revision");
    let saved = notes
        .save(context(), String::new(), "same".into(), "device-a")
        .unwrap();
    let mut incoming = notes.export().unwrap().remove(0);
    incoming.rev += 1;
    notes.merge(&[incoming.clone()]).unwrap();
    let exported = notes.export().unwrap();
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].id, saved.id);
    assert_eq!(exported[0].rev, incoming.rev);
}

#[test]
fn merge_moves_an_identity_without_leaving_the_old_path() {
    let notes = service("merge-location");
    let saved = notes
        .save(context(), String::new(), "move me".into(), "device-a")
        .unwrap();
    let old_path = notes.note_path(&context(), &saved.id).unwrap();
    let mut incoming = notes.export().unwrap().remove(0);
    incoming.section = "section-92".into();
    incoming.rev += 1;
    notes.merge(&[incoming.clone()]).unwrap();

    let exported = notes.export().unwrap();
    assert_eq!(exported, vec![incoming]);
    assert!(!old_path.exists());
    assert!(!old_path.with_extension("sync.json").exists());
}

#[test]
fn root_selection_rejects_stale_handles_and_changes_only_after_persisting() {
    let base = std::env::temp_dir().join(format!(
        "isct-notes-root-selection-{}",
        uuid::Uuid::new_v4()
    ));
    let first_root = base.join("first");
    let second_root = base.join("second");
    let store = NoteStore::default();
    let stale = store.at(first_root.clone());

    assert!(store
        .select_root(second_root.clone(), || Err::<(), _>("persist failed"))
        .is_err());
    stale
        .save(context(), String::new(), "still current".into(), "device-a")
        .unwrap();

    store
        .select_root(second_root.clone(), || Ok::<(), &str>(()))
        .unwrap();
    assert!(matches!(
        stale.save(context(), String::new(), "stale".into(), "device-a"),
        Err(NoteError::Storage(message))
            if message == "Course storage changed; retry with the current folder"
    ));
    store
        .at(second_root)
        .save(context(), String::new(), "new root".into(), "device-a")
        .unwrap();
    let _ = fs::remove_dir_all(base);
}

#[test]
fn constructing_a_handle_does_not_wait_for_long_note_io() {
    let root = std::env::temp_dir().join(format!(
        "isct-notes-nonblocking-at-{}",
        uuid::Uuid::new_v4()
    ));
    let store = NoteStore::default();
    let notes = store.at(root.clone());
    let (entered_send, entered_receive) = std::sync::mpsc::channel();
    let (release_send, release_receive) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        notes.transaction::<_, NoteError>(|_| {
            entered_send.send(()).unwrap();
            release_receive.recv().unwrap();
            Ok(())
        })
    });
    entered_receive
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();

    let (finished_send, finished_receive) = std::sync::mpsc::channel();
    let constructor = std::thread::spawn(move || {
        let _ = store.at(root);
        finished_send.send(()).unwrap();
    });
    finished_receive
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("at() waited for the filesystem gate");

    release_send.send(()).unwrap();
    holder.join().unwrap().unwrap();
    constructor.join().unwrap();
}

#[cfg(unix)]
#[test]
fn symbolic_link_note_is_rejected() {
    use std::os::unix::fs::symlink;
    let notes = service("symlink");
    let id = uuid::Uuid::new_v4().to_string();
    let dir = storage_dir(
        &crate::storage::material_root(&notes.root),
        &["2026-3Q", "CSC.T251", "section-91", "notes"],
    )
    .unwrap();
    let outside = notes.root.join("outside.md");
    fs::write(&outside, "keep").unwrap();
    symlink(&outside, dir.join(format!("{id}.md"))).unwrap();
    assert!(matches!(
        notes.save(context(), id, "replace".into(), "device-a"),
        Err(NoteError::Storage(message)) if message == "Cannot edit symbolic link"
    ));
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
}

#[cfg(unix)]
#[test]
fn failed_restore_does_not_delete_notes_omitted_by_the_backup() {
    use std::os::unix::fs::symlink;
    let notes = service("restore-failure");
    let existing = notes
        .save(context(), String::new(), "keep".into(), "device-a")
        .unwrap();
    let incoming_id = uuid::Uuid::new_v4().to_string();
    let dir = storage_dir(
        &crate::storage::material_root(&notes.root),
        &["2026-3Q", "CSC.T251", "section-91", "notes"],
    )
    .unwrap();
    let outside = notes.root.join("outside.md");
    fs::write(&outside, "outside").unwrap();
    symlink(&outside, dir.join(format!("{incoming_id}.md"))).unwrap();
    let incoming = LocalNoteBackup {
        id: incoming_id,
        term: "2026-3Q".into(),
        course: "CSC.T251".into(),
        section: "section-91".into(),
        text: "incoming".into(),
        deleted: false,
        updated_at: chrono::Utc::now().to_rfc3339(),
        updated_by: "device-b".into(),
        rev: 1,
        deleted_at: None,
    };
    assert!(notes.restore(&[incoming]).is_err());
    assert_eq!(
        notes.read(context(), existing.id).unwrap().text.as_deref(),
        Some("keep")
    );
}
