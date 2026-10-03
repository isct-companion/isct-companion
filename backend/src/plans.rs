use chrono::{DateTime, NaiveDate, NaiveDateTime};
use rrule::RRuleSet;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurrencePreview {
    pub normalized: String,
    pub occurrences: Vec<String>,
}

fn local_start(value: &str) -> Result<String, String> {
    if let Ok(datetime) = DateTime::parse_from_rfc3339(value) {
        return Ok(datetime.format("%Y%m%dT%H%M%S").to_string());
    }
    if let Ok(datetime) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S") {
        return Ok(datetime.format("%Y%m%dT%H%M%S").to_string());
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "Plan start must be an ISO date or date-time.".to_string())?;
    Ok(date.format("%Y%m%dT000000").to_string())
}

pub fn validate_rrule(raw: &str) -> Result<String, String> {
    let rule = raw.trim().strip_prefix("RRULE:").unwrap_or(raw.trim());
    if rule.is_empty() || rule.contains(['\r', '\n']) {
        return Err("Enter one RRULE without additional calendar properties.".into());
    }
    let upper = rule.to_ascii_uppercase();
    let frequency = upper
        .split(';')
        .find_map(|part| part.strip_prefix("FREQ="))
        .ok_or_else(|| "RRULE must include FREQ.".to_string())?;
    if !matches!(frequency, "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY") {
        return Err("Plans may repeat daily, weekly, monthly, or yearly.".into());
    }
    for forbidden in ["BYHOUR=", "BYMINUTE=", "BYSECOND="] {
        if upper.split(';').any(|part| part.starts_with(forbidden)) {
            return Err("Plan recurrence has a minimum unit of one day.".into());
        }
    }
    Ok(upper)
}

pub fn preview_rrule(
    start: &str,
    raw: &str,
    exdates: &[String],
    limit: u16,
) -> Result<RecurrencePreview, String> {
    let normalized = validate_rrule(raw)?;
    let mut calendar = format!(
        "DTSTART;TZID=Asia/Tokyo:{}\nRRULE:{}",
        local_start(start)?,
        normalized
    );
    for date in exdates {
        calendar.push_str(&format!("\nEXDATE;TZID=Asia/Tokyo:{}", local_start(date)?));
    }
    let set: RRuleSet = calendar
        .parse()
        .map_err(|error| format!("Invalid RRULE: {error}"))?;
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
