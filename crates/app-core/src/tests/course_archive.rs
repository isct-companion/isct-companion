use super::*;
use std::{io::Write, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("core-archive-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn archive(path: &Path, files: &[(&str, &str)]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in files {
        if name.ends_with('/') {
            zip.add_directory(*name, options).unwrap();
            continue;
        }
        zip.start_file(*name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn lists_only_files_a_person_would_open_and_extracts_one_within_its_limit() {
    let fixture = Fixture::new("list");
    let zip = fixture.0.join("problem.zip");
    archive(
        &zip,
        &[
            ("problem/", ""),
            ("problem/Makefile", "all:"),
            ("problem/Main.java", "class Main {}"),
            ("__MACOSX/problem/._Main.java", "junk"),
            ("problem/.DS_Store", "junk"),
            ("../escape.txt", "outside"),
        ],
    );
    assert_eq!(
        list(&zip).unwrap(),
        [
            ArchiveEntry {
                name: "problem/Main.java".into(),
                bytes: 13
            },
            ArchiveEntry {
                name: "problem/Makefile".into(),
                bytes: 4
            },
        ]
    );

    let target = fixture.0.join("Main-0000aaaa.java");
    assert_eq!(
        extract(&zip, "problem/Main.java", &target, Some(12)).unwrap(),
        Extracted::TooLarge(13)
    );
    assert!(!target.exists());
    assert_eq!(
        extract(&zip, "problem/Main.java", &target, Some(13)).unwrap(),
        Extracted::Saved
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "class Main {}");
    // What the listing leaves out is never extracted.
    for name in ["../escape.txt", "problem/.DS_Store", "problem/", "gone.txt"] {
        assert_eq!(
            extract(&zip, name, &target, None).unwrap(),
            Extracted::Missing,
            "{name}"
        );
    }
    assert!(!fixture.0.join("escape.txt").exists());
}

#[test]
fn an_archive_too_big_to_show_as_a_folder_is_not_listed() {
    let fixture = Fixture::new("limits");
    let many = fixture.0.join("many.zip");
    let names: Vec<String> = (0..=MAX_LISTED_FILES)
        .map(|n| format!("f{n}.txt"))
        .collect();
    archive(
        &many,
        &names
            .iter()
            .map(|name| (name.as_str(), ""))
            .collect::<Vec<_>>(),
    );
    assert_eq!(list(&many), None);
    let garbage = fixture.0.join("garbage.zip");
    fs::write(&garbage, "not a zip").unwrap();
    assert_eq!(list(&garbage), None);
}

#[test]
fn entry_names_read_utf8_and_japanese_windows_archives() {
    assert_eq!(
        entry_name("講義/第1回.pdf".as_bytes(), Some("x")),
        "講義/第1回.pdf"
    );
    let (shift_jis, _, _) = encoding_rs::SHIFT_JIS.encode("講義\\第1回.pdf");
    assert_eq!(entry_name(&shift_jis, Some("x")), "講義/第1回.pdf");
    assert_eq!(
        entry_name(&[0xff, 0xfe, 0xfd], Some("fallback")),
        "fallback"
    );
}
