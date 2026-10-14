use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::calendar::quarter_range;
use super::{add_days, resolve_day_agenda, AgendaOptions, OccurrenceKind, ScheduleError, Term};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "kebab-case")]
pub enum CalendarExportScope {
    #[default]
    Courses,
    CoursesAndPlans,
}

pub fn export_quarter_ics(
    workspace: &Value,
    term: &str,
    scope: CalendarExportScope,
    generated_at: DateTime<FixedOffset>,
) -> Result<String, ScheduleError> {
    Term::parse(term)?;
    let range = quarter_range(workspace, term)?.ok_or_else(|| {
        ScheduleError::InvalidInput("Configure this quarter's dates before exporting it.".into())
    })?;
    let days =
        (super::parse_civil_date(&range.to)? - super::parse_civil_date(&range.from)?).num_days();
    if !(0..=366).contains(&days) {
        return Err(ScheduleError::InvalidInput(
            "Invalid quarter date range.".into(),
        ));
    }
    let mut lines = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//IsctCompanion//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "BEGIN:VTIMEZONE".into(),
        "TZID:Asia/Tokyo".into(),
        "BEGIN:STANDARD".into(),
        "DTSTART:19700101T000000".into(),
        "TZOFFSETFROM:+0900".into(),
        "TZOFFSETTO:+0900".into(),
        "TZNAME:JST".into(),
        "END:STANDARD".into(),
        "END:VTIMEZONE".into(),
    ];
    let stamp = generated_at
        .with_timezone(&Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string();
    for offset in 0..=days {
        let date = add_days(&range.from, offset)?;
        for block in resolve_day_agenda(
            workspace,
            term,
            &date,
            AgendaOptions {
                include_personal: matches!(scope, CalendarExportScope::CoursesAndPlans),
                ..AgendaOptions::default()
            },
        )? {
            if block.kind != OccurrenceKind::Course
                && !(matches!(scope, CalendarExportScope::CoursesAndPlans)
                    && block.kind == OccurrenceKind::Plan)
            {
                continue;
            }
            lines.push("BEGIN:VEVENT".into());
            lines.push(format!(
                "UID:{}@isctcompanion.local",
                percent_encode(&block.key)
            ));
            lines.push(format!("DTSTAMP:{stamp}"));
            lines.push(format!("SUMMARY:{}", escape(&block.title)));
            lines.push(format!(
                "DTSTART;TZID=Asia/Tokyo:{}T{}00",
                date.replace('-', ""),
                block.start_time.replace(':', "")
            ));
            let end_date = block.end_date.as_deref().unwrap_or(&date);
            lines.push(format!(
                "DTEND;TZID=Asia/Tokyo:{}T{}00",
                end_date.replace('-', ""),
                block.end_time.replace(':', "")
            ));
            if !block.room.is_empty() {
                lines.push(format!("LOCATION:{}", escape(&block.room)));
            }
            lines.push("END:VEVENT".into());
        }
    }
    lines.push("END:VCALENDAR".into());
    Ok(lines
        .into_iter()
        .map(|line| fold(&line))
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n")
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}
fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(byte) {
            encoded.push(char::from(*byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}
fn fold(line: &str) -> String {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut size = 0;
    for character in line.chars() {
        let bytes = character.len_utf8();
        if size + bytes > 75 {
            parts.push(current);
            current = " ".into();
            size = 1;
        }
        current.push(character);
        size += bytes;
    }
    parts.push(current);
    parts.join("\r\n")
}
