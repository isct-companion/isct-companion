use super::*;
use crate::course_materials::tests::{enrolled, store};

fn service(name: &str) -> Notes {
    NoteStore::default()
        .at(std::env::temp_dir().join(format!("isct-notes-{name}-{}", uuid::Uuid::new_v4())))
}

/// A section of a course LMS enrolment wrote, and the folder Core names for
/// that course.
fn context(name: &str) -> (DataStore, CourseFolder, CourseKey) {
    let data = store(name);
    let (course_id, folder) = enrolled(&data);
    let context = CourseFolder {
        course_id,
        section: "section-91".into(),
    };
    (data, context, folder)
}

#[test]
fn save_read_list_and_move_through_the_trash() {
    let notes = service("roundtrip");
    let (data, context, _) = context("roundtrip");
    let saved = notes
        .save(
            &data,
            context.clone(),
            String::new(),
            "# 読書ノート\n".into(),
        )
        .unwrap();
    let read = notes
        .read(&data, context.clone(), saved.id.clone())
        .unwrap();
    assert_eq!(read.name, "読書ノート");
    assert_eq!(read.text.as_deref(), Some("# 読書ノート\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let folder = course_folder(&data, &context.course_id).unwrap();
        let path = notes.note_path(&folder, "section-91", &saved.id).unwrap();
        assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
    let listed = |notes: &Notes| {
        notes
            .list(&data, &context.course_id, vec!["section-91".into()])
            .unwrap()
            .len()
    };
    assert_eq!(listed(&notes), 1);
    notes
        .move_note(&data, context.clone(), saved.id.clone(), false)
        .unwrap();
    assert_eq!(listed(&notes), 0);
    notes
        .move_note(&data, context.clone(), saved.id.clone(), true)
        .unwrap();
    // A course Core does not know has no folder to keep notes in.
    assert!(matches!(
        notes.save(
            &data,
            CourseFolder {
                course_id: "unknown".into(),
                section: "section-91".into()
            },
            String::new(),
            "lost".into()
        ),
        Err(NoteError::Input(_))
    ));
    assert_eq!(listed(&notes), 1);
}

#[test]
fn an_oversized_note_is_rejected_before_touching_storage_and_one_at_the_limit_is_kept() {
    let notes = service("limits");
    let (data, context, _) = context("limits");
    assert!(matches!(
        notes.save(
            &data,
            context.clone(),
            String::new(),
            "x".repeat(MAX_NOTE_BYTES as usize + 1),
        ),
        Err(NoteError::TooLarge)
    ));
    assert!(!notes.root.exists());
    assert!(notes
        .save(
            &data,
            context,
            String::new(),
            "a".repeat(MAX_NOTE_BYTES as usize)
        )
        .is_ok());
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
    let (data, context, _) = context("root-selection");

    assert!(store
        .select_root(second_root.clone(), || Err::<(), _>("persist failed"))
        .is_err());
    stale
        .save(
            &data,
            context.clone(),
            String::new(),
            "still current".into(),
        )
        .unwrap();

    store
        .select_root(second_root.clone(), || Ok::<(), &str>(()))
        .unwrap();
    assert!(matches!(
        stale.save(&data, context.clone(), String::new(), "stale".into()),
        Err(NoteError::Storage(message))
            if message == "Course storage changed; retry with the current folder"
    ));
    store
        .at(second_root)
        .save(&data, context, String::new(), "new root".into())
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
    let (data, context, folder) = context("symlink");
    let id = uuid::Uuid::new_v4().to_string();
    let dir = storage_dir(
        &crate::storage::material_root(&notes.root),
        &[&folder.term, &folder.course, "section-91", "notes"],
    )
    .unwrap();
    let outside = notes.root.join("outside.md");
    fs::write(&outside, "keep").unwrap();
    symlink(&outside, dir.join(format!("{id}.md"))).unwrap();
    assert!(matches!(
        notes.save(&data, context, id, "replace".into()),
        Err(NoteError::Storage(message)) if message == "Cannot edit symbolic link"
    ));
    assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
}
