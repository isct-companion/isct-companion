//! Date-first calendar content, shared by the interactive view and ICS export.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::SchoolReads;

use chrono::Datelike;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::agenda_layout::finalize_agenda;
use super::calendar::{academic_year_of, calendar_state, shown_label};
use super::occurrence::{personal_occurrences, CourseSessions};
use super::{
    array, parse_civil_date, teaching_periods, text, visible, DatedAgenda, Occurrence,
    ScheduleError, Term, TimetableAxis,
};
use crate::locale::ContentLanguage;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TimetablePage {
    pub monday: String,
    pub agendas: Vec<DatedAgenda>,
    pub axis: TimetableAxis,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum TimetableMode {
    /// The calendar shows classes and personal events for all seven days.
    Calendar,
    /// The class timetable shows courses and exams from Monday to Friday.
    Classes,
}

pub(super) fn timetable_weeks(
    workspace: &Value,
    school: &SchoolReads,
    monday: &str,
    reach: u8,
    mode: TimetableMode,
) -> Result<Vec<TimetablePage>, ScheduleError> {
    if reach > 4 || parse_civil_date(monday)?.weekday() != chrono::Weekday::Mon {
        return Err(ScheduleError::InvalidInput(
            "timetable requires a Monday and up to four surrounding weeks".into(),
        ));
    }
    let reach = 7 * i64::from(reach);
    let first = super::add_days(monday, -reach)?;
    let last = super::add_days(monday, reach + 6)?;
    let periods = teaching_periods(workspace)?;
    let (days_per_page, include_personal) = match mode {
        TimetableMode::Calendar => (7, true),
        TimetableMode::Classes => (5, false),
    };
    let agendas = calendar_between_with_personal(workspace, school, &first, &last, include_personal)?;
    agendas
        .chunks(7)
        .map(|week| {
            let days = &week[..days_per_page];
            let axis = super::timing::axis_for_occurrences(
                &periods,
                days.iter().flat_map(|day| &day.occurrences),
            )?;
            Ok(TimetablePage {
                monday: days[0].date.clone(),
                agendas: days.to_vec(),
                axis,
            })
        })
        .collect()
}

/// Inclusive civil-date range with the timetable's cancellation context.
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
    calendar_between_with_options(workspace, school, from, to, include_personal, true)
}

pub(super) fn calendar_between_with_options(
    workspace: &Value,
    school: &SchoolReads,
    from: &str,
    to: &str,
    include_personal: bool,
    include_cancelled: bool,
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
                include_cancelled,
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
        let state_label = state
            .as_ref()
            .and_then(|row| text(row, "label"))
            .filter(|label| !label.is_empty());
        let mut notices: Vec<_> = state_label
            .into_iter()
            .chain(important_dates.iter().filter_map(|row| text(row, "label")))
            .filter(|label| !label.is_empty())
            .collect();
        let occurrences = finalize_agenda(blocks, &periods)?;
        // Exams and timetable classes still happen when the weekly pattern
        // stops; such a day must not read as a day without classes.
        let no_classes =
            weekly_classes_stop && !occurrences.iter().any(Occurrence::is_active_class);
        if notices.is_empty() && weekly_classes_stop {
            notices.push(match (exam_period, language == ContentLanguage::Ja) {
                (true, true) => "試験期間",
                (true, false) => "Exam period",
                (false, true) => "授業なし",
                (false, false) => "No classes",
            });
        }
        let calendar_notice = (!notices.is_empty()).then(|| notices.join(" · "));
        agendas.push(DatedAgenda {
            occurrences,
            date,
            calendar_state: state,
            important_dates,
            no_classes,
            calendar_notice,
            exam_period,
        });
    }
    Ok(agendas)
}
