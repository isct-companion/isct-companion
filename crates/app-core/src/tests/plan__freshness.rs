use super::*;

fn now() -> DateTime<Utc> {
    "2026-10-20T03:00:00Z".parse().unwrap()
}

/// One function says whether a key is current or over (`d/wtgqyz`): a past
/// academic year's catalog and a month that is over are frozen, this year's
/// are current. (A course the stored enrolment list dropped is not read at
/// all: `plan::tests::a_course_page_opened_before_the_enrolment_list_waits_for_it`.)
#[test]
fn every_key_is_current_or_over_by_one_rule() {
    let catalog = |key: &str| Key::new(Dataset::CatalogOfficial, key);
    // 2026-10-20 is in academic year 2026 (April to March).
    assert_eq!(
        term(&catalog("2025/LA"), None, false, now()),
        Some(Term::Frozen)
    );
    assert_eq!(
        term(&catalog("2025/LA/en"), None, false, now()),
        Some(Term::Frozen)
    );
    assert_eq!(
        term(&catalog("2026/LA"), None, false, now()),
        Some(Term::Current)
    );
    let march: DateTime<Utc> = "2027-03-10T03:00:00Z".parse().unwrap();
    assert_eq!(
        term(&catalog("2026/LA"), None, false, march),
        Some(Term::Current)
    );
    let history = |month: &str| Key::new(Dataset::LmsHistory, month);
    assert_eq!(
        term(&history("2026-09"), None, false, now()),
        Some(Term::Frozen)
    );
    assert_eq!(
        term(&history("2026-10"), None, false, now()),
        Some(Term::Current)
    );
}
