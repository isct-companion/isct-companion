//! Date-first calendar content, shared by the interactive view and ICS export.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;

use super::calendar::{
    academic_year_of, calendar_state, content_language, localized, quarter_range,
};
use super::occurrence::{finalize_agenda, personal_occurrences};
use super::{
    array, parse_civil_date, placement_terms, resolve_day_agenda, text, visible, AgendaOptions,
    DatedAgenda, OccurrenceKind, ScheduleError, Term,
};

/// Inclusive civil-date range. Cancelled classes remain in the content so each
/// consumer can present or omit them without recomputing calendar policy.
pub fn calendar_between(
    workspace: &Value,
    from: &str,
    to: &str,
) -> Result<Vec<DatedAgenda>, ScheduleError> {
    let start = parse_civil_date(from)?;
    let end = parse_civil_date(to)?;
    let days = (end - start).num_days();
    if !(0..=366).contains(&days) {
        return Err(ScheduleError::InvalidInput(
            "date range must be ordered and no longer than 367 days".into(),
        ));
    }
    let mut terms: BTreeSet<String> = placement_terms(workspace).map(str::to_owned).collect();
    for event in array(workspace, "events").iter().filter(|row| visible(row)) {
        if let Some(term) = text(event, "termId") {
            terms.insert(term.into());
        }
    }
    for exam in array(workspace, "exams").iter().filter(|row| visible(row)) {
        if let (Some(year), Some(quarter)) = (
            exam.get("academicYear").and_then(Value::as_i64),
            text(exam, "quarter"),
        ) {
            terms.insert(format!("{year}-{quarter}"));
        }
    }
    for year in academic_year_of(from)?..=academic_year_of(to)? {
        for quarter in 1..=4 {
            terms.insert(format!("{year}-{quarter}Q"));
        }
    }
    // Keep source terms even when their ranges do not overlap the request:
    // an explicit override can move a class across a quarter or year boundary.
    let ranges: Vec<_> = terms
        .iter()
        .filter(|term| Term::parse(term).is_ok())
        .map(|term| Ok((term.clone(), quarter_range(workspace, term)?)))
        .collect::<Result<_, ScheduleError>>()?;
    let locale = content_language(workspace);
    let mut agendas = Vec::with_capacity((days + 1) as usize);
    for offset in 0..=days {
        let date = (start + chrono::Duration::days(offset)).to_string();
        let state = calendar_state(workspace, &date);
        let state_kind = state.as_ref().and_then(|row| text(row, "type"));
        let active_ranges: Vec<_> = ranges
            .iter()
            .filter_map(|(_, range)| range.as_ref())
            .filter(|range| date >= range.from && date <= range.to)
            .collect();
        let outside_teaching = !active_ranges.is_empty()
            && state_kind != Some("replacement")
            && active_ranges.iter().all(|range| {
                range.teaching_ranges.as_ref().is_some_and(|ranges| {
                    !ranges
                        .iter()
                        .any(|range| date >= range.start && date <= range.end)
                })
            });
        let exam_period = active_ranges.iter().any(|range| {
            range.exam_ranges.as_ref().is_some_and(|ranges| {
                ranges
                    .iter()
                    .any(|range| date >= range.start && date <= range.end)
            })
        });
        let weekly_classes_stop =
            outside_teaching || matches!(state_kind, Some("holiday" | "no-class" | "break"));
        let mut blocks = Vec::new();
        for (term, _) in &ranges {
            blocks.extend(resolve_day_agenda(
                workspace,
                term,
                &date,
                AgendaOptions {
                    include_personal: false,
                    include_cancelled_courses: true,
                    ..AgendaOptions::default()
                },
            )?);
        }
        // Dated events follow their date. Weekly template events follow their
        // term's range. Plans are global and must only be generated once.
        let events: Vec<_> = array(workspace, "events")
            .iter()
            .filter(|event| {
                visible(event)
                    && (text(event, "date").is_some()
                        || ranges.iter().any(|(term, range)| {
                            text(event, "termId") == Some(term.as_str())
                                && range
                                    .as_ref()
                                    .is_some_and(|range| date >= range.from && date <= range.to)
                        }))
            })
            .cloned()
            .collect();
        blocks.extend(personal_occurrences(workspace, &date, &events)?);
        // A course may have placements in overlapping configured quarters.
        // Keep its effective occurrence once, preferring a class that runs.
        blocks.sort_by_key(|block| block.cancelled == Some(true));
        let mut seen = HashSet::new();
        blocks.retain(|block| seen.insert(block.key.clone()));
        let important_dates: Vec<_> = array(workspace, "importantDates")
            .iter()
            .filter(|row| {
                visible(row)
                    && text(row, "startDate").is_some_and(|from| date.as_str() >= from)
                    && text(row, "endDate").is_some_and(|to| date.as_str() <= to)
            })
            .cloned()
            .collect();
        let mut notices = Vec::new();
        if let Some(state) = state.as_ref() {
            let label = localized(state.get("label"), locale, "");
            if !label.is_empty() {
                notices.push(label);
            }
        }
        notices.extend(important_dates.iter().filter_map(|row| {
            let label = localized(row.get("label"), locale, "");
            (!label.is_empty()).then_some(label)
        }));
        let occurrences = finalize_agenda(blocks)?;
        // Exams and timetable classes still happen when the weekly pattern
        // stops; such a day must not read as a day without classes.
        let no_classes = weekly_classes_stop
            && !occurrences.iter().any(|block| {
                block.cancelled != Some(true)
                    && matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam)
            });
        if notices.is_empty() && weekly_classes_stop {
            notices.push(
                match (exam_period, locale == "ja") {
                    (true, true) => "試験期間",
                    (true, false) => "Exam period",
                    (false, true) => "授業なし",
                    (false, false) => "No classes",
                }
                .into(),
            );
        }
        agendas.push(DatedAgenda {
            occurrences,
            date,
            calendar_state: state,
            important_dates,
            no_classes,
            calendar_notice: (!notices.is_empty()).then(|| notices.join(" · ")),
        });
    }
    Ok(agendas)
}
