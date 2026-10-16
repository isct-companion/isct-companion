use super::api::localized_search_url;
use super::*;
#[test]
fn language_url_preserves_filters_and_validates_locale() {
    let filters = SyllabusFilters::default();
    let url = localized_search_url("parallel", 2026, &filters, "ja").unwrap();
    assert_eq!(
        url.query_pairs()
            .filter(|(k, _)| k == "hl")
            .collect::<Vec<_>>()
            .len(),
        1
    );
    assert_eq!(url.query_pairs().find(|(k, _)| k == "hl").unwrap().1, "ja");
    assert_eq!(
        url.query_pairs().find(|(k, _)| k == "q").unwrap().1,
        "parallel"
    );
    assert!(localized_search_url("parallel", 2026, &filters, "xx").is_err());
}
