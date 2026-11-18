use super::*;

#[test]
fn saved_files_keep_their_contents_and_extension_without_overwriting() {
    let directory = std::env::temp_dir().join(format!("isct-downloads-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let name = safe_name(&format!("{}.pdf", "講".repeat(255)), "");
    fs::write(directory.join(&name), b"keep").unwrap();
    let source = directory.join("source.pdf");
    fs::write(&source, b"new").unwrap();

    let saved = write_unique(&directory, &name, fs::File::open(source).unwrap()).unwrap();

    assert_ne!(saved, directory.join(&name));
    assert_eq!(saved.parent(), Some(directory.as_path()));
    assert_eq!(saved.extension().unwrap(), "pdf");
    assert_eq!(fs::read(directory.join(name)).unwrap(), b"keep");
    assert_eq!(fs::read(saved).unwrap(), b"new");
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn a_failed_save_leaves_no_partial_download() {
    struct FailedRead;
    impl Read for FailedRead {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("source read failed"))
        }
    }
    let directory = std::env::temp_dir().join(format!("isct-downloads-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("notes.md"), b"keep").unwrap();
    let source = (&b"partial"[..]).chain(FailedRead);
    assert!(write_unique(&directory, "notes.md", source).is_err());
    assert_eq!(fs::read(directory.join("notes.md")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    fs::remove_dir_all(directory).unwrap();
}
