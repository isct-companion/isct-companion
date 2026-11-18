use super::*;

#[test]
fn saved_files_keep_their_contents_and_extension_without_overwriting() {
    let directory = std::env::temp_dir().join(format!("isct-downloads-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let names = [
        "Report.final.pdf".to_owned(),
        "README".to_owned(),
        "../../etc/passwd".to_owned(),
        "..hidden.pdf".to_owned(),
        "a\u{0}b\nc.pdf".to_owned(),
        "C:\\x\\講義.pdf".to_owned(),
        "Slides.PDF".to_owned(),
        " .. ".to_owned(),
        "".to_owned(),
        "notes.txt".to_owned(),
        "期末 2019.md".to_owned(),
        "a/b\\c:d?.txt".to_owned(),
        "../講義\n/資料.pdf".to_owned(),
        format!("{}.pdf", "a".repeat(251)),
        format!("{}.pdf", "講".repeat(255)),
        "CON.pdf".to_owned(),
    ];
    for suggested in names {
        let name = download_file_name(&suggested, "");
        let first = write_unique(&directory, &name, &b"old"[..]).unwrap();
        let second = write_unique(&directory, &name, fs::File::open(&first).unwrap()).unwrap();
        assert_ne!(first, second);
        let saved_name = second.file_name().unwrap().to_str().unwrap();
        assert!(!saved_name.starts_with('.'));
        assert!(!saved_name.chars().any(char::is_control));
        assert_eq!(first.parent(), Some(directory.as_path()));
        assert_eq!(second.parent(), Some(directory.as_path()));
        assert_eq!(first.extension(), second.extension());
        if suggested.to_ascii_lowercase().ends_with(".pdf") {
            assert_eq!(
                second
                    .extension()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_ascii_lowercase(),
                "pdf"
            );
        }
        assert_eq!(fs::read(first).unwrap(), b"old");
        assert_eq!(fs::read(second).unwrap(), b"old");
    }
    let name = download_file_name("Slides", "application/pdf");
    let pdf = write_unique(&directory, &name, &b"pdf"[..]).unwrap();
    assert_eq!(pdf.extension().unwrap(), "pdf");
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
