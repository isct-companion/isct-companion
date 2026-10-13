use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone};
use rrule::{RRuleSet, Tz};
use serde::Serialize;
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

fn local_start(value: &str) -> Result<String, PlanError> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
        return Ok(datetime.format("%Y%m%dT%H%M%S").to_string());
    }
    if let Ok(datetime) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Ok(datetime.format("%Y%m%dT%H%M%S").to_string());
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| PlanError::InvalidStart)?;
    Ok(date.format("%Y%m%dT000000").to_string())
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

fn recurrence_set(start: &str, raw: &str, exdates: &[String]) -> Result<(String, RRuleSet), PlanError> {
    let normalized = validate_rrule(raw)?;
    let mut calendar = format!(
        "DTSTART;TZID=Asia/Tokyo:{}\nRRULE:{}",
        local_start(start)?,
        normalized
    );
    for date in exdates {
        calendar.push_str(&format!("\nEXDATE;TZID=Asia/Tokyo:{}", local_start(date)?));
    }
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
    let (normalized, set) = recurrence_set(start, raw, exdates)?;
    let occurrences = set
        .all(limit.min(100))
        .dates
        .into_iter()
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
    let (_, set) = recurrence_set(start, raw, exdates)?;
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
        .map(|date| date.to_rfc3339())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
