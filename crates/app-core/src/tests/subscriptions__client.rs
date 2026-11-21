use super::*;
use crate::subscriptions::tests::{
    FIXTURE_ACADEMIC_CALENDAR_2026, FIXTURE_DEPARTMENTS_2025, FIXTURE_DEPARTMENTS_2026,
    FIXTURE_INDEX,
};

const FIXTURE_INDEX_HASH: &str = include_str!("../../tests/fixtures/subscriptions/index.sha256");

#[test]
fn frozen_fixture_manifest_hashes_match_every_source() {
    assert_eq!(
        data_encoding::HEXLOWER.encode(&Sha256::digest(FIXTURE_INDEX.as_bytes())),
        FIXTURE_INDEX_HASH.trim()
    );
    let index: SubscriptionIndex = serde_json::from_str(FIXTURE_INDEX).unwrap();
    let year = index.academic_years.get("2026").unwrap();
    let cohort_2025 = index.admission_years.get("2025").unwrap();
    let cohort_2026 = index.admission_years.get("2026").unwrap();
    let actual = [
        content_hash(&serde_json::from_str(FIXTURE_ACADEMIC_CALENDAR_2026).unwrap()).unwrap(),
        content_hash(&serde_json::from_str(FIXTURE_DEPARTMENTS_2025).unwrap()).unwrap(),
        content_hash(&serde_json::from_str(FIXTURE_DEPARTMENTS_2026).unwrap()).unwrap(),
    ];
    let expected = [
        year.academic_calendar.hash.as_str(),
        cohort_2025.departments.hash.as_str(),
        cohort_2026.departments.hash.as_str(),
    ];
    assert_eq!(
        actual.iter().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn exam_paths_select_their_own_feed_kind() {
    let exams = ExamFeedKind::Exams;
    let classes = ExamFeedKind::ExamWeekClasses;
    assert_eq!(
        exam_scope("exam-week-classes/graduate/2026-2q.json", 2026, classes),
        Some(("graduate".into(), "2q".into()))
    );
    assert_eq!(
        exam_scope("exam-week-classes/graduate/2026-2q.json", 2026, exams),
        None
    );
    assert_eq!(
        exam_scope("exam/graduate/2026-2q.json", 2026, classes),
        None
    );
}
