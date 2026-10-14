use std::collections::HashSet;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, TimeZone};
use rrule::{RRuleSet, Tz};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurrencePreview {
    pub normalized: String,
    pub occurrences: Vec<String>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PlanError {
    #[error("Plan start must be an ISO date or date-time.")]
    InvalidStart,
    #[error("Plan title is required.")]
    InvalidTitle,
    #[error("Plan duration must be between 1 and 10080 minutes.")]
    InvalidDuration,
    #[error("Plan exclusion dates must be ISO dates or date-times.")]
    InvalidExdate,
    #[error("Plan reminder must be a non-negative integer.")]
    InvalidReminder,
    #[error("Plan draft must be an object.")]
    InvalidDraft,
    #[error("Enter one RRULE without additional calendar properties.")]
    AdditionalCalendarProperties,
    #[error("RRULE must include FREQ.")]
    MissingFrequency,
    #[error("Plans may repeat daily, weekly, monthly, or yearly.")]
    UnsupportedFrequency,
    #[error("Plan recurrence has a minimum unit of one day.")]
    SubdailyFrequency,
    #[error("Invalid RRULE: {0}")]
    InvalidRule(String),
    #[error("range start is invalid")]
    InvalidRangeStart,
    #[error("range end is invalid")]
    InvalidRangeEnd,
}

fn jst() -> Result<FixedOffset, PlanError> {
    FixedOffset::east_opt(9 * 3600).ok_or(PlanError::InvalidStart)
}

/// Parse every stored plan timestamp using Japan civil time as the domain clock.
/// Legacy date and local date-time values are floating JST; explicit offsets are
/// converted to JST before their civil date or time is used.
pub fn parse_plan_timestamp(value: &str) -> Result<DateTime<FixedOffset>, PlanError> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
        if !strict_rfc3339_shape(value, &datetime) {
            return Err(PlanError::InvalidStart);
        }
        return Ok(datetime.with_timezone(&jst()?));
    }
    let local = if let Ok(datetime) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        (datetime.format("%Y-%m-%dT%H:%M:%S").to_string() == value).then_some(datetime)
    } else if let Ok(datetime) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M") {
        (datetime.format("%Y-%m-%dT%H:%M").to_string() == value).then_some(datetime)
    } else if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        (date.format("%Y-%m-%d").to_string() == value).then_some(date.and_time(NaiveTime::MIN))
    } else {
        None
    }
    .ok_or(PlanError::InvalidStart)?;
    jst()?
        .from_local_datetime(&local)
        .single()
        .ok_or(PlanError::InvalidStart)
}

fn strict_rfc3339_shape(value: &str, parsed: &DateTime<FixedOffset>) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || bytes[..19]
            .iter()
            .enumerate()
            .any(|(index, byte)| !matches!(index, 4 | 7 | 10 | 13 | 16) && !byte.is_ascii_digit())
        || parsed.format("%Y-%m-%dT%H:%M:%S").to_string() != value[..19]
    {
        return false;
    }
    let suffix = &value[19..];
    if suffix == "Z" {
        return true;
    }
    if let Some(fraction) = suffix.strip_suffix('Z') {
        return fraction.strip_prefix('.').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
    }
    let zone_at = suffix
        .rfind(['+', '-'])
        .filter(|index| suffix.len().saturating_sub(*index) == 6);
    let Some(zone_at) = zone_at else { return false };
    let fraction = &suffix[..zone_at];
    let zone = &suffix[zone_at..];
    (fraction.is_empty()
        || fraction.strip_prefix('.').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        }))
        && zone.as_bytes().get(3) == Some(&b':')
        && zone[1..3].bytes().all(|byte| byte.is_ascii_digit())
        && zone[4..6].bytes().all(|byte| byte.is_ascii_digit())
}

pub fn canonical_plan_timestamp(value: &str) -> Result<String, PlanError> {
    Ok(parse_plan_timestamp(value)?.to_rfc3339_opts(SecondsFormat::Secs, false))
}

pub fn plan_civil_start(value: &str) -> Result<(NaiveDate, NaiveTime), PlanError> {
    let parsed = parse_plan_timestamp(value)?;
    Ok((parsed.date_naive(), parsed.time()))
}

fn local_start(value: &str) -> Result<String, PlanError> {
    Ok(parse_plan_timestamp(value)?
        .format("%Y%m%dT%H%M%S")
        .to_string())
}

pub fn canonicalize_plan_draft(draft: &mut Value) -> Result<(), PlanError> {
    let object = draft.as_object_mut().ok_or(PlanError::InvalidDraft)?;
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or(PlanError::InvalidTitle)?
        .to_owned();
    object.insert("title".into(), Value::String(title));

    let start = object
        .get("start")
        .and_then(Value::as_str)
        .ok_or(PlanError::InvalidStart)?;
    object.insert(
        "start".into(),
        Value::String(canonical_plan_timestamp(start)?),
    );

    if !object
        .get("duration")
        .and_then(Value::as_u64)
        .is_some_and(|minutes| (1..=10_080).contains(&minutes))
    {
        return Err(PlanError::InvalidDuration);
    }

    if let Some(rule) = object.get("rrule").filter(|value| !value.is_null()) {
        let normalized = rule
            .as_str()
            .ok_or_else(|| PlanError::InvalidRule("RRULE must be text.".into()))
            .and_then(validate_rrule)?;
        recurrence_set(
            object
                .get("start")
                .and_then(Value::as_str)
                .ok_or(PlanError::InvalidStart)?,
            &normalized,
        )?;
        object.insert("rrule".into(), Value::String(normalized));
    }

    let exdates = object
        .entry("exdates")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or(PlanError::InvalidExdate)?;
    let mut normalized = Vec::with_capacity(exdates.len());
    for value in exdates.iter() {
        let raw = value.as_str().ok_or(PlanError::InvalidExdate)?;
        let (date, _) = plan_civil_start(raw).map_err(|_| PlanError::InvalidExdate)?;
        normalized.push(Value::String(date.to_string()));
    }
    normalized.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    normalized.dedup();
    *exdates = normalized;

    if object
        .get("reminder")
        .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
    {
        return Err(PlanError::InvalidReminder);
    }
    Ok(())
}

pub fn validate_rrule(raw: &str) -> Result<String, PlanError> {
    let rule = raw.trim().strip_prefix("RRULE:").unwrap_or(raw.trim());
    if rule.is_empty() || rule.contains(['\r', '\n']) {
        return Err(PlanError::AdditionalCalendarProperties);
    }
    let upper = rule.to_ascii_uppercase();
    let frequency = upper
        .split(';')
        .find_map(|part| part.strip_prefix("FREQ="))
        .ok_or(PlanError::MissingFrequency)?;
    if !matches!(frequency, "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY") {
        return Err(PlanError::UnsupportedFrequency);
    }
    for forbidden in ["BYHOUR=", "BYMINUTE=", "BYSECOND="] {
        if upper.split(';').any(|part| part.starts_with(forbidden)) {
            return Err(PlanError::SubdailyFrequency);
        }
    }
    Ok(upper)
}

fn recurrence_set(start: &str, raw: &str) -> Result<(String, RRuleSet), PlanError> {
    let normalized = validate_rrule(raw)?;
    let calendar = format!(
        "DTSTART;TZID=Asia/Tokyo:{}\nRRULE:{}",
        local_start(start)?,
        normalized
    );
    let set = calendar
        .parse::<RRuleSet>()
        .map_err(|error| PlanError::InvalidRule(error.to_string()))?;
    Ok((normalized, set))
}

pub fn preview_rrule(
    start: &str,
    raw: &str,
    exdates: &[String],
    limit: u16,
) -> Result<RecurrencePreview, PlanError> {
    let (normalized, set) = recurrence_set(start, raw)?;
    let excluded = excluded_dates(exdates)?;
    let japan = jst()?;
    let occurrences = set
        .all(limit.saturating_add(exdates.len().min(100) as u16).min(100))
        .dates
        .into_iter()
        .filter(|date| !excluded.contains(&date.with_timezone(&japan).date_naive()))
        .take(usize::from(limit.min(100)))
        .map(|date| date.to_rfc3339())
        .collect();
    Ok(RecurrencePreview {
        normalized,
        occurrences,
    })
}

pub fn occurrences_between(
    start: &str,
    raw: &str,
    exdates: &[String],
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<String>, PlanError> {
    let (_, set) = recurrence_set(start, raw)?;
    let excluded = excluded_dates(exdates)?;
    let japan = jst()?;
    let range_start = from
        .and_hms_opt(0, 0, 0)
        .ok_or(PlanError::InvalidRangeStart)?;
    let after = Tz::Asia__Tokyo
        .from_local_datetime(&range_start)
        .single()
        .ok_or(PlanError::InvalidRangeStart)?;
    let range_end = to
        .and_hms_opt(23, 59, 59)
        .ok_or(PlanError::InvalidRangeEnd)?;
    let before = Tz::Asia__Tokyo
        .from_local_datetime(&range_end)
        .single()
        .ok_or(PlanError::InvalidRangeEnd)?;
    Ok(set
        .after(after)
        .before(before)
        .all(1_000)
        .dates
        .into_iter()
        .filter(|date| !excluded.contains(&date.with_timezone(&japan).date_naive()))
        .map(|date| date.to_rfc3339())
        .collect())
}

fn excluded_dates(values: &[String]) -> Result<HashSet<NaiveDate>, PlanError> {
    values
        .iter()
        .map(|value| {
            plan_civil_start(value)
                .map(|(date, _)| date)
                .map_err(|_| PlanError::InvalidExdate)
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
}
