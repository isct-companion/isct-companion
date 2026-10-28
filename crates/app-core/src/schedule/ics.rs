use super::SchoolReads;
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::calendar::quarter_range;
use super::{calendar_between, OccurrenceKind, ScheduleError, Term};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "kebab-case")]
pub enum CalendarExportScope {
    #[default]
    Courses,
    /// Courses and the weekly personal events of the same quarters.
    CoursesAndEvents,
}

pub fn export_quarter_ics(
    workspace: &Value,
    school: &SchoolReads,
    term: &str,
    scope: CalendarExportScope,
    generated_at: DateTime<FixedOffset>,
) -> Result<String, ScheduleError> {
    Term::parse(term)?;
    let range = quarter_range(workspace, term)?.ok_or_else(|| {
        ScheduleError::InvalidInput("Configure this quarter's dates before exporting it.".into())
    })?;
    export_calendar_ics(
        workspace,
        school,
        &range.from,
        &range.to,
        scope,
        generated_at,
    )
}

pub fn export_calendar_ics(
    workspace: &Value,
    school: &SchoolReads,
    from: &str,
    to: &str,
    scope: CalendarExportScope,
    generated_at: DateTime<FixedOffset>,
) -> Result<String, ScheduleError> {
    let agendas = calendar_between(workspace, school, from, to)?;
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
    for agenda in agendas {
        let date = agenda.date;
        for block in agenda.occurrences {
            if block.cancelled == Some(true) {
                continue;
            }
            if block.kind != OccurrenceKind::Course
                && !(matches!(scope, CalendarExportScope::CoursesAndEvents)
                    && block.kind == OccurrenceKind::Event)
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
/// Letters, digits and `-_.!~*'()` stay; every other byte is `%XX`.
pub(super) fn percent_encode(value: &str) -> String {
    const KEEP: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')');
    percent_encoding::utf8_percent_encode(value, KEEP).to_string()
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
