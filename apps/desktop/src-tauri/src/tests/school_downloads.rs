use super::*;

#[test]
fn downloads_are_limited_to_the_visible_school_view() {
    let lms = SchoolSite::ScienceLms;
    let file = "https://lms.s.isct.ac.jp/2025/pluginfile.php/224823/mod_resource/content/3/LAIIAssignment1.pdf";
    assert!(download_allowed(file, lms, Some(lms)));
    assert!(!download_allowed(file, lms, None));
    assert!(!download_allowed(file, lms, Some(SchoolSite::Kyomu)));
    for url in [
        "https://example.com/file.pdf",
        "http://lms.s.isct.ac.jp/2025/pluginfile.php/1/file.pdf",
        "https://user:secret@lms.s.isct.ac.jp/file.pdf",
        "blob:https://lms.s.isct.ac.jp/uuid",
        "data:application/pdf;base64,AAAA",
    ] {
        assert!(!download_allowed(url, lms, Some(lms)), "{url}");
    }
}

#[test]
fn download_destinations_stay_directly_in_the_download_folder() {
    let downloads = Path::new("/Users/student/Downloads");
    assert!(download_destination_ok(
        &downloads.join("Lecture 1 (2).pptx"),
        downloads
    ));
    for bad in [
        "/Users/student/Downloads/../.zshrc",
        "/Users/student/Downloads/nested/file.docx",
        "/Users/student/Desktop/file.docx",
        "/Users/student/Downloads/line\nbreak.pdf",
        "/Users/student/Downloads/back\\slash.pdf",
        "/Users/student/Downloads",
    ] {
        assert!(!download_destination_ok(Path::new(bad), downloads));
    }
}

#[test]
fn viewer_file_names_are_plain_and_bounded() {
    let pdf = "application/pdf";
    assert_eq!(
        download_file_name("LAIIAssignment1.pdf", pdf),
        "LAIIAssignment1.pdf"
    );
    assert_eq!(
        download_file_name("../../etc/passwd", pdf),
        "_.._etc_passwd.pdf"
    );
    assert_eq!(download_file_name("..hidden.pdf", pdf), "hidden.pdf");
    assert_eq!(download_file_name("a\u{0}b\nc.pdf", pdf), "abc.pdf");
    assert_eq!(download_file_name("C:\\x\\講義.pdf", pdf), "C__x_講義.pdf");
    assert_eq!(download_file_name("Slides", pdf), "Slides.pdf");
    assert_eq!(download_file_name("Slides.PDF", pdf), "Slides.PDF");
    assert_eq!(download_file_name(" .. ", pdf), "download.pdf");
    assert_eq!(download_file_name("", "application/zip"), "download");
    assert_eq!(download_file_name("notes.txt", "text/plain"), "notes.txt");
    let long = download_file_name(&format!("{}.pdf", "講".repeat(400)), pdf);
    assert!(long.len() <= DOWNLOAD_NAME_LIMIT, "{}", long.len());
    assert!(long.ends_with("講.pdf"));
    assert!(safe_file_name(&long));
}

#[test]
fn unique_names_number_before_the_extension() {
    assert_eq!(
        download_candidates("Report.final.pdf")
            .take(3)
            .collect::<Vec<_>>(),
        [
            "Report.final.pdf",
            "Report.final (1).pdf",
            "Report.final (2).pdf"
        ]
    );
    assert_eq!(
        download_candidates("README").nth(1).as_deref(),
        Some("README (1)")
    );
}

#[test]
fn viewer_saves_never_overwrite_existing_files() {
    let directory = std::env::temp_dir().join(format!(
        "isct-companion-download-test-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("a.pdf"), b"old").unwrap();
    let first = write_unique(&directory, "a.pdf", b"new").unwrap();
    let second = write_unique(&directory, "a.pdf", b"newer").unwrap();
    assert_eq!(first, directory.join("a (1).pdf"));
    assert_eq!(second, directory.join("a (2).pdf"));
    assert_eq!(fs::read(directory.join("a.pdf")).unwrap(), b"old");
    assert_eq!(fs::read(&second).unwrap(), b"newer");
    fs::remove_dir_all(&directory).unwrap();
}
