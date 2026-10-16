use super::*;
use crate::storage::stored_file;
use serde_json::json;
use uuid::Uuid;

fn store(name: &str) -> DataStore {
    let path = std::env::temp_dir().join(format!("course-materials-{name}-{}.db", Uuid::new_v4()));
    DataStore::initialize(path).unwrap().0
}

fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("course-materials-{name}-{}", Uuid::new_v4()))
}

fn course() -> CourseKey {
    CourseKey {
        term: "2026-3Q".into(),
        course: "CSC.T251".into(),
    }
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
        Ok((_, LmsResourceKind::View { cmid: 42, .. }))
    ));
    assert!(!safe_external_url("https://user:secret@example.test/file"));
}

#[tokio::test]
async fn cached_hit_and_missing_do_not_require_a_download_session() {
    let root = root("cache");
    let service = CourseMaterials::new(store("cache"), root.clone(), Downloader::default(), None);
    let source = "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/readme.txt";
    let url = Url::parse(source).unwrap();
    let path = material_path(&root, &course(), &url).unwrap();
    fs::write(&path, "cached text").unwrap();

    let material = service
        .open(course(), source.into(), true, true, Some(1))
        .await
        .unwrap();
    let Material::Ready { file } = material else {
        panic!("cached material should be ready");
    };
    assert!(file.cached);
    assert_eq!(file.text.as_deref(), Some("cached text"));

    let missing = service
        .open(
            course(),
            "https://lms.s.isct.ac.jp/2026/pluginfile.php/2/missing.pdf".into(),
            false,
            false,
            None,
        )
        .await
        .unwrap();
    assert_eq!(missing, Material::Missing);
}

#[tokio::test]
async fn unavailable_native_operations_keep_distinct_typed_errors() {
    let service = CourseMaterials::new(
        store("unsupported"),
        root("unsupported"),
        Downloader::default(),
        None,
    );
    assert!(matches!(
        service
            .open(
                course(),
                "https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=9".into(),
                false,
                false,
                None,
            )
            .await,
        Err(MaterialError::UnsupportedResolution)
    ));
    assert!(matches!(
        service
            .open(
                course(),
                "https://lms.s.isct.ac.jp/2026/pluginfile.php/9/a.pdf".into(),
                true,
                false,
                None,
            )
            .await,
        Err(MaterialError::UnsupportedDownload)
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
    for (error, reason) in [
        (FetchError::Login, MaterialFailure::Login),
        (FetchError::Invalid, MaterialFailure::Invalid),
        (FetchError::Network, MaterialFailure::Network),
        (FetchError::Timeout, MaterialFailure::Timeout),
        (FetchError::Cancelled, MaterialFailure::Cancelled),
    ] {
        assert_eq!(material_for_fetch_error(error), failed(reason));
    }
    assert_eq!(
        serde_json::to_value(failed(MaterialFailure::Login)).unwrap()["reason"],
        "login"
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
        "x".repeat(crate::storage::MAX_TEXT_PREVIEW_BYTES as usize + 1),
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
    let current = root.join("2026-3Q/CSC.T251/materials");
    let legacy = root.join("2026-3Q/CSC.T251/section-1/materials");
    let notes = root.join("2026-3Q/CSC.T251/section-1/notes");
    let workspace = root.join("workspaces/local/materials");
    let nested_notes_materials = root.join("2026-3Q/CSC.T251/notes/materials");
    let nested_workspace_materials = root.join("2026-3Q/CSC.T251/workspace/materials");
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
    let service = CourseMaterials::new(store("cleanup"), root.clone(), Downloader::default(), None);
    assert_eq!(service.clean(1).await.unwrap(), 2);
    assert!(stale.iter().all(|path| !path.exists()));
    assert!(part.exists());
    assert!(note.exists());
    assert!(workspace_file.exists());
    assert!(nested_note.exists());
    assert!(nested_workspace.exists());
}

#[tokio::test]
async fn cleanup_reclaims_stale_partials_of_other_processes_only() {
    let root = root("partials");
    let materials = root.join("2026-3Q/CSC.T251/materials");
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
    let service =
        CourseMaterials::new(store("partials"), root.clone(), Downloader::default(), None);
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
