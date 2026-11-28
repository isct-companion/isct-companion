//! Date-first calendar content, shared by the interactive view and ICS export.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::SchoolReads;

use serde_json::Value;

use super::agenda_layout::finalize_agenda;
use super::calendar::{academic_year_of, calendar_state, shown_label};
use super::occurrence::{personal_occurrences, CourseSessions};
use super::{
    array, parse_civil_date, teaching_periods, text, visible, DatedAgenda, Occurrence,
    ScheduleError, Term,
};
use crate::locale::ContentLanguage;

/// Inclusive civil-date range. Cancelled classes remain in the content so each
/// consumer can present or omit them without recomputing calendar policy.
pub(crate) fn calendar_between(
    workspace: &Value,
    school: &SchoolReads,
    from: &str,
    to: &str,
) -> Result<Vec<DatedAgenda>, ScheduleError> {
    calendar_between_with_personal(workspace, school, from, to, true)
}

pub(super) fn calendar_between_with_personal(
    workspace: &Value,
    school: &SchoolReads,
    from: &str,
    to: &str,
    include_personal: bool,
) -> Result<Vec<DatedAgenda>, ScheduleError> {
    let start = parse_civil_date(from)?;
    let end = parse_civil_date(to)?;
    let days = (end - start).num_days();
    if !(0..=366).contains(&days) {
        return Err(ScheduleError::InvalidInput(
            "date range must be ordered and no longer than 367 days".into(),
        ));
    }
    let periods = teaching_periods(workspace)?;
    let mut terms = super::exams::exam_terms(workspace, school);
    for event in array(workspace, "events").iter().filter(|row| visible(row)) {
        if let Some(term) = text(event, "termId") {
            terms.insert(term.into());
        }
    }
    for year in academic_year_of(from)?..=academic_year_of(to)? {
        for quarter in 1..=4 {
            terms.insert(format!("{year}-{quarter}Q"));
        }
    }
    // Keep source terms even when their ranges do not overlap the request:
    // an explicit override can move a class across a quarter or year boundary.
    let sources = terms
        .iter()
        .filter(|term| Term::parse(term).is_ok())
        .map(|term| {
            Ok((
                CourseSessions::new(workspace, term, None)?,
                super::exams::published_sessions_by_date(workspace, school, term),
            ))
        })
        .collect::<Result<Vec<_>, ScheduleError>>()?;
    let language = ContentLanguage::of(workspace);
    let mut agendas = Vec::with_capacity((days + 1) as usize);
    for offset in 0..=days {
        let date = (start + chrono::Duration::days(offset)).to_string();
        let state = calendar_state(workspace, &date);
        let state_kind = state.as_ref().and_then(|row| text(row, "type"));
        let active_ranges: Vec<_> = sources
            .iter()
            .filter_map(|(sessions, _)| sessions.range.as_ref())
            .filter(|range| range.contains(&date))
            .collect();
        let outside_teaching = !active_ranges.is_empty()
            && state_kind != Some("replacement")
            && active_ranges
                .iter()
                .all(|range| range.outside_teaching(&date));
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
        for (sessions, exams) in &sources {
            blocks.extend(sessions.blocks_on(
                &date,
                exams.get(&date).map(Vec::as_slice).unwrap_or_default(),
                true,
            )?);
        }
        if include_personal {
            let events: Vec<_> = array(workspace, "events")
                .iter()
                .filter(|event| {
                    visible(event)
                        && sources.iter().any(|(sessions, _)| {
                            text(event, "termId") == Some(sessions.term)
                                && sessions
                                    .range
                                    .as_ref()
                                    .is_some_and(|range| range.contains(&date))
                        })
                })
                .cloned()
                .collect();
            blocks.extend(personal_occurrences(&date, &events)?);
        }
        let important_dates: Vec<_> = array(workspace, "importantDates")
            .iter()
            .filter(|row| {
                visible(row)
                    && text(row, "startDate").is_some_and(|from| date.as_str() >= from)
                    && text(row, "endDate").is_some_and(|to| date.as_str() <= to)
            })
            .map(|row| shown_label(row, language))
            .collect();
        let mut notices = Vec::new();
        notices.extend(
            state
                .iter()
                .chain(&important_dates)
                .filter_map(|row| text(row, "label"))
                .filter(|label| !label.is_empty())
                .map(str::to_owned),
        );
        let occurrences = finalize_agenda(blocks, &periods)?;
        // Exams and timetable classes still happen when the weekly pattern
        // stops; such a day must not read as a day without classes.
        let no_classes =
            weekly_classes_stop && !occurrences.iter().any(Occurrence::is_active_class);
        if notices.is_empty() && weekly_classes_stop {
            notices.push(
                match (exam_period, language == ContentLanguage::Ja) {
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
