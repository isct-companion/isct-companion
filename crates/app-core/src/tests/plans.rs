use super::*;
use serde_json::json;

#[test]
fn plan_timestamps_share_japan_civil_semantics() {
    assert_eq!(
        canonical_plan_timestamp("2026-10-14T09:00").unwrap(),
        "2026-10-14T09:00:00+09:00"
    );
    assert_eq!(
        canonical_plan_timestamp("2026-10-13T18:00:00-04:00").unwrap(),
        "2026-10-14T07:00:00+09:00"
    );
    assert_eq!(
        plan_civil_start("2026-10-14").unwrap(),
        (
            NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
            NaiveTime::MIN
        )
    );
    for invalid in [
        "2026-1-14",
        "2026-10-4T09:00",
        "2026-10-14T9:00",
        "2026-10-14 09:00",
        "2026/10/14 09:00",
    ] {
        assert_eq!(parse_plan_timestamp(invalid), Err(PlanError::InvalidStart));
    }
}

#[test]
fn plan_draft_is_canonical_and_date_exclusions_apply_to_recurrence() {
    let mut draft = json!({
        "title":"  Review  ", "start":"2026-10-14T09:00", "duration":60,
        "rrule":"freq=daily;count=3",
        "exdates":["2026-10-15T23:00:00+09:00", "2026-10-15"]
    });
    canonicalize_plan_draft(&mut draft).unwrap();
    assert_eq!(draft["title"], "Review");
    assert_eq!(draft["start"], "2026-10-14T09:00:00+09:00");
    assert_eq!(draft["rrule"], "FREQ=DAILY;COUNT=3");
    assert_eq!(draft["exdates"], json!(["2026-10-15"]));
    assert_eq!(
        occurrences_between(
            draft["start"].as_str().unwrap(),
            draft["rrule"].as_str().unwrap(),
            &["2026-10-15".into()],
            NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(),
        )
        .unwrap()
        .iter()
        .map(|value| plan_civil_start(value).unwrap().0)
        .collect::<Vec<_>>(),
        vec![
            NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()
        ]
    );

    let mut invalid_rule = json!({
        "title":"Broken", "start":"2026-10-14T09:00", "duration":60,
        "rrule":"FREQ=DAILY;INTERVAL=garbage", "exdates":[]
    });
    assert!(matches!(
        canonicalize_plan_draft(&mut invalid_rule),
        Err(PlanError::InvalidRule(_))
    ));
}

#[test]
fn until_uses_japan_civil_time_in_every_accepted_form() {
    let dates = |rule: &str| {
        occurrences_between(
            "2026-12-29T23:30:00+09:00",
            rule,
            &[],
            NaiveDate::from_ymd_opt(2026, 12, 29).unwrap(),
            NaiveDate::from_ymd_opt(2027, 1, 5).unwrap(),
        )
        .unwrap()
        .iter()
        .map(|value| plan_civil_start(value).unwrap().0.to_string())
        .collect::<Vec<_>>()
    };
    // Date-only: the whole JST day, including a late-evening start.
    assert_eq!(
        dates("FREQ=DAILY;UNTIL=20261231"),
        ["2026-12-29", "2026-12-30", "2026-12-31"]
    );
    // Floating: JST, independent of the machine's zone.
    assert_eq!(
        dates("FREQ=DAILY;UNTIL=20261230T233000"),
        ["2026-12-29", "2026-12-30"]
    );
    // UTC: 2026-12-30T14:30Z is 23:30 JST on 30 December.
    assert_eq!(
        dates("FREQ=DAILY;UNTIL=20261230T143000Z"),
        ["2026-12-29", "2026-12-30"]
    );
    for invalid in ["FREQ=DAILY;UNTIL=2026-12-31", "FREQ=DAILY;UNTIL=20261331"] {
        assert!(matches!(
            validate_plan_recurrence("2026-12-29T09:00", Some(invalid)),
            Err(PlanError::InvalidRule(_))
        ));
    }
    let mut draft = json!({
        "title":"Until", "start":"2026-12-29T09:00", "duration":30,
        "rrule":"freq=daily;until=20261231", "exdates":[]
    });
    canonicalize_plan_draft(&mut draft).unwrap();
    assert_eq!(draft["rrule"], "FREQ=DAILY;UNTIL=20261231");
}

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

#[test]
fn recurrence_range_does_not_return_unrelated_dates() {
    let dates = occurrences_between(
        "2026-10-01T10:00:00+09:00",
        "FREQ=WEEKLY;BYDAY=TH",
        &[],
        NaiveDate::from_ymd_opt(2026, 10, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 10, 14).unwrap(),
    )
    .unwrap();
    assert_eq!(dates.len(), 1);
    assert!(dates[0].starts_with("2026-10-08T10:00:00"));
}
