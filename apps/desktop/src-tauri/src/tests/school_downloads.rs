use super::*;

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
