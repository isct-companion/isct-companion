use super::course_identity;
use url::Url;

#[test]
fn identity_comes_from_the_path_or_the_id_prefix() {
    let full =
        Url::parse("https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602421")
            .unwrap();
    assert_eq!(course_identity(&full), Some((2026, "202602421".into())));
    let short = Url::parse("https://syllabus.s.isct.ac.jp/courses/-/-/-/202608712").unwrap();
    assert_eq!(course_identity(&short), Some((2026, "202608712".into())));
    let bad = Url::parse("https://syllabus.s.isct.ac.jp/courses/-/-/-/abc").unwrap();
    assert_eq!(course_identity(&bad), None);
}
