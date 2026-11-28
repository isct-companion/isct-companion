//! The browsable Today agenda, including row labels and their refresh time.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Datelike, Duration, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::tasks::{next_assignment_boundary, task_view, TaskView};
use super::today::{
    at, class_moment, jst, start_of_next_day, timed_classes, today_agenda_between, LOOKAHEAD_DAYS,
};
use super::ExamView;
use super::{
    add_days, parse_civil_date, teaching_periods, tokyo_date, Occurrence, ScheduleError,
    SchoolReads,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TodayAgenda {
    pub date: String,
    pub today: String,
    pub week: Vec<AgendaDate>,
    pub rows: Vec<AgendaRow>,
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub important_dates: Vec<Value>,
    pub calendar_notice: Option<String>,
    pub day_kind: AgendaDayKind,
    pub next_class: Option<Occurrence>,
    pub exams: Vec<ExamView>,
    pub tasks: TaskView,
    pub next_boundary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgendaDate {
    pub date: String,
    pub has_items: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AgendaRow {
    pub occurrence: Occurrence,
    pub status: Option<AgendaStatus>,
    pub highlighted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AgendaStatus {
    Earlier,
    Now,
    Next,
    Later,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AgendaDayKind {
    TeachingDay,
    Weekend,
    ExamPeriod,
    NoClasses,
}

pub(super) fn read(
    workspace: &Value,
    school: &SchoolReads,
    date: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<TodayAgenda, ScheduleError> {
    let now = now.with_timezone(&jst()?);
    let today = tokyo_date(now);
    let date = date.unwrap_or(&today);
    let is_today = date == today;
    let civil = parse_civil_date(date)?;
    let weekday = civil.weekday().num_days_from_monday();
    let monday = (civil - Duration::days(i64::from(weekday))).to_string();
    let last = add_days(date, LOOKAHEAD_DAYS)?;
    let days = today_agenda_between(workspace, school, &monday, &last)?;
    let week = days[..7]
        .iter()
        .map(|day| AgendaDate {
            date: day.date.clone(),
            has_items: !day.occurrences.is_empty(),
        })
        .collect();
    let (day, next_days) = days
        .split_at(weekday as usize)
        .1
        .split_first()
        .ok_or_else(|| ScheduleError::Compatibility("missing selected date in agenda".into()))?;
    let periods = teaching_periods(workspace)?;
    let classes = timed_classes(day, &periods)?;
    let later = next_days
        .iter()
        .map(|day| timed_classes(day, &periods))
        .collect::<Result<Vec<_>, _>>()?;
    let selected_time = if is_today { now } else { at(date, "00:00")? };
    let moment = class_moment(selected_time, &classes, &later);
    let next = moment.upcoming.first().map(|item| &item.occurrence);
    let current = moment.current.as_ref().map(|item| &item.occurrence);
    let highlighted = current.or(next).map(|row| &row.key);
    let mut boundary = start_of_next_day(&today)?;
    let rows = day
        .occurrences
        .iter()
        .map(|row| {
            let status = if is_today {
                let start = at(&row.date, &row.start_time)?;
                let end = at(row.end_date.as_deref().unwrap_or(&row.date), &row.end_time)?;
                for at in [start, end].into_iter().filter(|at| *at > now) {
                    boundary = boundary.min(at);
                }
                Some(if end <= now {
                    AgendaStatus::Earlier
                } else if start <= now {
                    AgendaStatus::Now
                } else if next.is_some_and(|next| next.key == row.key) {
                    AgendaStatus::Next
                } else {
                    AgendaStatus::Later
                })
            } else {
                None
            };
            Ok(AgendaRow {
                occurrence: row.clone(),
                status,
                highlighted: is_today && highlighted == Some(&row.key),
            })
        })
        .collect::<Result<_, ScheduleError>>()?;
    let exams: Vec<_> = super::exams::today_exams(workspace, school, now, date.max(&today))?
        .into_iter()
        .take(3)
        .map(|(exam, end)| {
            boundary = boundary.min(end);
            exam
        })
        .collect();
    if let Some(at) = next_assignment_boundary(workspace, now) {
        boundary = boundary.min(at);
    }
    let next_class = next.filter(|row| row.date.as_str() > date).cloned();
    Ok(TodayAgenda {
        date: date.into(),
        today,
        week,
        rows,
        important_dates: day.important_dates.clone(),
        calendar_notice: day
            .calendar_state
            .as_ref()
            .and_then(|state| super::text(state, "label"))
            .filter(|label| !label.is_empty())
            .map(str::to_owned),
        day_kind: if weekday >= 5 {
            AgendaDayKind::Weekend
        } else if day.exam_period {
            AgendaDayKind::ExamPeriod
        } else if day.no_classes {
            AgendaDayKind::NoClasses
        } else {
            AgendaDayKind::TeachingDay
        },
        next_class,
        exams,
        tasks: task_view(workspace, now)?,
        next_boundary: boundary.to_rfc3339(),
    })
}
