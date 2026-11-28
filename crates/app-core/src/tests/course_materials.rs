use super::*;
use crate::storage::{component, stored_file};
use serde_json::json;
use uuid::Uuid;

pub(crate) fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("course-materials-{name}-{}", Uuid::new_v4()));
    DataStore::initialize_dir(path).unwrap().0
}

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("course-materials-{name}-{}", Uuid::new_v4()))
}

#[cfg(unix)]
fn course() -> CourseKey {
    CourseKey {
        term: "2026-3Q".into(),
        course: "CSC.T251".into(),
    }
}

/// A course as LMS enrolment writes it: its id and the folder Core names.
pub(crate) fn enrolled(store: &DataStore) -> (String, CourseKey) {
    let enrolment: adpt_isct::lms::LmsEnrolment = serde_json::from_value(json!({
        "language":"en",
        "courses":[{"id":99,"idnumber":"202602431","shortname":"CSC.T273[2026]","fullname":"Compilers","coursecategory":3}],
        "timetables":[{"term":"2026-3Q","meetings":{"99":[{"day":2,"periods":[3,4]}]},"unscheduled":[]}]
    }))
    .unwrap();
    store
        .write(
            Some(crate::data::WorkspaceChangeSource::SchoolPull),
            |transaction| {
                crate::lms_ingest::apply_lms_enrolment(transaction, &enrolment, crate::clock::now())
                    .map_err(|error| crate::data::DataError::from(error.to_string()))
            },
        )
        .unwrap();
    let workspace = store.load_fields(&["courses"]).unwrap();
    let row = &workspace["courses"][0];
    (
        row["id"].as_str().unwrap().to_owned(),
        crate::schedule::files_of(row).unwrap(),
    )
}

#[test]
fn url_paths_and_names_preserve_the_desktop_contract() {
    for value in ["..", ".", "x/y", "x\\y", "", "../../notes"] {
        assert!(component(value).is_err());
    }
    assert!(component("2026-3Q").is_ok());
    assert!(component("CSC.T251").is_ok());
    let japanese = material_name(&Url::parse("https://lms.s.isct.ac.jp/2025/pluginfile.php/224957/mod_resource/content/1/01%E5%9B%9E%E7%9B%AE%20SW-0%20v6.pdf").unwrap());
    assert!(japanese.starts_with("01回目 SW-0 v6-") && japanese.ends_with(".pdf"));
    assert_ne!(
        material_name(&Url::parse("https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.pdf").unwrap()),
        material_name(&Url::parse("https://lms.s.isct.ac.jp/2025/pluginfile.php/2/a.pdf").unwrap())
    );
    assert!(matches!(
        lms_resource_url("https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=42"),
        Some((_, LmsResourceKind::View { cmid: 42, .. }))
    ));
    assert!(!safe_external_url("https://user:secret@example.test/file"));
}

/// Every source has an answer, and what is not a file to fetch is decided
/// without a request: the downloader here has no school session, so a
/// request would end in a failure instead of the answer asserted.
#[tokio::test]
async fn cached_hit_missing_and_pages_do_not_reach_the_school() {
    let root = root("cache");
    let store = store("cache");
    let service = CourseMaterials::new(store.clone(), root.clone(), Downloader::default());
    let (id, folder) = enrolled(&store);
    let open = |source: &str| {
        let (service, id, source) = (service.clone(), id.clone(), source.to_owned());
        async move { service.open(id, source, true, false, None).await.unwrap() }
    };
    let invalid = Material::Failed {
        reason: MaterialFailure::Invalid,
        page: None,
    };
    let page = |site, url: &str| Material::Page {
        site,
        url: url.into(),
    };

    // LMS activities that are not files open as their LMS page: an LTI tool
    // (the course survey), a forum, a feedback, a module Core has never
    // seen, and an activity address without an id.
    for source in [
        "https://lms.s.isct.ac.jp/2025/mod/ltifmane/view.php?id=150975",
        "https://lms.s.isct.ac.jp/2025/mod/forum/view.php?id=140001",
        "https://lms.s.isct.ac.jp/2025/mod/feedback/view.php?id=140002",
        "https://lms.s.isct.ac.jp/2025/mod/hvp/view.php?id=140003",
        "https://lms.s.isct.ac.jp/2025/mod/resource/view.php",
    ] {
        assert_eq!(open(source).await, page(SchoolSite::ScienceLms, source));
    }
    // A link to a school site with an adapter opens in its signed-in view;
    // the same host over HTTP would be refused there, so it is no page.
    let syllabus = "https://syllabus.s.isct.ac.jp/courses/2025/4/0-904-342300-0-0/202503924";
    assert_eq!(open(syllabus).await, page(SchoolSite::Syllabus, syllabus));
    assert_eq!(open(&syllabus.replacen("https", "http", 1)).await, invalid);
    let video = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    assert_eq!(open(video).await, Material::External { url: video.into() });
    for corrupt in [
        "",
        "lecture slides",
        "javascript:alert(1)",
        "file:///etc/hosts",
    ] {
        assert_eq!(open(corrupt).await, invalid, "{corrupt:?}");
    }
    // A `mod/url` resolved before keeps what it resolved to: a school site
    // with an adapter is its page, anything else an outside page.
    let forms = "https://forms.office.com/r/AbCdEf1234";
    for (cmid, resolved, expected) in [
        (140010, syllabus, page(SchoolSite::Syllabus, syllabus)),
        (140011, forms, Material::External { url: forms.into() }),
    ] {
        let source = format!("https://lms.s.isct.ac.jp/2025/mod/url/view.php?id={cmid}");
        store
            .test_connection()
            .execute(
                "INSERT INTO lms_items (id,value,hash,updated_at,updated_by,rev,source)
                     VALUES (?1,?2,'h','2026-10-01T00:00:00Z','lms',1,'lms')",
                [
                    format!("m{cmid}"),
                    json!({"id":format!("m{cmid}"),"cmid":cmid,"module":"url","url":source,"resourceUrl":resolved})
                        .to_string(),
                ],
            )
            .unwrap();
        assert_eq!(open(&source).await, expected);
    }
    // A course that is gone has no folder for the file, and the item still
    // opens on its LMS page.
    let file = "https://lms.s.isct.ac.jp/2026/pluginfile.php/3/handout.pdf";
    assert_eq!(
        service
            .open("gone".into(), file.into(), true, false, None)
            .await
            .unwrap(),
        Material::Failed {
            reason: MaterialFailure::Unresolved,
            page: Some(SchoolWebTarget {
                site: SchoolSite::ScienceLms,
                url: file.into(),
            }),
        }
    );

    // The file in the folder the course page names is read without a request.
    let source = "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/readme.txt";
    let url = Url::parse(source).unwrap();
    let path = material_path(&root, &folder, &url).unwrap();
    fs::write(&path, "cached text").unwrap();

    let material = service
        .open(id.clone(), source.into(), true, true, Some(1))
        .await
        .unwrap();
    let Material::Ready { file, .. } = material else {
        panic!("cached material should be ready");
    };
    assert!(file.cached);
    assert_eq!(file.text.as_deref(), Some("cached text"));

    let missing = service
        .open(
            id,
            "https://lms.s.isct.ac.jp/2026/pluginfile.php/2/missing.pdf".into(),
            false,
            false,
            None,
        )
        .await
        .unwrap();
    assert!(matches!(missing, Material::Missing { .. }));
}

/// A downloaded ZIP opens as its files, and each one opens like any other
/// source: extracted next to the archive under a cache name of its own,
/// bounded by the same download limit, without a request.
#[tokio::test]
async fn a_zip_opens_as_its_files_and_each_file_as_a_course_file() {
    let root = root("archive");
    let store = store("archive");
    let service = CourseMaterials::new(store.clone(), root.clone(), Downloader::default());
    let (id, folder) = enrolled(&store);
    let source =
        "https://lms.s.isct.ac.jp/2026/pluginfile.php/7/mod_resource/content/1/OOP-2A-problem.zip";
    let zip = material_path(&root, &folder, &Url::parse(source).unwrap()).unwrap();
    crate::file_preview::archive::tests::archive(
        &zip,
        &[
            ("OOP-2A/README.md", "# 2A"),
            ("OOP-2A/Main.java", "class Main {}"),
        ],
    );

    let Material::Archive { file, files, page } = service
        .open(id.clone(), source.into(), true, false, None)
        .await
        .unwrap()
    else {
        panic!("a listable zip opens as an archive");
    };
    assert_eq!(file.path, zip.to_string_lossy());
    assert_eq!(page.url, source);
    assert_eq!(
        files,
        [
            ArchiveFile {
                name: "OOP-2A/Main.java".into(),
                url: format!("{source}#entry=OOP-2A/Main.java"),
                bytes: 13,
            },
            ArchiveFile {
                name: "OOP-2A/README.md".into(),
                url: format!("{source}#entry=OOP-2A/README.md"),
                bytes: 4,
            },
        ]
    );

    let readme = &files[1].url;
    let Material::Ready { file, page } = service
        .open(id.clone(), readme.clone(), true, true, Some(1024))
        .await
        .unwrap()
    else {
        panic!("a file of the archive is ready");
    };
    assert_eq!(page.url, source);
    assert_eq!(file.text.as_deref(), Some("# 2A"));
    let extracted = Path::new(&file.path);
    assert_eq!(extracted.parent(), zip.parent());
    assert!(is_material_cache_name(extracted));
    assert!(file.name.starts_with("README-") && file.name.ends_with(".md"));
    // The extracted file is shown by its own kind, the archive's file name aside.
    assert_eq!(file.viewer, crate::file_preview::Viewer::Markdown);

    assert!(matches!(
        service
            .open(id.clone(), files[0].url.clone(), true, false, Some(4))
            .await
            .unwrap(),
        Material::TooLarge { bytes: 13, .. }
    ));
    let gone = archive_entry_source(source, "OOP-2A/gone.txt");
    assert!(matches!(
        service
            .open(id.clone(), gone, true, false, None)
            .await
            .unwrap(),
        Material::Failed {
            reason: MaterialFailure::Unresolved,
            ..
        }
    ));
    // Names keep Japanese and spaces through the source.
    let named = archive_entry_source(source, "第1回/課題 1.pdf");
    assert_eq!(
        archive_entry(&named),
        Some((source.to_owned(), "第1回/課題 1.pdf".to_owned()))
    );

    // An archive too big to show as a folder is the file itself.
    let many = "https://lms.s.isct.ac.jp/2026/pluginfile.php/8/many.zip";
    let names: Vec<String> = (0..=1000).map(|n| format!("f{n}.txt")).collect();
    crate::file_preview::archive::tests::archive(
        &material_path(&root, &folder, &Url::parse(many).unwrap()).unwrap(),
        &names
            .iter()
            .map(|name| (name.as_str(), ""))
            .collect::<Vec<_>>(),
    );
    assert!(matches!(
        service
            .open(id, many.into(), true, false, None)
            .await
            .unwrap(),
        Material::Ready { .. }
    ));
}

#[test]
fn stored_folders_validate_every_file_and_fetch_errors_keep_wire_reasons() {
    let valid = json!({"folderFiles":[
        {"name":"one","url":"https://lms.s.isct.ac.jp/2026/pluginfile.php/1/one.pdf"},
        {"name":"two","url":"https://lms.s.isct.ac.jp/2026/pluginfile.php/2/two.pdf"}
    ]});
    assert_eq!(stored_folder_files(Some(&valid)).unwrap().len(), 2);
    let invalid = json!({"folderFiles":[
        {"name":"one","url":"https://lms.s.isct.ac.jp/2026/pluginfile.php/1/one.pdf"},
        {"name":"bad","url":"https://example.test/two.pdf"}
    ]});
    assert!(stored_folder_files(Some(&invalid)).is_none());
    // The desktop file card reads these wire names.
    assert_eq!(
        serde_json::to_value(failure_of(FetchError::Login)).unwrap(),
        "login"
    );
    assert_eq!(
        serde_json::to_value(failure_of(FetchError::WriteDenied)).unwrap(),
        "denied"
    );
    assert_eq!(bounded_download_limit(Some(0)), Some(1));
    assert_eq!(
        bounded_download_limit(Some(u64::MAX)),
        Some(MAX_AUTO_DOWNLOAD_BYTES)
    );
    assert_eq!(bounded_download_limit(None), None);
    assert!(is_material_cache_name(Path::new("foo.bar-12345678")));
    assert!(is_material_cache_name(Path::new("foo.bar-12345678.pdf")));
    assert!(!is_material_cache_name(Path::new(
        "README-12345678.part-10-2"
    )));
}

/// Controlled folder access refuses creating the course folder in Documents
/// as a read-only parent does. The refusal must reach the file card as
/// `Denied`, and creating it must succeed once the user allows the app.
#[cfg(unix)]
#[test]
fn a_refused_course_folder_is_denied_and_works_after_access_is_allowed() {
    use std::os::unix::fs::PermissionsExt;
    let parent = root("denied");
    fs::create_dir_all(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let root = parent.join("IsctCompanion");
    let url = Url::parse("https://lms.s.isct.ac.jp/2026/pluginfile.php/1/a.pdf").unwrap();
    let refused = material_path(&root, &course(), &url);
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    let Err(MaterialError::Denied(message)) = refused else {
        panic!("a refused course folder must be reported as denied: {refused:?}");
    };
    assert!(message.contains("choose another course folder"));
    assert!(material_path(&root, &course(), &url).is_ok());
}

/// Opening, revealing, copying and sharing a stored file all pass a path
/// from the page through this; only a file below the root gets through.
#[test]
fn stored_file_accepts_only_a_file_under_the_course_folder() {
    let root = root("stored-scope");
    let folder = root.join("2026-3Q").join("CSC.T251");
    fs::create_dir_all(&folder).unwrap();
    let handout = folder.join("handout.pdf");
    fs::write(&handout, "safe").unwrap();
    let outside = root.with_extension("outside");
    fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.txt");
    fs::write(&secret, "secret").unwrap();
    let text = |path: &Path| path.to_string_lossy().into_owned();

    assert_eq!(
        stored_file(&root, &text(&handout)).unwrap(),
        handout.canonicalize().unwrap()
    );
    for refused in [
        text(&secret),
        text(
            &folder
                .join("..")
                .join("..")
                .join("..")
                .join(secret.file_name().unwrap()),
        ),
        text(
            &root
                .join("..")
                .join(outside.file_name().unwrap())
                .join("secret.txt"),
        ),
        text(&root),
        text(&folder),
        text(&folder.join("missing.pdf")),
        "2026-3Q/CSC.T251/handout.pdf".into(),
    ] {
        assert!(
            stored_file(&root, &refused).is_err(),
            "{refused} must be refused"
        );
    }

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[cfg(unix)]
#[test]
fn stored_file_accepts_an_alias_for_the_same_root_without_following_children() {
    use std::os::unix::fs::symlink;

    let actual = root("stored-alias");
    fs::create_dir_all(&actual).unwrap();
    let alias = actual.with_extension("alias");
    symlink(&actual, &alias).unwrap();
    let file = alias.join("handout.txt");
    fs::write(&file, "safe").unwrap();
    assert_eq!(
        stored_file(&alias, &file.to_string_lossy()).unwrap(),
        actual.join("handout.txt").canonicalize().unwrap()
    );

    let outside = actual.with_extension("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "secret").unwrap();
    symlink(&outside, actual.join("linked")).unwrap();
    assert!(stored_file(&alias, &alias.join("linked/secret.txt").to_string_lossy()).is_err());

    fs::remove_file(alias).unwrap();
    fs::remove_dir_all(actual).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn binary_preview_is_optional_and_text_preview_is_bounded() {
    let root = root("preview");
    fs::create_dir_all(&root).unwrap();
    let binary = root.join("image.png");
    fs::write(&binary, [0x89, b'P', b'N', b'G', 0xff]).unwrap();
    assert_eq!(
        describe(binary, "url".into(), String::new(), true, true)
            .unwrap()
            .text,
        None
    );
    let large = root.join("large.txt");
    fs::write(
        &large,
        "x".repeat(crate::file_preview::MAX_TEXT_BYTES as usize + 1),
    )
    .unwrap();
    assert_eq!(
        describe(large, "url".into(), String::new(), true, true)
            .unwrap()
            .text,
        None
    );
}

#[tokio::test]
async fn cleanup_removes_only_stale_complete_material_cache_files() {
    let root = root("cleanup");
    let course = root.join("material/2026-3Q/CSC.T251");
    let current = course.join("materials");
    let legacy = course.join("section-1/materials");
    let notes = course.join("section-1/notes");
    let workspace = root.join("workspace/2026-3Q/CSC.T251/Report 3/materials");
    let nested_notes_materials = course.join("notes/materials");
    let nested_workspace_materials = course.join("workspace/materials");
    for directory in [
        &current,
        &legacy,
        &notes,
        &workspace,
        &nested_notes_materials,
        &nested_workspace_materials,
    ] {
        fs::create_dir_all(directory).unwrap();
    }
    let stale = [
        current.join("one-1234abcd.pdf"),
        legacy.join("two-abcdef12"),
    ];
    for path in &stale {
        fs::write(path, "cache").unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
    }
    let part = current.join("active-1234abcd.pdf.part");
    let note = notes.join("note.md");
    let workspace_file = workspace.join("user-1234abcd.pdf");
    let nested_note = nested_notes_materials.join("user-1234abcd.pdf");
    let nested_workspace = nested_workspace_materials.join("user-abcdef12.pdf");
    for path in [
        &part,
        &note,
        &workspace_file,
        &nested_note,
        &nested_workspace,
    ] {
        fs::write(path, "keep").unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
    }
    let service = CourseMaterials::new(store("cleanup"), root.clone(), Downloader::default());
    assert_eq!(service.clean(1).await.unwrap(), 2);
    assert!(stale.iter().all(|path| !path.exists()));
    for kept in [part, note, workspace_file, nested_note, nested_workspace] {
        assert!(kept.exists(), "{}", kept.display());
    }
}

#[tokio::test]
async fn cleanup_reclaims_stale_partials_of_other_processes_only() {
    let root = root("partials");
    let materials = root.join("material/2026-3Q/CSC.T251/materials");
    fs::create_dir_all(&materials).unwrap();
    let other = std::process::id().wrapping_add(1);
    let age = |path: &Path, modified: SystemTime| {
        fs::write(path, "partial").unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    };
    let orphaned = materials.join(format!("lecture-1234abcd.mp4.part-{other}-3"));
    let recent = materials.join(format!("slides-1234abcd.pdf.part-{other}-4"));
    let own = materials.join(format!("notes-1234abcd.pdf.part-{}-5", std::process::id()));
    age(
        &orphaned,
        SystemTime::now() - STALE_PARTIAL_AGE - Duration::from_secs(60),
    );
    age(&recent, SystemTime::now());
    age(&own, SystemTime::UNIX_EPOCH);
    let service = CourseMaterials::new(store("partials"), root.clone(), Downloader::default());
    // A long retention for cached files does not keep orphaned partials.
    assert_eq!(service.clean(MAX_RETENTION_DAYS).await.unwrap(), 1);
    assert!(!orphaned.exists());
    assert!(recent.exists());
    assert!(own.exists());
    assert_eq!(
        crate::downloads::partial_download_owner("a.pdf.part-42-7"),
        Some(42)
    );
    assert_eq!(crate::downloads::partial_download_owner("a.pdf.part"), None);
    assert_eq!(crate::downloads::partial_download_owner("a.part-x-1"), None);
}
