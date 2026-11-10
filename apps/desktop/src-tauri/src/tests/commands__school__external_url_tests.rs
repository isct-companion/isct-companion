use super::external_url;

#[test]
fn opens_school_and_plain_web_links_only() {
    assert_eq!(
        external_url("https://syllabus.s.isct.ac.jp/courses?jwc=202602431").as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses?jwc=202602431")
    );
    assert_eq!(external_url("https://lms.s.isct.ac.jp/2025/search/index.php?q=linear%20algebra&areaids=core_course-course#results").as_deref(), Some("https://lms.s.isct.ac.jp/2025/search/index.php?q=linear%20algebra&areaids=core_course-course#results"));
    assert!(external_url("https://example.org/paper?id=1").is_some());
    assert!(external_url("file:///etc/passwd").is_none());
    assert!(external_url("javascript:alert(1)").is_none());
    assert!(external_url("https://user:pw@example.org/").is_none());
}
