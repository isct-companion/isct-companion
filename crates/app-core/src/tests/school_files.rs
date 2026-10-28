use super::*;

fn named(disposition: Option<&str>, url: &str, mime: Option<&str>) -> String {
    file_name(
        disposition.map(str::as_bytes),
        &Url::parse(url).unwrap(),
        mime,
    )
}

// isct.ac.jp serves PDFs `inline; filename="…"` with the name in raw UTF-8;
// the WebView's own string is not trusted for those bytes, so Core reads the
// response header.
#[test]
fn a_disposition_names_the_file_whatever_its_type() {
    let page = "https://www.isct.ac.jp/component_download_file.php?id=1";
    let pdf = Some("application/pdf");
    assert_eq!(
        named(Some("inline; filename=\"環境報告書2026.pdf\""), page, pdf),
        "環境報告書2026.pdf"
    );
    // RFC 6266: `filename*` wins over the fallback in either order.
    for header in [
        "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''%E8%AC%9B%E7%BE%A9.pdf",
        "attachment; filename*=utf-8''%E8%AC%9B%E7%BE%A9.pdf; filename=\"fallback.pdf\"",
    ] {
        assert_eq!(named(Some(header), page, pdf), "講義.pdf", "{header}");
    }
    assert_eq!(
        named(
            Some("attachment; filename*=iso-8859-1'en'%E9t%E9.txt"),
            page,
            None
        ),
        "été.txt"
    );
    // Quoted strings keep escaped quotes and semicolons.
    assert_eq!(
        named(
            Some(r#"attachment; filename="a\"b;c.pdf"; size=3"#),
            page,
            pdf
        ),
        "a\"b;c.pdf"
    );
    assert_eq!(
        named(Some("attachment; filename=\"%E8%AC%9B.pdf\""), page, pdf),
        "講.pdf"
    );
    assert_eq!(
        named(Some("attachment; filename=plain.zip"), page, None),
        "plain.zip"
    );
    // Header bytes that are not UTF-8 are Latin-1.
    assert_eq!(
        file_name(
            Some(b"attachment; filename=\"caf\xe9.txt\""),
            &Url::parse(page).unwrap(),
            Some("text/plain"),
        ),
        "café.txt"
    );
    // A disposition without a usable name falls back to the URL.
    for header in [
        "inline",
        "attachment; filename=\"\"",
        "attachment; filename*=koi8-r''x.txt",
    ] {
        assert_eq!(
            named(Some(header), page, pdf),
            "component_download_file.pdf",
            "{header}"
        );
    }
}

#[test]
fn without_a_disposition_the_url_and_type_name_the_file() {
    let pdf = Some("application/pdf; charset=binary");
    assert_eq!(
        named(
            None,
            "https://kyomu0.gakumu.titech.ac.jp/Titech/Common/student_manual_j.pdf",
            pdf
        ),
        "student_manual_j.pdf"
    );
    assert_eq!(
        named(
            None,
            "https://www.isct.ac.jp/files/%E8%AC%9B%E7%BE%A9.PDF",
            pdf
        ),
        "講義.PDF"
    );
    assert_eq!(
        named(
            None,
            "https://www.isct.ac.jp/files/report",
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        ),
        "report.docx"
    );
    assert_eq!(
        named(
            None,
            "https://www.isct.ac.jp/photo.jpeg",
            Some("image/jpeg")
        ),
        "photo.jpeg"
    );
    assert_eq!(named(None, "https://www.isct.ac.jp/", pdf), "download.pdf");
    assert_eq!(
        named(None, "https://www.isct.ac.jp/data.bin", None),
        "data.bin"
    );
    assert_eq!(
        named(
            None,
            "https://www.isct.ac.jp/data.bin",
            Some("application/octet-stream")
        ),
        "data.bin"
    );
}

#[test]
fn names_are_plain_and_bounded() {
    let pdf = "application/pdf";
    assert_eq!(
        plain_name("LAIIAssignment1.pdf", pdf),
        "LAIIAssignment1.pdf"
    );
    assert_eq!(plain_name("../../etc/passwd", pdf), "_.._etc_passwd.pdf");
    assert_eq!(plain_name("..hidden.pdf", pdf), "hidden.pdf");
    assert_eq!(plain_name("a\u{0}b\nc.pdf", pdf), "abc.pdf");
    assert_eq!(plain_name("C:\\x\\講義.pdf", pdf), "C__x_講義.pdf");
    assert_eq!(plain_name("Slides", pdf), "Slides.pdf");
    assert_eq!(plain_name("Slides.PDF", pdf), "Slides.PDF");
    assert_eq!(plain_name(" .. ", pdf), "download.pdf");
    assert_eq!(plain_name("", "application/zip"), "download");
    assert_eq!(plain_name("notes.txt", "text/plain"), "notes.txt");
    let long = plain_name(&format!("{}.pdf", "講".repeat(400)), pdf);
    assert!(long.len() <= NAME_LIMIT, "{}", long.len());
    assert!(long.ends_with("講.pdf"));
}

#[test]
fn unique_names_number_before_the_extension() {
    assert_eq!(
        name_candidates("Report.final.pdf")
            .take(3)
            .collect::<Vec<_>>(),
        [
            "Report.final.pdf",
            "Report.final (1).pdf",
            "Report.final (2).pdf"
        ]
    );
    assert_eq!(
        name_candidates("README").nth(1).as_deref(),
        Some("README (1)")
    );
}
