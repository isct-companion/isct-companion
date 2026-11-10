use super::*;

fn at(date: &str) -> DateTime<Utc> {
    date.parse().unwrap()
}

#[test]
fn history_thins_to_hourly_then_daily_and_keeps_the_newest_four() {
    let now = at("2026-10-30T12:00:00Z");
    let dates = [
        "2026-10-30T11:50:00Z", // 0: newest four
        "2026-10-30T11:40:00Z", // 1
        "2026-10-30T11:30:00Z", // 2
        "2026-10-30T11:20:00Z", // 3
        "2026-10-30T11:10:00Z", // 4: hour 11 is taken
        "2026-10-30T09:15:00Z", // 5: newest of hour 9
        "2026-10-30T09:05:00Z", // 6: hour 9 is taken
        "2026-10-29T08:00:00Z", // 7: older than a day, newest of the 29th
        "2026-10-29T07:00:00Z", // 8: the 29th is taken
        "2026-10-01T00:00:00Z", // 9: newest of the 1st, 29.5 days old
        "2026-09-29T00:00:00Z", // 10: older than 30 days
        "invalid",              // 11: unknown, never dropped
        "2027-01-01T00:00:00Z", // 12: future, never dropped
    ];
    assert_eq!(expired(dates, now), [4, 6, 8, 10]);
}

#[test]
fn a_quiet_workspace_keeps_its_last_four_versions() {
    let now = at("2026-10-30T00:00:00Z");
    let old = [
        "2026-05-04T00:00:00Z",
        "2026-05-03T00:00:00Z",
        "2026-05-02T00:00:00Z",
        "2026-05-01T00:00:00Z",
        "2026-04-30T00:00:00Z",
    ];
    assert_eq!(expired(old, now), [4]);
    // Equal dates rank by their given order, so one of them still goes.
    let same = ["2026-05-01T00:00:00Z"; 6];
    assert_eq!(expired(same, now), [4, 5]);
}

#[test]
fn two_months_of_hourly_uploads_stay_bounded() {
    let now = at("2026-10-30T00:00:00Z");
    let dates = (0..24 * 60)
        .map(|hours| (now - Duration::hours(hours)).to_rfc3339())
        .collect::<Vec<_>>();
    let kept = dates.len() - expired(dates.iter().map(String::as_str), now).len();
    // 25 hourly copies (the 24-hour boundary is inclusive) plus one per
    // remaining day of the 30.
    assert!(kept <= 24 + 1 + 30, "kept {kept}");
}
