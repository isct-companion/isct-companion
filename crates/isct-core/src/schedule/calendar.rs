use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::{add_days, array, parse_civil_date, text, visible, ScheduleError, Term};

const QUARTERS: [&str; 4] = ["1Q", "2Q", "3Q", "4Q"];
const DAY_TYPES: [&str; 4] = ["holiday", "no-class", "replacement", "break"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TeachingRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct QuarterRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub teaching_ranges: Option<Vec<TeachingRange>>,
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CalendarDayView {
    pub date: String,
    #[serde(rename = "type")]
    pub day_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_day: Option<u8>,
    pub label: String,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "unknown | null"))]
    pub official: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(type = "unknown | null"))]
    pub exception: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct QuarterView {
    pub quarter: String,
    pub from: String,
    pub to: String,
    pub origin: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AcademicYearCalendar {
    pub year: i32,
    pub quarters: Vec<QuarterView>,
    pub days: Vec<CalendarDayView>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub important_dates: Vec<Value>,
    pub published: bool,
}

pub fn calendar_state(workspace: &Value, date: &str) -> Option<Value> {
    array(workspace, "exceptions")
        .iter()
        .find(|row| visible(row) && text(row, "date") == Some(date))
        .or_else(|| {
            array(workspace, "academicCalendar").iter().find(|row| {
                visible(row)
                    && text(row, "date") == Some(date)
                    && text(row, "type").is_some_and(|kind| DAY_TYPES.contains(&kind))
            })
        })
        .cloned()
}

pub(crate) fn quarter_range(
    workspace: &Value,
    term: &str,
) -> Result<Option<QuarterRange>, ScheduleError> {
    let term = Term::parse(term)?;
    let quarter = format!("{}Q", term.quarter);
    let next = if term.quarter == 4 {
        Term {
            year: term.year + 1,
            quarter: 1,
        }
    } else {
        Term {
            year: term.year,
            quarter: term.quarter + 1,
        }
    };
    let configured_year = profile_year(workspace, term.year);
    let next_profile = profile_year(workspace, next.year);
    let configured_start = configured_year
        .and_then(|row| row.get("quarterStarts"))
        .and_then(|starts| text(starts, &quarter))
        .filter(|raw| parse_civil_date(raw).is_ok());
    let next_quarter = format!("{}Q", next.quarter);
    let configured_next = next_profile
        .and_then(|row| row.get("quarterStarts"))
        .and_then(|starts| text(starts, &next_quarter))
        .filter(|raw| parse_civil_date(raw).is_ok());
    let start = boundary(workspace, term.year, &quarter, "quarter-start");
    let end = boundary(workspace, term.year, &quarter, "quarter-end");
    let next_start = boundary(workspace, next.year, &next_quarter, "quarter-start");
    let from = configured_start.or(start);
    let to = if let Some(value) = configured_next {
        Some(add_days(value, -1)?)
    } else if let Some(value) = end {
        Some(value.into())
    } else if let Some(value) = next_start {
        Some(add_days(value, -1)?)
    } else {
        None
    };
    let (Some(from), Some(to)) = (from, to) else {
        return Ok(None);
    };
    let canonical_id = format!("calendar:{}:quarter-start:{quarter}", term.year);
    let teaching_ranges = array(workspace, "academicCalendar")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(canonical_id.as_str()))
        .and_then(|row| row.get("teachingRanges"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some(TeachingRange {
                        start: text(row, "start")?.into(),
                        end: text(row, "end")?.into(),
                    })
                })
                .collect()
        });
    Ok(Some(QuarterRange {
        teaching_ranges,
        from: from.into(),
        to,
    }))
}

fn profile_year(workspace: &Value, year: i32) -> Option<&Value> {
    workspace
        .pointer("/academicProfile/years")
        .and_then(Value::as_array)?
        .iter()
        .find(|row| row.get("academicYear").and_then(Value::as_i64) == Some(i64::from(year)))
}

fn boundary<'a>(workspace: &'a Value, year: i32, quarter: &str, kind: &str) -> Option<&'a str> {
    let exact = format!("calendar:{year}:{kind}:{quarter}");
    let prefix = format!("calendar:{year}:");
    array(workspace, "academicCalendar")
        .iter()
        .find(|row| {
            visible(row)
                && text(row, "type") == Some(kind)
                && (row.get("academicYear").and_then(Value::as_i64) == Some(i64::from(year))
                    || text(row, "id").is_some_and(|id| id.starts_with(&prefix)))
                && (text(row, "quarter") == Some(quarter)
                    || text(row, "id") == Some(exact.as_str()))
        })
        .and_then(|row| text(row, "date"))
        .filter(|date| parse_civil_date(date).is_ok())
}

pub fn academic_year_of(date: &str) -> Result<i32, ScheduleError> {
    let date = parse_civil_date(date)?;
    Ok(if date.month() >= 4 {
        date.year()
    } else {
        date.year() - 1
    })
}

pub fn academic_year_calendar(
    workspace: &Value,
    year: i32,
) -> Result<AcademicYearCalendar, ScheduleError> {
    if !(1900..=9999).contains(&year) {
        return Err(ScheduleError::InvalidInput("Invalid academic year.".into()));
    }
    let locale = content_language(workspace);
    let official_rows: Vec<_> = array(workspace, "academicCalendar")
        .iter()
        .filter(|row| {
            visible(row)
                && row
                    .get("academicYear")
                    .and_then(Value::as_i64)
                    .map(|value| value as i32)
                    .or_else(|| text(row, "date").and_then(|date| academic_year_of(date).ok()))
                    == Some(year)
        })
        .collect();
    let official: BTreeMap<_, _> = official_rows
        .iter()
        .filter(|row| text(row, "type").is_some_and(|kind| DAY_TYPES.contains(&kind)))
        .filter_map(|row| {
            let date = text(row, "date")?;
            Some((
                date.to_owned(),
                json!({
                    "type": text(row, "type")?,
                    "replacementDay": row.get("replacementDay").cloned().unwrap_or(Value::Null),
                    "label": localized(row.get("label"), locale, date),
                }),
            ))
        })
        .collect();
    let mut days = Vec::new();
    let mut changed = HashSet::new();
    for exception in array(workspace, "exceptions")
        .iter()
        .filter(|row| visible(row) && text(row, "date").is_some_and(|date| in_year(date, year)))
    {
        let date = text(exception, "date").unwrap_or_default().to_owned();
        let published = official.get(&date).cloned();
        changed.insert(date.clone());
        days.push(CalendarDayView {
            date,
            day_type: text(exception, "type").unwrap_or_default().into(),
            replacement_day: exception
                .get("replacementDay")
                .and_then(Value::as_u64)
                .map(|v| v as u8),
            label: text(exception, "label").unwrap_or_default().into(),
            origin: if published.is_some() {
                "changed"
            } else {
                "own"
            }
            .into(),
            official: published,
            exception: Some(exception.clone()),
        });
    }
    for (date, row) in official {
        if changed.contains(&date) {
            continue;
        }
        days.push(CalendarDayView {
            date,
            day_type: text(&row, "type").unwrap_or_default().into(),
            replacement_day: row
                .get("replacementDay")
                .and_then(Value::as_u64)
                .map(|v| v as u8),
            label: text(&row, "label").unwrap_or_default().into(),
            origin: "official".into(),
            official: None,
            exception: None,
        });
    }
    days.sort_by(|left, right| left.date.cmp(&right.date));
    let configured: HashMap<_, _> = profile_year(workspace, year)
        .and_then(|row| row.get("quarterStarts"))
        .and_then(Value::as_object)
        .map(|values| {
            values
                .iter()
                .filter_map(|(k, v)| Some((k.as_str(), v.as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    let mut quarters = Vec::new();
    for quarter in QUARTERS {
        if let Some(range) = quarter_range(workspace, &format!("{year}-{quarter}"))? {
            quarters.push(QuarterView {
                quarter: quarter.into(),
                origin: if configured.get(quarter).copied() == Some(range.from.as_str()) {
                    "own"
                } else {
                    "official"
                }
                .into(),
                from: range.from,
                to: range.to,
            });
        }
    }
    let mut important_dates: Vec<_> = array(workspace, "importantDates")
        .iter()
        .filter(|row| {
            visible(row) && row.get("academicYear").and_then(Value::as_i64) == Some(i64::from(year))
        })
        .cloned()
        .collect();
    important_dates.sort_by(|a, b| text(a, "startDate").cmp(&text(b, "startDate")));
    Ok(AcademicYearCalendar {
        year,
        quarters,
        days,
        important_dates,
        published: !official_rows.is_empty(),
    })
}

pub fn academic_term_for_date(
    date: &str,
    profile: &Value,
) -> Result<Option<String>, ScheduleError> {
    parse_civil_date(date)?;
    let mut starts = profile
        .get("years")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|year| {
            let academic_year = year.get("academicYear").and_then(Value::as_i64);
            QUARTERS.into_iter().filter_map(move |quarter| {
                let start = year
                    .get("quarterStarts")
                    .and_then(|value| text(value, quarter))?;
                Some((start.to_owned(), format!("{}-{quarter}", academic_year?)))
            })
        })
        .filter(|(start, _)| start.as_str() <= date)
        .collect::<Vec<_>>();
    starts.sort_by(|left, right| right.0.cmp(&left.0));
    Ok(starts.first().map(|(_, term)| term.clone()))
}

fn in_year(date: &str, year: i32) -> bool {
    date >= format!("{year}-04-01").as_str() && date <= format!("{}-03-31", year + 1).as_str()
}

pub(crate) fn content_language(workspace: &Value) -> &str {
    match workspace
        .pointer("/settings/contentLanguage")
        .and_then(Value::as_str)
    {
        Some("ja") => "ja",
        Some("en") => "en",
        _ if workspace
            .pointer("/settings/locale")
            .and_then(Value::as_str)
            .is_some_and(|locale| matches!(locale, "ja" | "zh")) =>
        {
            "ja"
        }
        _ => "en",
    }
}

pub(crate) fn localized(value: Option<&Value>, locale: &str, fallback: &str) -> String {
    let Some(value) = value else {
        return fallback.into();
    };
    if let Some(text) = value.as_str() {
        return text.into();
    }
    let language = if locale == "ja" { "ja" } else { "en" };
    value
        .get(language)
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get(if language == "ja" { "en" } else { "ja" })
                .and_then(Value::as_str)
        })
        .unwrap_or(fallback)
        .into()
}

use chrono::Datelike;
