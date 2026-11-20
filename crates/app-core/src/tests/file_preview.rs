use super::*;

#[test]
fn a_file_is_shown_by_its_kind_and_this_platform() {
    let shown = |name: &str| viewer(name, None, 10, b"");
    assert_eq!(shown("Lecture 3.PDF"), Viewer::Pdf);
    assert_eq!(shown("notes.md"), Viewer::Markdown);
    assert_eq!(shown("Main.java"), Viewer::Text);
    assert_eq!(shown("board.HEIC"), Viewer::Image);
    // Android's system player plays media; the others play it in the app.
    let (video, audio) = if cfg!(target_os = "android") {
        (Viewer::External, Viewer::External)
    } else {
        (Viewer::Video, Viewer::Audio)
    };
    assert_eq!(shown("lecture.mov"), video);
    assert_eq!(shown("talk.m4a"), audio);
    // Office documents preview in the app only where the system has a viewer.
    let office = if cfg!(target_os = "ios") {
        Viewer::Document
    } else {
        Viewer::External
    };
    for name in ["report.docx", "slides.pptx", "grades.xlsx", "deck.key"] {
        assert_eq!(shown(name), office, "{name}");
    }
    // A notebook or web page is not read as its source; a ZIP Core did not
    // list and any other archive open whole.
    for name in ["lab.ipynb", "index.html", "problem.zip", "data.tar.gz"] {
        assert_eq!(shown(name), Viewer::External, "{name}");
    }
}

#[test]
fn a_file_of_no_known_kind_is_text_only_when_it_reads_as_text() {
    assert_eq!(viewer("Makefile", None, 4, b"all:"), Viewer::Text);
    // A character cut off where sniffing stops is still text.
    let cut = &"講義".as_bytes()[..4];
    assert_eq!(viewer("README", None, 6, cut), Viewer::Text);
    assert_eq!(viewer("a.out", None, 4, b"\x7fELF\0"), Viewer::External);
    assert_eq!(
        viewer("blob", None, 3, &[0xff, 0xfe, 0x41]),
        Viewer::External
    );
    assert_eq!(viewer("empty", None, 0, b""), Viewer::External);
}

#[test]
fn a_validated_type_wins_over_the_uploaders_name_except_markdown_stored_as_text() {
    assert_eq!(
        viewer("answers.bat", Some("text/plain"), 9, b"x"),
        Viewer::Text
    );
    assert_eq!(viewer("scan", Some("image/jpeg"), 9, b""), Viewer::Image);
    assert_eq!(
        viewer("exam.png", Some("application/pdf"), 9, b""),
        Viewer::Pdf
    );
    // The exam site stores Markdown as text/plain.
    assert_eq!(
        viewer("notes.markdown", Some("text/plain; charset=utf-8"), 9, b"#"),
        Viewer::Markdown
    );
}

#[test]
fn text_too_large_to_read_in_the_app_says_so() {
    assert_eq!(
        viewer("big.md", None, MAX_TEXT_BYTES, b"#"),
        Viewer::Markdown
    );
    assert_eq!(
        viewer("big.md", None, MAX_TEXT_BYTES + 1, b"#"),
        Viewer::TooLarge
    );
    assert_eq!(
        viewer("log", None, MAX_TEXT_BYTES + 1, b"ok"),
        Viewer::TooLarge
    );
    // Size limits only text, which the shells read whole.
    assert_eq!(
        viewer("big.pdf", None, MAX_TEXT_BYTES * 50, b""),
        Viewer::Pdf
    );
}

#[test]
fn the_catalog_lists_every_kind_once_with_the_viewer_kind_of_reads() {
    let catalog = catalog();
    let mut seen = std::collections::HashSet::new();
    for entry in &catalog {
        assert!(seen.insert(entry.kind), "{:?} listed twice", entry.kind);
        for extension in &entry.extensions {
            assert_eq!(kind_of(&format!("x.{extension}")), Some(entry.kind));
        }
        assert_eq!(entry.viewer, viewer_of_kind(entry.kind));
    }
}

#[test]
fn a_file_on_disk_is_sniffed_and_an_unreadable_one_opens_elsewhere() {
    let dir = std::env::temp_dir().join(format!("core-preview-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("0f7a3c2e-cached");
    std::fs::write(&file, "# Heading\n").unwrap();
    // A cached past exam has a generated name; the upload's name decides.
    assert_eq!(
        viewer_of_file(&file, "notes.md", Some("text/plain")),
        Viewer::Markdown
    );
    assert_eq!(viewer_of_file(&file, "0f7a3c2e-cached", None), Viewer::Text);
    assert_eq!(
        viewer_of_file(&dir.join("gone.md"), "gone.md", None),
        Viewer::External
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
