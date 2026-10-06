mod api;
mod model;
mod parser;
mod transport;

pub use api::{
    department_catalog, detail, search, search_bilingual, sync_department, sync_offering,
    sync_offering_summary, SyllabusCourse, SyllabusFilters, SyllabusMeeting, SyllabusSearch,
};
pub use model::{
    CatalogSearchItem, CatalogSearchResult, ClassFormat, Credits, DeliveryKind,
    DepartmentSyncResult, FormatKind, Localized, Offering, OfferingContent, ScheduleEntry,
    ScheduleKind, SyllabusBlock, SyllabusCompetency, SyllabusContent, SyllabusLecture,
    SyllabusLink, SyllabusMetadata, SyllabusSection, SyncOfferingResult,
};

use api::{
    localized_search_url, parse_content, parse_detail_html, parse_search_html, quarters,
    search_url, utc_now,
};
use transport::{MAX_BODY, ORIGIN};
use url::Url;

#[cfg(test)]
mod language_tests {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_structured_search_transport() {
        let filters = SyllabusFilters {
            teacher: Some("Kobayashi".into()),
            code: Some("CSC.T273".into()),
            groups: Some(vec!["200".into(), "300".into()]),
            department: Some("4/0-904-342300-0-0".into()),
            ..Default::default()
        };
        let url = search_url("", 2026, &filters).unwrap();
        assert_eq!(url.path(), "/courses/2026/4/0-904-342300-0-0/search");
        assert_eq!(
            url.query_pairs()
                .filter(|(k, _)| k == "group")
                .map(|(_, v)| v.to_string())
                .collect::<Vec<_>>(),
            vec!["200", "300"]
        );
        assert!(url
            .query_pairs()
            .any(|(k, v)| k == "teacher" && v == "Kobayashi"));
        let captured = parse_search_html(
            include_str!("../../artifacts/syllabus-research/combined-search.html"),
            2026,
            100,
        )
        .unwrap();
        assert_eq!(captured.total, 1);
        assert_eq!(captured.items[0].code, "CSC.T273");
        assert!(!captured.truncated);
        let invalid = SyllabusFilters {
            department: Some("../search?x=1".into()),
            ..Default::default()
        };
        assert!(search_url("test", 2026, &invalid).is_err());
        let invalid = SyllabusFilters {
            groups: Some(vec!["999".into()]),
            ..Default::default()
        };
        assert!(search_url("test", 2026, &invalid).is_err());
        let global = search_url("a & b", 2025, &SyllabusFilters::default()).unwrap();
        assert_eq!(global.path(), "/search");
        assert!(global.query_pairs().any(|(k, v)| k == "q" && v == "a & b"));
    }
    #[test]
    fn parses_real_multi_course_examples() {
        let fixtures = [
            (
                "social",
                "7/0-907-0-110100-0/202600907",
                include_str!("../../artifacts/syllabus-research/social-en-content.html"),
            ),
            (
                "parallel",
                "4/0-904-342300-0-0/202602440",
                include_str!("../../artifacts/syllabus-research/parallel-en-content.html"),
            ),
            (
                "procedural",
                "4/0-904-342300-0-0/202602417",
                include_str!("../../artifacts/syllabus-research/procedural-en-content.html"),
            ),
            (
                "logic",
                "4/1-22-442222-0-0/202604060",
                include_str!("../../artifacts/syllabus-research/logic-en-content.html"),
            ),
            (
                "assembly",
                "4/0-904-342300-0-0/202602427",
                include_str!("../../artifacts/syllabus-research/assembly-en-content.html"),
            ),
            (
                "programming-one",
                "4/0-904-342200-0-0/202602384",
                include_str!("../../artifacts/syllabus-research/programming-one-en-content.html"),
            ),
        ];
        let mut result = Vec::new();
        for (name, path, html) in fixtures {
            let url = Url::parse(&format!("{ORIGIN}/courses/2026/{path}?hl=en")).unwrap();
            let course = parse_detail_html(&url, html).unwrap();
            assert!(course
                .content
                .as_ref()
                .is_some_and(|c| !c.sections.is_empty()));
            if ["logic", "assembly", "programming-one"].contains(&name) {
                assert_eq!(course.quarter, "3Q");
            }
            result.push(course);
        }
        if let Ok(path) = std::env::var("ISCT_COMPANION_PARSED_EXAMPLES_OUTPUT") {
            std::fs::write(path, serde_json::to_string_pretty(&result).unwrap()).unwrap();
        }
    }
    #[test]
    fn parses_verified_search_fixture() {
        let result = parse_search_html(
            include_str!("../../artifacts/syllabus-research/search-excerpt.html"),
            2026,
            12,
        )
        .unwrap();
        assert_eq!(result.total, 113);
        assert!(result.truncated);
        let course = result
            .items
            .iter()
            .find(|item| item.code == "CSC.T354")
            .unwrap();
        assert_eq!(course.external_id, "202602440");
        assert_eq!(course.quarter, "2Q");
        assert!(course
            .url
            .starts_with("https://syllabus.s.isct.ac.jp/courses/2026/"));
    }
    #[test]
    fn parses_verified_detail_fixture() {
        let url = Url::parse(
            "https://syllabus.s.isct.ac.jp/courses/2026/4/0-904-342300-0-0/202602440?hl=en",
        )
        .unwrap();
        let course = parse_detail_html(
            &url,
            include_str!("../../artifacts/syllabus-research/parallel-en-content.html"),
        )
        .unwrap();
        assert_eq!(course.credits, Some(2.0));
        assert_eq!(course.meetings.len(), 2);
        assert_eq!(course.meetings[0].days, vec![2]);
        assert_eq!(course.meetings[1].days, vec![5]);
        assert_eq!(course.meetings[0].periods, vec![3, 4]);
        assert_eq!(course.meetings[0].room, "W9-323");
    }
    #[test]
    fn preserves_multi_quarter_offerings() {
        assert_eq!(quarters("2026 1-2Q / 3-4Q"), vec!["1Q", "2Q", "3Q", "4Q"]);
    }
    #[test]
    fn parses_complete_bilingual_content_without_fabricating_resources() {
        for (language, html) in [
            (
                "en",
                include_str!("../../artifacts/syllabus-research/social-en-content.html"),
            ),
            (
                "ja",
                include_str!("../../artifacts/syllabus-research/social-ja-content.html"),
            ),
        ] {
            let url = Url::parse(&format!(
                "{ORIGIN}/courses/2026/7/0-907-0-110100-0/202600907?hl={language}"
            ))
            .unwrap();
            let result = parse_detail_html(&url, html).unwrap();
            assert_eq!(result.credits, Some(2.0));
            assert!(result.updated.is_some());
            if language == "en" {
                if let Ok(path) = std::env::var("ISCT_COMPANION_PARSED_SYLLABUS_OUTPUT") {
                    std::fs::write(path, serde_json::to_string_pretty(&result).unwrap()).unwrap();
                }
            }
            let content = result.content.unwrap();
            assert_eq!(content.source_language, language);
            assert_eq!(content.lecture_plan.len(), 14);
            assert!(content.sections.iter().any(|s| s.key == "contact"));
            assert!(content.sections.iter().any(|s| s.key == "officeHours"));
            assert!(content
                .sections
                .iter()
                .any(|s| s.key == "practicalExperience"));
            assert!(content.sections.iter().all(|s| s.links.is_empty()));
            assert!(content
                .sections
                .iter()
                .find(|s| s.key == "references")
                .unwrap()
                .blocks[0]
                .text
                .contains("ISBN"));
            assert_eq!(content.fetched_at.len(), 20);
            assert!(content.fetched_at.ends_with('Z'));
        }
    }
    #[test]
    fn preserves_checked_competencies_and_absent_optional_sections() {
        let url = Url::parse(&format!(
            "{ORIGIN}/courses/2026/4/0-904-342300-0-0/202602440?hl=en"
        ))
        .unwrap();
        let result = parse_detail_html(
            &url,
            include_str!("../../artifacts/syllabus-research/parallel-en-content.html"),
        )
        .unwrap();
        let content = result.content.unwrap();
        assert_eq!(content.lecture_plan.len(), 14);
        assert_eq!(content.competencies.len(), 5);
        assert_eq!(
            content.competencies.iter().filter(|c| c.selected).count(),
            2
        );
        assert!(!content.sections.iter().any(|s| s.key == "contact"));
        assert!(content.lecture_plan[0].topic.contains("Introduction"));
        assert!(content
            .sections
            .iter()
            .find(|s| s.key == "references")
            .unwrap()
            .blocks[0]
            .text
            .contains("\n"));
    }
    #[test]
    fn rejects_incomplete_content_instead_of_replacing_good_cache() {
        let url = Url::parse(&format!(
            "{ORIGIN}/courses/2026/4/0-904-342300-0-0/202602440?hl=en"
        ))
        .unwrap();
        assert!(parse_detail_html(
            &url,
            include_str!("../../artifacts/syllabus-research/detail-en-excerpt.html")
        )
        .is_err());
    }
    #[test]
    fn emits_plain_text_and_only_actual_safe_links() {
        let url = Url::parse(&format!("{ORIGIN}/courses/2026/4/a/1?hl=en")).unwrap();
        let html = r#"<h1>Example</h1><h3>Course overview and goals</h3><p>A &#x26; B<br>next<script>evil()</script></p><h3>Course description and aims</h3><p><a href="javascript:evil()">bad</a><a href="https://example.org/book">Book</a></p>"#;
        let content = parse_content(&url, html, &[], None).unwrap();
        assert_eq!(content.sections[0].blocks[0].text, "A & B\nnext");
        assert_eq!(content.sections[1].links.len(), 1);
        assert_eq!(content.sections[1].links[0].url, "https://example.org/book");
    }
    #[test]
    fn parses_optional_live_capture() {
        let Ok(path) = std::env::var("ISCT_COMPANION_LIVE_SYLLABUS_HTML") else {
            return;
        };
        let html = std::fs::read_to_string(path).unwrap();
        let result = parse_search_html(&html, 2026, 12).unwrap();
        assert!(!result.items.is_empty());
        assert!(result.total >= result.items.len());
    }
}
