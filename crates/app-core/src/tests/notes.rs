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
fn save_read_list_and_move_through_the_trash() {
    let notes = service("roundtrip");
    let saved = notes
        .save(context(), String::new(), "# 読書ノート\n".into())
        .unwrap();
    let read = notes.read(context(), saved.id.clone()).unwrap();
    assert_eq!(read.name, "読書ノート");
    assert_eq!(read.text.as_deref(), Some("# 読書ノート\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = notes.note_path(&context(), &saved.id).unwrap();
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    let listed = |notes: &Notes| {
        notes
            .list(context(), vec!["section-91".into()])
            .unwrap()
            .len()
    };
    assert_eq!(listed(&notes), 1);
    notes.move_note(context(), saved.id.clone(), false).unwrap();
    assert_eq!(listed(&notes), 0);
    notes.move_note(context(), saved.id.clone(), true).unwrap();
    assert_eq!(listed(&notes), 1);
}

#[test]
fn an_oversized_note_is_rejected_before_touching_storage() {
    let notes = service("limits");
    assert!(matches!(
        notes.save(
            context(),
            String::new(),
            "x".repeat(MAX_NOTE_BYTES as usize + 1),
        ),
        Err(NoteError::TooLarge)
    ));
    assert!(!notes.root.exists());
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
        .save(context(), String::new(), "still current".into())
        .unwrap();

    store
        .select_root(second_root.clone(), || Ok::<(), &str>(()))
        .unwrap();
    assert!(matches!(
        stale.save(context(), String::new(), "stale".into()),
        Err(NoteError::Storage(message))
            if message == "Course storage changed; retry with the current folder"
    ));
    store
        .at(second_root)
        .save(context(), String::new(), "new root".into())
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
        let _gate = notes.lock_selected().unwrap();
        entered_send.send(()).unwrap();
        release_receive.recv().unwrap();
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
    holder.join().unwrap();
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
        notes.save(context(), id, "replace".into()),
        Err(NoteError::Storage(message)) if message == "Cannot edit symbolic link"
    ));
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
}
