#[test]
fn offering_locators_resolve_to_the_canonical_detail_path() {
    let url = super::offering_url("202608712").unwrap();
    assert_eq!(
        url.as_str(),
        "https://syllabus.s.isct.ac.jp/courses/-/-/-/202608712"
    );
    // Setting the language must keep the id.
    assert!(super::localized_detail_url(&url, "en")
        .as_str()
        .ends_with("/202608712?hl=en"));
    assert!(matches!(
        super::offering_url("not a URL"),
        Err(super::SyllabusError::InvalidInput(_))
    ));
}
