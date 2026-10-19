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

#[test]
fn finder_summary_keeps_filter_metadata_without_claiming_bilingual_content() {
    let url = url::Url::parse(
        "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602440?hl=en",
    )
    .unwrap();
    let page = super::parser::parse_detail(
        &url,
        include_str!("../../tests/fixtures/syllabus/parallel-en-content.html"),
        "en",
    )
    .unwrap();
    let summary = super::single_detail(page).unwrap();
    assert_eq!(summary.offering.id, "202602440");
    assert_eq!(
        summary.offering.credits,
        super::Credits {
            lecture: 1.0,
            exercise: 1.0,
            lab: 0.0
        }
    );
    assert_eq!(summary.offering.schedule.len(), 2);
    assert!(summary.offering.fetched_at.ja.is_none());
    assert_eq!(summary.content.len(), 1);
    assert_eq!(summary.content[0].lang, "en");
}
