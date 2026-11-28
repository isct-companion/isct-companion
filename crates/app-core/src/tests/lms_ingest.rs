//! The course and enrolment commits are guarded over captured REST answers
//! by the capture path test in `net__moodle__mapping.rs`.
use super::*;
use chrono::TimeZone;

#[test]
fn course_terms_follow_the_grid_and_the_month_in_japan() {
    let now = Utc.with_ymd_and_hms(2026, 10, 20, 3, 0, 0).unwrap();
    let course = |id: u64, year: i32, quarter: u8, shown: &[&str]| StoredLmsCourse {
        id,
        title: String::new(),
        year: Some(year),
        quarter: Some(quarter),
        shown_in: shown.iter().map(|term| term.to_string()).collect(),
    };
    // 3Q now; course 2 started in 2Q but the current grid lists it.
    let courses = [
        course(1, 2026, 3, &[]),
        course(2, 2026, 2, &["2026-3Q"]),
        course(3, 2026, 1, &[]),
        course(4, 2025, 3, &[]),
    ];
    let current = current_lms_term(now);
    assert_eq!(current, (2026, 3));
    let terms: Vec<Term> = courses
        .iter()
        .map(|course| lms_course_term(course, current))
        .collect();
    assert_eq!(
        terms,
        [Term::Current, Term::Current, Term::Past, Term::Frozen]
    );
}
