use super::*;

#[test]
fn names_are_plain_and_bounded() {
    let pdf = "application/pdf";
    assert_eq!(safe_name("LAIIAssignment1.pdf", pdf), "LAIIAssignment1.pdf");
    assert_eq!(safe_name("../../etc/passwd", pdf), "_.._etc_passwd.pdf");
    assert_eq!(safe_name("..hidden.pdf", pdf), "hidden.pdf");
    assert_eq!(safe_name("a\u{0}b\nc.pdf", pdf), "abc.pdf");
    assert_eq!(safe_name("C:\\x\\講義.pdf", pdf), "C__x_講義.pdf");
    assert_eq!(safe_name("Slides", pdf), "Slides.pdf");
    assert_eq!(safe_name("Slides.PDF", pdf), "Slides.PDF");
    assert_eq!(safe_name(" .. ", pdf), "download.pdf");
    assert_eq!(safe_name("", "application/zip"), "download");
    assert_eq!(safe_name("notes.txt", "text/plain"), "notes.txt");
    assert_eq!(safe_name("期末 2019.md", ""), "期末 2019.md");
    #[cfg(windows)]
    assert_eq!(safe_name("CON.pdf", pdf), "_CON.pdf");
    let long = safe_name(&format!("{}.pdf", "講".repeat(400)), pdf);
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
