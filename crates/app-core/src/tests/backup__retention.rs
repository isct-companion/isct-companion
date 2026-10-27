use super::*;

#[test]
fn cleanup_requires_both_age_and_four_strictly_newer_known_dates() {
    let now: DateTime<Utc> = "2026-10-17T00:00:00Z".parse().unwrap();
    let dates = [
        "2026-10-17T00:00:00Z",
        "2026-10-16T00:00:00Z",
        "2026-09-17T00:00:00Z",
        "2026-08-03T00:00:00Z",
        "2026-08-02T00:00:00Z",
        "invalid",
        "2027-01-01T00:00:00Z",
    ];
    assert_eq!(expired(dates, now), [4]); // exact 30 days and the newest four survive
    assert!(expired(dates[..4].iter().copied(), now).is_empty());
    let same_day = ["2026-08-01T00:00:00Z"; 8];
    assert!(expired(same_day, now).is_empty());
}
