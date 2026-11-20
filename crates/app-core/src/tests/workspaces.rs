use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("core-workspace-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn archive(path: &Path, files: &[(&str, &str)]) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, body) in files {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

fn course() -> CourseKey {
    CourseKey {
        term: "2026-3Q".into(),
        course: "CSC.T251".into(),
    }
}

#[test]
fn cross_platform_names_and_zip_extension_remain_compatible() {
    assert_eq!(safe_name("OOP 1B: A/B?", "x"), "OOP 1B_ A_B_");
    assert_eq!(safe_name(" .. ", "x"), "x");
    assert_eq!(safe_name("..hidden", "x"), "hidden");
    let directory = Path::new("/tmp/Report 3");
    assert_eq!(zip_file_name("", directory), "Report 3.zip");
    assert_eq!(zip_file_name("24B12345", directory), "24B12345.zip");
    assert_eq!(zip_file_name("a.ZIP", directory), "a.ZIP");
}

#[test]
fn creates_once_and_preserves_existing_student_files() {
    let fixture = Fixture::new("create");
    let source = fixture.0.join("handout.txt");
    fs::write(&source, "instructions").unwrap();
    let service = Workspaces::new(fixture.0.clone());
    let created = service
        .create(
            &course(),
            "assignment-1".into(),
            "OOP 1B: A/B?".into(),
            "# Brief".into(),
            vec![Attachment {
                path: source.to_string_lossy().into_owned(),
                name: "handout.txt".into(),
            }],
        )
        .unwrap();
    let own = PathBuf::from(&created.path).join("student.txt");
    fs::write(&own, "keep").unwrap();
    let reopened = service
        .create(
            &course(),
            "assignment-1".into(),
            "renamed".into(),
            "replacement".into(),
            Vec::new(),
        )
        .unwrap();
    assert_eq!(created.path, reopened.path);
    assert_eq!(fs::read_to_string(own).unwrap(), "keep");
    assert_eq!(created.name, "OOP 1B_ A_B_");
}

#[test]
fn archive_paths_cannot_escape_and_junk_is_ignored() {
    let fixture = Fixture::new("unpack");
    let source = fixture.0.join("src.zip");
    archive(
        &source,
        &[
            ("problem/Main.java", "class Main {}"),
            ("__MACOSX/problem/._Main.java", "x"),
            ("../evil", "x"),
        ],
    );
    let output = fixture.0.join("output");
    fs::create_dir_all(&output).unwrap();
    unpack(&source, &output, "src").unwrap();
    assert_eq!(
        fs::read_to_string(output.join("problem/Main.java")).unwrap(),
        "class Main {}"
    );
    assert!(!fixture.0.join("evil").exists());
    assert!(!output.join("__MACOSX").exists());

    let loose = fixture.0.join("loose.zip");
    archive(&loose, &[("a.txt", "a"), ("b.txt", "b")]);
    unpack(&loose, &output, "loose").unwrap();
    assert!(output.join("loose/a.txt").is_file());
}

#[test]
fn submission_archive_is_recursive_and_skips_symlinks() {
    let fixture = Fixture::new("pack");
    let submit = fixture.0.join("submit");
    fs::create_dir_all(submit.join("src")).unwrap();
    fs::write(submit.join("report.pdf"), "pdf").unwrap();
    fs::write(submit.join("src/Main.java"), "java").unwrap();
    fs::write(submit.join(".DS_Store"), "junk").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(fixture.0.join("outside"), submit.join("linked")).unwrap();
    let output = fixture.0.join("out.zip");
    pack(&submit, &output).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(output).unwrap()).unwrap();
    let mut names: Vec<_> = (0..zip.len())
        .map(|index| zip.by_index(index).unwrap().name().unwrap().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["report.pdf", "src/", "src/Main.java"]);
}

#[cfg(unix)]
#[test]
fn workspace_validation_rejects_symlinked_or_outside_paths() {
    let fixture = Fixture::new("validation");
    let service = Workspaces::new(fixture.0.clone());
    let created = service
        .create(
            &course(),
            "assignment-1".into(),
            "Assignment".into(),
            String::new(),
            Vec::new(),
        )
        .unwrap();
    let link = fixture.0.join("link");
    std::os::unix::fs::symlink(&created.path, &link).unwrap();
    assert!(service.workspace_path(&link.to_string_lossy()).is_err());
    assert!(service
        .workspace_path(&fixture.0.to_string_lossy())
        .is_err());
}

#[test]
fn upload_reads_only_the_matching_assignments_submit_files_and_supports_zip() {
    use base64::Engine;
    let fixture = Fixture::new("upload");
    let service = Workspaces::new(fixture.0.clone());
    let workspace = service
        .create(
            &course(),
            "a1".into(),
            "Report".into(),
            String::new(),
            vec![],
        )
        .unwrap();
    let root = PathBuf::from(&workspace.path);
    fs::write(root.join("private.txt"), "not for submission").unwrap();
    fs::write(root.join("submit/report.txt"), "my report").unwrap();
    assert!(service
        .prepare_upload(&workspace.path, "another-assignment")
        .is_err());
    let uploaded = service.prepare_upload(&workspace.path, "a1").unwrap();
    assert_eq!(uploaded.files.len(), 1);
    assert_eq!(uploaded.files[0].name, "report.txt");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&uploaded.files[0].data)
            .unwrap(),
        b"my report"
    );
    fs::create_dir(root.join("submit/src")).unwrap();
    fs::write(root.join("submit/src/Main.java"), "class Main {}").unwrap();
    assert!(service.prepare_upload(&workspace.path, "a1").is_err());
    service
        .set_submission(&workspace.path, true, "student.zip")
        .unwrap();
    {
        let zip = service.prepare_upload(&workspace.path, "a1").unwrap();
        assert_eq!(zip.files.len(), 1);
        assert_eq!(zip.files[0].name, "student.zip");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&zip.files[0].data)
            .unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(archive.by_name("src/Main.java").is_ok());
        assert!(archive.by_name("private.txt").is_err());
    }
    service.set_submission(&workspace.path, false, "").unwrap();
    fs::remove_dir_all(root.join("submit/src")).unwrap();
    fs::File::create(root.join("submit/large.bin"))
        .unwrap()
        .set_len(20 * 1024 * 1024 + 1)
        .unwrap();
    assert!(service.prepare_upload(&workspace.path, "a1").is_err());
}

#[test]
fn folder_deleted_outside_the_app_is_missing_and_hand_in_falls_back_to_lms() {
    let fixture = Fixture::new("missing");
    let service = Workspaces::new(fixture.0.clone());
    let created = service
        .create(
            &course(),
            "a".into(),
            "Report".into(),
            String::new(),
            Vec::new(),
        )
        .unwrap();
    fs::write(PathBuf::from(&created.path).join("submit/answer.txt"), "x").unwrap();
    let open = HandInView::of(
        service.find(&course(), "a").unwrap(),
        HandInAvailability::PrepareDraft,
    );
    assert_eq!(open.hand_in, HandInAvailability::PrepareDraft);
    // Closed work goes to LMS even while its workspace is still there.
    let closed = HandInView::of(
        service.find(&course(), "a").unwrap(),
        HandInAvailability::OpenLms,
    );
    assert_eq!(closed.hand_in, HandInAvailability::OpenLms);
    assert!(closed.workspace.is_some());

    fs::remove_dir_all(&created.path).unwrap();
    assert!(matches!(
        service.prepare_upload(&created.path, "a"),
        Err(HandInError::Workspace(WorkspaceError::Missing))
    ));
    assert!(matches!(
        service.workspace_path(&created.path),
        Err(WorkspaceError::Missing)
    ));
    assert!(matches!(
        service.set_submission(&created.path, true, ""),
        Err(WorkspaceError::Missing)
    ));
    service.delete(&created.path).unwrap();
    // The shell reads the kind, not an OS message.
    assert_eq!(
        serde_json::to_value(crate::school_browser::SubmissionError::from(
            WorkspaceError::Missing
        ))
        .unwrap(),
        serde_json::json!({"kind": "workspaceMissing"})
    );
    let gone = HandInView::of(
        service.find(&course(), "a").unwrap(),
        HandInAvailability::PrepareDraft,
    );
    assert_eq!(gone.hand_in, HandInAvailability::OpenLms);
    assert!(gone.workspace.is_none());
}

#[test]
fn draft_upload_rechecks_the_stored_assignment_before_reading_files() {
    use crate::data::DataError;
    use serde_json::json;

    let fixture = Fixture::new("draft-eligibility");
    let store = DataStore::initialize_dir(fixture.0.join("data")).unwrap().0;
    let notes = NoteStore::default();
    notes.at(fixture.0.join("files"));
    let service = AssignmentWorkspaces::new(store.clone(), notes, Downloader::default());
    let folder = service
        .at()
        .unwrap()
        .create(
            &course(),
            "a".into(),
            "Report".into(),
            String::new(),
            Vec::new(),
        )
        .unwrap();
    fs::write(PathBuf::from(&folder.path).join("submit/answer.txt"), "x").unwrap();

    let page = "https://lms.s.isct.ac.jp/2026/mod/assign/view.php?id=7";
    let save = |accepts: bool| {
        let row = json!({
            "id": "a", "sourceUrl": page, "dueAt": "2026-01-01T00:00:00Z",
            "acceptsSubmission": accepts
        });
        store.write(None, |transaction| {
            transaction.execute(
                "INSERT INTO assignments (id,value,hash,updated_at,updated_by,rev)
                 VALUES ('a',?1,?2,'2026-01-01T00:00:00Z','seed',1)
                 ON CONFLICT (id) DO UPDATE SET value=excluded.value, hash=excluded.hash, rev=rev+1",
                rusqlite::params![row.to_string(), accepts.to_string()],
            ).map_err(DataError::from)?;
            Ok::<_, DataError>(())
        }).unwrap();
    };
    save(true);
    let (destination, prepared) = service.prepare_draft("a", Some(&folder.path)).unwrap();
    assert_eq!(destination, page);
    assert_eq!(prepared.files[0].name, "answer.txt");

    save(false);
    assert!(matches!(
        service.prepare_draft("a", Some(&folder.path)),
        Err(HandInError::Unavailable)
    ));
}

#[test]
fn the_layout_before_the_trees_moves_in_without_overwriting() {
    let fixture = Fixture::new("migrate");
    let root = &fixture.0;
    let old = root.join("2026-3Q/CSC.T251");
    let workspace = old.join("workspaces/Report 3");
    fs::create_dir_all(workspace.join(SUBMIT_DIR)).unwrap();
    fs::write(
        workspace.join(META),
        r#"{"assignment":"a1","title":"Report 3"}"#,
    )
    .unwrap();
    fs::write(workspace.join("submit/answer.pdf"), "mine").unwrap();
    fs::create_dir_all(old.join("materials")).unwrap();
    fs::write(old.join("materials/slides-1234abcd.pdf"), "old copy").unwrap();
    // Already in the material tree, so the old copy must not replace it.
    let newer = root.join("material/2026-3Q/CSC.T251/materials/slides-1234abcd.pdf");
    fs::create_dir_all(newer.parent().unwrap()).unwrap();
    fs::write(&newer, "new copy").unwrap();
    fs::create_dir_all(root.join("Photos")).unwrap();

    let first = storage::migrate_layout(root).unwrap();
    assert_eq!(first, storage::Moved { moved: 1, kept: 1 });
    let found = Workspaces::new(root.clone())
        .find(&course(), "a1")
        .unwrap()
        .unwrap();
    assert!(Path::new(&found.path).ends_with("workspace/2026-3Q/CSC.T251/Report 3"));
    assert_eq!(
        fs::read_to_string(Path::new(&found.path).join("submit/answer.pdf")).unwrap(),
        "mine"
    );
    assert_eq!(fs::read_to_string(&newer).unwrap(), "new copy");
    assert!(old.join("materials/slides-1234abcd.pdf").exists());
    assert!(!old.join("workspaces").exists());
    // A folder not named like a quarter is not the app's.
    assert!(root.join("Photos").is_dir());
    // Running again finds only what it already had to keep.
    assert_eq!(
        storage::migrate_layout(root).unwrap(),
        storage::Moved { moved: 0, kept: 1 }
    );
}

// The one `ASSIGNMENT.md` the desktop and its agents starts from: the LMS
// description as plain text (the row's notes when the page has none), the
// due time in Japan time, and every attachment, saying which were not saved.
#[test]
fn brief_carries_the_description_due_time_and_each_attachment() {
    let attachment = |name: &str| crate::schedule::AssignmentAttachment {
        name: name.into(),
        url: format!("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/{name}"),
    };
    let mut detail = AssignmentDetail {
        assignment_id: "a1".into(),
        course_id: "c1".into(),
        title: "Report 3".into(),
        due_at: Some("2026-11-05T14:59:00Z".into()),
        deadline: None,
        state: crate::schedule::AssignmentState::Open,
        hand_in_availability: HandInAvailability::PrepareDraft,
        submission: None,
        submission_status: None,
        grade: None,
        source_url: Some("https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=7".into()),
        body_html: "<p>Answer <b>all</b> questions.</p><ul><li>PDF only</li></ul>".into(),
        details: "row notes".into(),
        original_body_html: None,
        attachments: vec![attachment("sheet.pdf"), attachment("data.csv")],
    };
    let missing = [MissingAttachment {
        name: "data.csv".into(),
        reason: "login".into(),
    }];
    assert_eq!(
        brief(&detail, "Physics I", &missing),
        "# Report 3\n\n\
         - Course: Physics I\n\
         - Due: 2026-11-05 23:59 JST\n\
         - LMS: https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=7\n\
         - Hand in: put the files to submit in `submit/`\n\n\
         Answer all questions.\n\n- PDF only\n\n\
         ## Attachments\n\n\
         - sheet.pdf\n\
         - data.csv (not downloaded: login)\n"
    );
    // Without a description, due time, page or attachments, only what is
    // known is written.
    detail.body_html.clear();
    detail.attachments.clear();
    detail.due_at = None;
    detail.source_url = None;
    assert_eq!(
        brief(&detail, "", &[]),
        "# Report 3\n\n- Hand in: put the files to submit in `submit/`\n\nrow notes\n"
    );
}
