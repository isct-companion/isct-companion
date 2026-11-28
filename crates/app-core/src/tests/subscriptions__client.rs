use super::*;
use crate::subscriptions::tests::{
    FIXTURE_ACADEMIC_CALENDAR_2026, FIXTURE_DEPARTMENTS_2025, FIXTURE_DEPARTMENTS_2026,
    FIXTURE_INDEX,
};

#[test]
fn published_sources_verify_against_their_manifest_hash_and_an_edited_source_does_not() {
    let index: SubscriptionIndex = serde_json::from_str(FIXTURE_INDEX).unwrap();
    let calendar = &index.academic_years["2026"].academic_calendar;
    let cohort_2025 = &index.admission_years["2025"].departments;
    let cohort_2026 = &index.admission_years["2026"].departments;
    // The published files are pretty-printed while the manifest hashes their
    // content, so these pass only if Core hashes content the way Isct-Data does.
    for (raw, source) in [
        (FIXTURE_ACADEMIC_CALENDAR_2026, calendar),
        (FIXTURE_DEPARTMENTS_2025, cohort_2025),
        (FIXTURE_DEPARTMENTS_2026, cohort_2026),
    ] {
        validate_indexed_source(source).unwrap();
        verify_source_hash(raw, &source.hash).unwrap();
    }

    let mut edited: Value = serde_json::from_str(FIXTURE_DEPARTMENTS_2026).unwrap();
    edited["admissionYear"] = 2025.into();
    assert!(verify_source_hash(&edited.to_string(), &cohort_2026.hash).is_err());
    assert!(verify_source_hash(FIXTURE_DEPARTMENTS_2025, &cohort_2026.hash).is_err());
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
