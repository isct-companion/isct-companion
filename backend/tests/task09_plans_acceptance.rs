use isct_core::plans::{preview_rrule, validate_rrule};

#[test]
fn rejects_sub_daily_frequencies_and_components_with_a_clear_boundary() {
    for rule in [
        "FREQ=HOURLY;INTERVAL=2",
        "FREQ=MINUTELY",
        "FREQ=SECONDLY",
        "FREQ=DAILY;BYHOUR=9",
        "FREQ=WEEKLY;BYMINUTE=30",
        "FREQ=MONTHLY;BYSECOND=5",
    ] {
        let error = validate_rrule(rule).expect_err(rule);
        let message = error.to_string().to_ascii_lowercase();
        assert!(
            message.contains("day") || message.contains("daily"),
            "{rule} should explain the minimum daily recurrence boundary: {error}"
        );
    }
}

#[test]
fn weekly_tokyo_rule_expands_tuesday_and_friday_and_respects_exdate() {
    let preview = preview_rrule(
        "2026-10-06T09:00:00+09:00",
        "FREQ=WEEKLY;BYDAY=TU,FR;UNTIL=20261020T000000Z",
        &["2026-10-09T09:00:00+09:00".into()],
        10,
    )
    .expect("valid Tokyo recurrence");

    assert_eq!(
        preview.occurrences,
        [
            "2026-10-06T09:00:00+09:00",
            "2026-10-13T09:00:00+09:00",
            "2026-10-16T09:00:00+09:00",
            "2026-10-20T09:00:00+09:00",
        ]
    );
}
