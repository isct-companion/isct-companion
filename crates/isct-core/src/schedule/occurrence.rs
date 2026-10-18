use std::collections::HashSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::alternation::{self, Alternation, Resolution};
use super::calendar::{calendar_state, quarter_range};
use super::civil::weekday;
use super::colors::display_color;
use super::{
    add_days, array, parse_civil_date, text, visible, ScheduleError, ScheduleIntensiveDay,
};
use super::{clock_for_periods, teaching_periods, TeachingPeriod};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum OccurrenceKind {
    Course,
    Event,
    Plan,
    Exam,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct Occurrence {
    pub key: String,
    pub id: String,
    pub kind: OccurrenceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    pub date: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub room: String,
    pub start_time: String,
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_with: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overridden: Option<bool>,
    /// Set on 物理学演習・実験 blocks, which meet every other week.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternation: Option<Alternation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScheduleProblemKind {
    CourseMeeting,
    Plan,
    ScheduleOverride,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScheduleProblem {
    pub kind: ScheduleProblemKind,
    pub id: String,
    pub title: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct AgendaOptions {
    pub include_personal: bool,
    pub include_cancelled_courses: bool,
    pub courses: Option<Vec<Value>>,
    pub events: Option<Vec<Value>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlanningDay {
    pub day: u8,
    pub blocks: Vec<Occurrence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlanningWeek {
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub courses: Vec<Value>,
    pub days: Vec<PlanningDay>,
    pub conflict_count: usize,
    /// Every clash, once per course involved, so a shell can explain a
    /// red block without re-deriving overlaps.
    pub conflicts: Vec<CourseConflict>,
}

/// One meeting of a course that overlaps another block in the quarter template.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseConflict {
    pub course_id: String,
    /// 1 = Monday … 7 = Sunday.
    pub day: u8,
    /// This course's meeting, not the overlap window.
    pub start_time: String,
    pub end_time: String,
    /// The other course; absent when the other block is a personal event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with_course_id: Option<String>,
    pub with_title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PlanCivilStart {
    pub date: String,
    pub time: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CourseTimeline {
    pub last_started: Option<String>,
    pub next: Option<String>,
    pub class_at: Option<String>,
}

impl Default for AgendaOptions {
    fn default() -> Self {
        Self {
            include_personal: true,
            include_cancelled_courses: false,
            courses: None,
            events: None,
        }
    }
}

pub fn planning_week(workspace: &Value, term: &str) -> Result<PlanningWeek, ScheduleError> {
    super::Term::parse(term)?;
    let teaching_periods = teaching_periods(workspace)?;
    let courses = scheduled_courses(workspace, term);
    let events: Vec<_> = array(workspace, "events")
        .iter()
        .filter(|row| {
            visible(row) && text(row, "termId") == Some(term) && text(row, "date").is_none()
        })
        .collect();
    let mut days = Vec::new();
    let mut conflict_count = 0;
    let mut conflicts = Vec::new();
    for day in 1..=7_u8 {
        let date = add_days("2000-01-02", i64::from(day))?;
        let mut blocks = Vec::new();
        for course in &courses {
            let id = text(course, "id").unwrap_or_default();
            for (index, meeting) in course
                .get("meetings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if !meeting
                    .get("days")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_u64() == Some(u64::from(day)))
                    })
                {
                    continue;
                }
                let Ok((start, end)) = meeting_clock(meeting, &teaching_periods) else {
                    continue;
                };
                let mut block = course_occurrence(
                    course,
                    format!("{id}-{index}-{day}"),
                    &date,
                    text(meeting, "room").unwrap_or_default(),
                    &start,
                    &end,
                    None,
                );
                block.alternation = alternation::template(workspace, course, term);
                blocks.push(block);
            }
        }
        for event in &events {
            if !event
                .get("days")
                .and_then(Value::as_array)
                .is_some_and(|values| {
                    values
                        .iter()
                        .any(|value| value.as_u64() == Some(u64::from(day)))
                })
            {
                continue;
            }
            let id = text(event, "id").unwrap_or_default();
            blocks.push(Occurrence {
                key: format!("{id}-{day}"),
                id: id.into(),
                kind: OccurrenceKind::Event,
                term_id: None,
                course_id: None,
                date: date.clone(),
                title: text(event, "title").unwrap_or_default().into(),
                subtitle: None,
                room: text(event, "room").unwrap_or_default().into(),
                start_time: text(event, "startTime").unwrap_or_default().into(),
                end_time: text(event, "endTime").unwrap_or_default().into(),
                end_date: None,
                color: "#8a887f".into(),
                conflict: None,
                dim: None,
                cancelled: None,
                time_label: None,
                conflict_with: None,
                overridden: None,
                alternation: None,
            });
        }
        for left in 0..blocks.len() {
            for right in left + 1..blocks.len() {
                if !overlap(&blocks[left], &blocks[right])? {
                    continue;
                }
                conflict_count += 1;
                for (own, other) in [(left, right), (right, left)] {
                    let (own, other) = (&blocks[own], &blocks[other]);
                    let Some(course_id) = own
                        .course_id
                        .clone()
                        .filter(|_| own.kind == OccurrenceKind::Course)
                    else {
                        continue;
                    };
                    if other.course_id.as_ref() == Some(&course_id) {
                        continue;
                    }
                    conflicts.push(CourseConflict {
                        course_id,
                        day,
                        start_time: own.start_time.clone(),
                        end_time: own.end_time.clone(),
                        with_course_id: other.course_id.clone(),
                        with_title: other.title.clone(),
                    });
                }
            }
        }
        mark_conflicts(&mut blocks)?;
        blocks.sort_by(|left, right| {
            left.start_time
                .cmp(&right.start_time)
                .then(left.key.cmp(&right.key))
        });
        days.push(PlanningDay { day, blocks });
    }
    Ok(PlanningWeek {
        courses,
        days,
        conflict_count,
        conflicts,
    })
}

pub fn plan_start(plan: &Value) -> Result<PlanCivilStart, ScheduleError> {
    let (date, time) = plan_civil_start(text(plan, "start").ok_or_else(|| {
        ScheduleError::InvalidInput("Plan start must be an ISO date or date-time.".into())
    })?)?;
    Ok(PlanCivilStart { date, time })
}

/// The course's meetings around `now` across every quarter it belongs to, so a
/// 3Q–4Q course keeps its class times when the quarter changes.
pub fn course_timeline(
    workspace: &Value,
    course_id: &str,
    now: DateTime<FixedOffset>,
    before_days: u16,
    after_days: u16,
) -> Result<CourseTimeline, ScheduleError> {
    if before_days > 366 || after_days > 366 {
        return Err(ScheduleError::InvalidInput(
            "course timeline range is too long".into(),
        ));
    }
    let today = parse_civil_date(&super::tokyo_date(now))?;
    let jst = FixedOffset::east_opt(9 * 3600)
        .ok_or_else(|| ScheduleError::Compatibility("JST offset unavailable".into()))?;
    let local_now = now.with_timezone(&jst).format("%Y-%m-%dT%H:%M").to_string();
    let terms = array(workspace, "courses")
        .iter()
        .find(|row| visible(row) && text(row, "id") == Some(course_id))
        .map(|course| course_terms(workspace, course))
        .unwrap_or_default();
    let mut starts = Vec::new();
    for offset in -(i64::from(before_days))..=i64::from(after_days) {
        let date = (today + Duration::days(offset)).to_string();
        for term in &terms {
            for row in resolve_day_agenda(
                workspace,
                term,
                &date,
                AgendaOptions {
                    include_personal: false,
                    ..AgendaOptions::default()
                },
            )?
            .into_iter()
            .filter(|row| row.course_id.as_deref() == Some(course_id))
            {
                starts.push(format!("{}T{}", row.date, row.start_time));
            }
        }
    }
    starts.sort();
    starts.dedup();
    let last_started = starts
        .iter()
        .rev()
        .find(|value| value.as_str() <= local_now.as_str())
        .map(|value| format!("{value}:00+09:00"));
    let next = starts
        .iter()
        .find(|value| value.as_str() > local_now.as_str())
        .map(|value| format!("{value}:00+09:00"));
    let class_at = last_started
        .as_ref()
        .filter(|value| value.starts_with(&today.to_string()))
        .cloned()
        .or_else(|| next.clone())
        .or_else(|| last_started.clone());
    Ok(CourseTimeline {
        last_started,
        next,
        class_at,
    })
}

/// Live memberships and plan items: the rows that put a course on the
/// timetable of their quarter.
fn placements(workspace: &Value) -> impl Iterator<Item = &Value> {
    array(workspace, "memberships")
        .iter()
        .chain(array(workspace, "planItems"))
        .filter(|row| visible(row))
}

/// Courses on the timetable of `term`: those with a live membership or plan
/// item there. A course running 3Q–4Q is one course (its `termId` is where it
/// was added) with a row in each quarter.
pub fn scheduled_courses(workspace: &Value, term: &str) -> Vec<Value> {
    let clock = teaching_periods(workspace).unwrap_or_default();
    let ids: HashSet<&str> = placements(workspace)
        .filter(|row| text(row, "termId") == Some(term))
        .filter_map(|row| text(row, "courseId"))
        .collect();
    array(workspace, "courses")
        .iter()
        .filter(|row| visible(row) && text(row, "id").is_some_and(|id| ids.contains(id)))
        .cloned()
        .map(|mut course| {
            if let Some(meetings) = course.get_mut("meetings").and_then(Value::as_array_mut) {
                meetings.retain(|meeting| {
                    meeting
                        .get("termIds")
                        .and_then(Value::as_array)
                        .is_none_or(|terms| terms.iter().any(|t| t.as_str() == Some(term)))
                });
                // Imported summaries can contain both a full meeting and a
                // contained slot (5–8 plus 7–8). One course cannot clash with
                // its own duplicate; retain the covering slot for each day.
                let mut normalized = Vec::<Value>::new();
                for meeting in meetings.iter() {
                    let covers = |a: &Value, b: &Value| {
                        a["days"] == b["days"]
                            && a["room"] == b["room"]
                            && a["termIds"] == b["termIds"]
                            && meeting_clock(a, &clock)
                                .ok()
                                .zip(meeting_clock(b, &clock).ok())
                                .is_some_and(|((start_a, end_a), (start_b, end_b))| {
                                    start_a <= start_b && end_a >= end_b
                                })
                    };
                    if normalized.iter().any(|existing| covers(existing, meeting)) {
                        continue;
                    }
                    normalized.retain(|existing| !covers(meeting, existing));
                    normalized.push(meeting.clone());
                }
                *meetings = normalized;
            }
            course
        })
        .collect()
}

/// Every quarter a course belongs to, sorted: its own `termId` and each
/// quarter a live membership or plan item places it in.
pub fn course_terms(workspace: &Value, course: &Value) -> Vec<String> {
    let id = text(course, "id");
    let mut terms: Vec<String> = text(course, "termId")
        .into_iter()
        .chain(
            placements(workspace)
                .filter(|row| id.is_some() && text(row, "courseId") == id)
                .filter_map(|row| text(row, "termId")),
        )
        .map(str::to_owned)
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

/// Visible courses that belong to `term` (see [`course_terms`]), whether or
/// not they are on its timetable.
pub fn term_courses<'a>(workspace: &'a Value, term: &str) -> Vec<&'a Value> {
    let placed: HashSet<&str> = placements(workspace)
        .filter(|row| text(row, "termId") == Some(term))
        .filter_map(|row| text(row, "courseId"))
        .collect();
    array(workspace, "courses")
        .iter()
        .filter(|row| {
            visible(row)
                && (text(row, "termId") == Some(term)
                    || text(row, "id").is_some_and(|id| placed.contains(id)))
        })
        .collect()
}

/// Every quarter that has a visible course or a live placement.
pub(crate) fn placement_terms(workspace: &Value) -> impl Iterator<Item = &str> {
    array(workspace, "courses")
        .iter()
        .filter(|row| visible(row))
        .chain(placements(workspace))
        .filter_map(|row| text(row, "termId"))
}

pub fn occurrences_on_date(
    workspace: &Value,
    term: &str,
    date: &str,
    options: AgendaOptions,
) -> Result<Vec<Occurrence>, ScheduleError> {
    let teaching_periods = teaching_periods(workspace)?;
    let day = weekday(date)?;
    let state = calendar_state(workspace, date);
    let state_kind = state.as_ref().and_then(|row| text(row, "type"));
    let closed = matches!(state_kind, Some("holiday" | "no-class" | "break"));
    let teaching_day = if state_kind == Some("replacement") {
        state
            .as_ref()
            .and_then(|row| row.get("replacementDay"))
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(day)
    } else if closed && !options.include_cancelled_courses {
        0
    } else {
        day
    };
    let range = quarter_range(workspace, term)?;
    let in_quarter = range
        .as_ref()
        .is_some_and(|range| date >= range.from.as_str() && date <= range.to.as_str());
    let outside_teaching = range
        .as_ref()
        .and_then(|range| range.teaching_ranges.as_ref())
        .is_some_and(|ranges| {
            state_kind != Some("replacement")
                && !ranges
                    .iter()
                    .any(|range| date >= range.start.as_str() && date <= range.end.as_str())
        });
    let courses = options
        .courses
        .unwrap_or_else(|| scheduled_courses(workspace, term));
    let mut blocks = Vec::new();
    if in_quarter {
        for course in courses.iter().filter(|row| visible(row)) {
            let course_id = text(course, "id").unwrap_or_default();
            for (index, meeting) in course
                .get("meetings")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .enumerate()
            {
                if text(meeting, "validFrom").is_some_and(|from| date < from)
                    || text(meeting, "validTo").is_some_and(|to| date > to)
                {
                    continue;
                }
                let Ok((start, end)) = meeting_clock(meeting, &teaching_periods) else {
                    continue;
                };
                let on = |weekday: u8| {
                    meeting
                        .get("days")
                        .and_then(Value::as_array)
                        .is_some_and(|days| {
                            days.iter()
                                .any(|value| value.as_u64() == Some(u64::from(weekday)))
                        })
                };
                let marker = match alternation::resolve(
                    workspace,
                    course,
                    term,
                    date,
                    options.include_cancelled_courses,
                ) {
                    Resolution::Weekly(marker) => marker,
                    Resolution::Hide => continue,
                    // A published session is dated; the teaching ranges do
                    // not apply to it.
                    Resolution::Show {
                        alternation,
                        cancelled,
                    } => {
                        if on(day) {
                            let mut block = course_occurrence(
                                course,
                                format!("{course_id}-weekly-{index}-{date}"),
                                date,
                                text(meeting, "room").unwrap_or_default(),
                                &start,
                                &end,
                                cancelled.then_some(true),
                            );
                            block.alternation = Some(alternation);
                            blocks.push(block);
                        }
                        continue;
                    }
                };
                if (outside_teaching && !options.include_cancelled_courses) || !on(teaching_day) {
                    continue;
                }
                let mut block = course_occurrence(
                    course,
                    format!("{course_id}-weekly-{index}-{date}"),
                    date,
                    text(meeting, "room").unwrap_or_default(),
                    &start,
                    &end,
                    (closed || outside_teaching).then_some(true),
                );
                block.alternation = marker;
                blocks.push(block);
            }
            for intensive in array(workspace, "intensiveSchedules").iter().filter(|row| {
                visible(row)
                    && text(row, "courseId") == Some(course_id)
                    && text(row, "from").is_some_and(|from| date >= from)
                    && text(row, "to").is_some_and(|to| date <= to)
            }) {
                let Some(selected) = intensive
                    .get("days")
                    .and_then(Value::as_array)
                    .and_then(|days| days.iter().find(|row| text(row, "date") == Some(date)))
                else {
                    continue;
                };
                let Some(periods) = periods(selected.get("periods")) else {
                    continue;
                };
                let (start, end) = clock_for_periods(&teaching_periods, periods)?;
                blocks.push(course_occurrence(
                    course,
                    format!("{}-{date}", text(intensive, "id").unwrap_or_default()),
                    date,
                    text(intensive, "room").unwrap_or_default(),
                    start,
                    end,
                    None,
                ));
            }
        }
    }
    if options.include_personal {
        let events = options.events.unwrap_or_else(|| {
            array(workspace, "events")
                .iter()
                .filter(|row| text(row, "termId") == Some(term))
                .cloned()
                .collect()
        });
        blocks.extend(personal_occurrences(workspace, date, &events)?);
    }
    mark_conflicts(&mut blocks)?;
    blocks.sort_by(|a, b| a.start_time.cmp(&b.start_time).then(a.key.cmp(&b.key)));
    Ok(blocks)
}

/// Personal items are resolved once per civil date, independently of course terms.
pub(crate) fn personal_occurrences(
    workspace: &Value,
    date: &str,
    events: &[Value],
) -> Result<Vec<Occurrence>, ScheduleError> {
    let day = weekday(date)?;
    let mut blocks = Vec::new();
    for event in events.iter().filter(|row| visible(row)) {
        let occurs = text(event, "date").map_or_else(
            || {
                event
                    .get("days")
                    .and_then(Value::as_array)
                    .is_some_and(|days| days.iter().any(|v| v.as_u64() == Some(u64::from(day))))
            },
            |value| value == date,
        );
        if !occurs {
            continue;
        }
        let id = text(event, "id").unwrap_or_default();
        blocks.push(Occurrence {
            key: format!("{id}-{date}"),
            id: id.into(),
            kind: OccurrenceKind::Event,
            term_id: None,
            course_id: None,
            date: date.into(),
            title: text(event, "title").unwrap_or_default().into(),
            subtitle: None,
            room: text(event, "room").unwrap_or_default().into(),
            start_time: text(event, "startTime").unwrap_or_default().into(),
            end_time: text(event, "endTime").unwrap_or_default().into(),
            end_date: None,
            color: "#8a887f".into(),
            conflict: None,
            dim: None,
            cancelled: None,
            time_label: None,
            conflict_with: None,
            overridden: None,
            alternation: None,
        });
    }
    for plan in array(workspace, "plans").iter().filter(|row| visible(row)) {
        // A stored plan that cannot be evaluated is reported by
        // `schedule_problems`; it must not hide every other block.
        if let Ok(Some(block)) = plan_occurrence(plan, date) {
            blocks.push(block);
        }
    }
    Ok(blocks)
}

fn plan_occurrence(plan: &Value, date: &str) -> Result<Option<Occurrence>, ScheduleError> {
    let start = plan_civil_start(text(plan, "start").unwrap_or_default())?;
    let exdates: Vec<String> = plan
        .get("exdates")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let excluded = exdates
        .iter()
        .any(|value| plan_civil_start(value).is_ok_and(|start| start.0 == date));
    let occurs = if excluded {
        false
    } else if let Some(rule) = text(plan, "rrule") {
        let recurrence_start = format!("{}T{}:00+09:00", start.0, start.1);
        crate::plans::occurrences_between(
            &recurrence_start,
            rule,
            &exdates,
            parse_civil_date(date)?,
            parse_civil_date(date)?,
        )
        .map_err(|e| ScheduleError::InvalidInput(e.to_string()))?
        .into_iter()
        .any(|v| v.starts_with(date))
    } else {
        start.0 == date
    };
    if !occurs {
        return Ok(None);
    }
    let duration = plan.get("duration").and_then(Value::as_i64).unwrap_or(0);
    let start_clock =
        NaiveDateTime::parse_from_str(&format!("{date}T{}:00", start.1), "%Y-%m-%dT%H:%M:%S")
            .map_err(|_| {
                ScheduleError::InvalidInput("Plan start must be an ISO date or date-time.".into())
            })?;
    let end = start_clock + Duration::minutes(duration);
    let id = text(plan, "id").unwrap_or_default();
    Ok(Some(Occurrence {
        key: format!("{id}-{date}"),
        id: id.into(),
        kind: OccurrenceKind::Plan,
        term_id: None,
        course_id: text(plan, "courseId").map(str::to_owned),
        date: date.into(),
        title: text(plan, "title").unwrap_or_default().into(),
        subtitle: None,
        room: text(plan, "room").unwrap_or_default().into(),
        start_time: start.1,
        end_time: end.format("%H:%M").to_string(),
        end_date: (end.date() != start_clock.date()).then(|| end.date().to_string()),
        color: "#5f6b8a".into(),
        conflict: None,
        dim: None,
        cancelled: None,
        time_label: None,
        conflict_with: None,
        overridden: None,
        alternation: None,
    }))
}

/// Stored user records the schedule skips or leaves unapplied. Read models
/// stay available; shells list these so the person can repair or delete them.
pub fn schedule_problems(workspace: &Value) -> Vec<ScheduleProblem> {
    let mut problems = Vec::new();
    if let Ok(teaching_periods) = teaching_periods(workspace) {
        for course in array(workspace, "courses")
            .iter()
            .filter(|row| visible(row))
        {
            for (index, meeting) in course
                .get("meetings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Err(error) = meeting_clock(meeting, &teaching_periods) {
                    problems.push(ScheduleProblem {
                        kind: ScheduleProblemKind::CourseMeeting,
                        id: format!("{}:{index}", text(course, "id").unwrap_or_default()),
                        title: text(course, "title").unwrap_or_default().into(),
                        message: error.to_string(),
                    });
                }
            }
        }
    }
    for plan in array(workspace, "plans").iter().filter(|row| visible(row)) {
        if let Err(error) = crate::plans::validate_plan_recurrence(
            text(plan, "start").unwrap_or_default(),
            text(plan, "rrule"),
        ) {
            problems.push(ScheduleProblem {
                kind: ScheduleProblemKind::Plan,
                id: text(plan, "id").unwrap_or_default().into(),
                title: text(plan, "title").unwrap_or_default().into(),
                message: error.to_string(),
            });
        }
    }
    for row in array(workspace, "scheduleOverrides")
        .iter()
        .filter(|row| visible(row))
    {
        if let Some(value) = row.get("periods").filter(|value| !value.is_null()) {
            let validation = periods(Some(value))
                .ok_or_else(|| {
                    ScheduleError::InvalidInput(
                        "Choose periods from 1 to 12 in ascending order.".into(),
                    )
                })
                .and_then(super::validate_period_range);
            if let Err(error) = validation {
                problems.push(ScheduleProblem {
                    kind: ScheduleProblemKind::ScheduleOverride,
                    id: text(row, "id").unwrap_or_default().into(),
                    title: text(row, "title").unwrap_or_default().into(),
                    message: error.to_string(),
                });
            }
        }
    }
    problems
}

fn course_occurrence(
    course: &Value,
    key: String,
    date: &str,
    room: &str,
    start: &str,
    end: &str,
    cancelled: Option<bool>,
) -> Occurrence {
    let id = text(course, "id").unwrap_or_default();
    Occurrence {
        key,
        id: id.into(),
        kind: OccurrenceKind::Course,
        term_id: None,
        course_id: Some(id.into()),
        date: date.into(),
        title: course
            .pointer("/overrides/title")
            .and_then(Value::as_str)
            .or_else(|| text(course, "title"))
            .unwrap_or_default()
            .into(),
        subtitle: text(course, "code").map(str::to_owned),
        room: room.into(),
        start_time: start.into(),
        end_time: end.into(),
        end_date: None,
        color: display_color(course),
        conflict: None,
        dim: Some(false),
        cancelled,
        time_label: None,
        conflict_with: None,
        overridden: None,
        alternation: None,
    }
}

fn mark_conflicts(blocks: &mut [Occurrence]) -> Result<(), ScheduleError> {
    for i in 0..blocks.len() {
        for j in i + 1..blocks.len() {
            if blocks[i].cancelled == Some(true) || blocks[j].cancelled == Some(true) {
                continue;
            }
            if overlap(&blocks[i], &blocks[j])? {
                blocks[i].conflict = Some(true);
                blocks[j].conflict = Some(true);
                // Name the other block so a red block says what it clashes with.
                if blocks[i].course_id.is_none() || blocks[i].course_id != blocks[j].course_id {
                    let (left, right) = (blocks[i].title.clone(), blocks[j].title.clone());
                    for (block, name) in [(i, right), (j, left)] {
                        let names = blocks[block].conflict_with.get_or_insert_with(Vec::new);
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
fn overlap(a: &Occurrence, b: &Occurrence) -> Result<bool, ScheduleError> {
    let end_a = format!(
        "{}T{}",
        a.end_date.clone().unwrap_or(if a.end_time <= a.start_time {
            add_days(&a.date, 1)?
        } else {
            a.date.clone()
        }),
        a.end_time
    );
    let end_b = format!(
        "{}T{}",
        b.end_date.clone().unwrap_or(if b.end_time <= b.start_time {
            add_days(&b.date, 1)?
        } else {
            b.date.clone()
        }),
        b.end_time
    );
    Ok(format!("{}T{}", a.date, a.start_time) < end_b
        && format!("{}T{}", b.date, b.start_time) < end_a)
}

pub fn resolve_day_agenda(
    workspace: &Value,
    term: &str,
    date: &str,
    options: AgendaOptions,
) -> Result<Vec<Occurrence>, ScheduleError> {
    let teaching_periods = teaching_periods(workspace)?;
    let mut blocks = occurrences_on_date(workspace, term, date, options.clone())?;
    let overrides: Vec<_> = array(workspace, "scheduleOverrides")
        .iter()
        .filter(|r| visible(r))
        .collect();
    blocks = blocks
        .into_iter()
        .filter_map(|mut block| {
            let change = overrides
                .iter()
                .find(|r| text(r, "targetKey") == Some(&block.key));
            match change {
                Some(r) if r.get("hidden").and_then(Value::as_bool) == Some(true) => None,
                Some(r) => {
                    // An override Core cannot apply leaves the class as
                    // scheduled; `schedule_problems` reports the record.
                    if apply_override(&mut block, r, &teaching_periods).is_err() {
                        return Some(block);
                    }
                    (block.date == date).then_some(block)
                }
                None => Some(block),
            }
        })
        .collect();
    for change in overrides.iter().filter(|r| {
        r.get("hidden").and_then(Value::as_bool) != Some(true)
            && text(r, "targetKind") == Some("course")
            && text(r, "date") == Some(date)
    }) {
        let key = text(change, "targetKey").unwrap_or_default();
        if blocks.iter().any(|b| b.key == key) {
            continue;
        }
        let Some(source) = key
            .get(key.len().saturating_sub(10)..)
            .filter(|v| parse_civil_date(v).is_ok())
        else {
            continue;
        };
        if source == date {
            continue;
        }
        if let Some(mut original) = occurrences_on_date(workspace, term, source, options.clone())?
            .into_iter()
            .find(|b| b.key == key)
        {
            if apply_override(&mut original, change, &teaching_periods).is_ok() {
                blocks.push(original)
            }
        }
    }
    let mut exams = Vec::new();
    for exam in super::exam_views(workspace, term)
        .into_iter()
        .filter(|exam| !exam.hidden && exam.status == "scheduled" && exam.date == date)
    {
        let (Some(id), Some(start), Some(end)) = (exam.course_id, exam.start_time, exam.end_time)
        else {
            continue;
        };
        exams.push(Occurrence {
            key: exam.key,
            id: id.clone(),
            kind: OccurrenceKind::Exam,
            term_id: None,
            course_id: Some(id),
            date: exam.date,
            title: exam.title,
            subtitle: Some(exam.code),
            room: exam.room,
            time_label: Some(exam.periods.map_or_else(
                || format!("{start}–{end}"),
                |p| format!("{}–{}限", p[0], p[1]),
            )),
            start_time: start,
            end_time: end,
            end_date: None,
            color: exam.color,
            conflict: None,
            dim: None,
            cancelled: None,
            conflict_with: None,
            overridden: exam.edited.then_some(true),
            alternation: None,
        });
    }
    blocks.extend(exams);
    for block in &mut blocks {
        if matches!(block.kind, OccurrenceKind::Course | OccurrenceKind::Exam) {
            block.term_id = Some(term.into());
        }
    }
    finalize_agenda(blocks)
}

pub(crate) fn finalize_agenda(
    mut blocks: Vec<Occurrence>,
) -> Result<Vec<Occurrence>, ScheduleError> {
    let exam_ids: HashSet<_> = blocks
        .iter()
        .filter(|block| block.kind == OccurrenceKind::Exam)
        .filter_map(|e| e.course_id.clone())
        .collect();
    blocks.retain(|b| {
        b.kind != OccurrenceKind::Course
            || !b
                .course_id
                .as_deref()
                .is_some_and(|id| exam_ids.contains(id))
    });
    for block in &mut blocks {
        block.conflict = None;
        block.conflict_with = None;
    }
    mark_conflicts(&mut blocks)?;
    for i in 0..blocks.len() {
        if blocks[i].kind != OccurrenceKind::Exam {
            continue;
        }
        let collisions: Vec<_> = (0..blocks.len())
            .filter(|&j| {
                j != i
                    && blocks[j].cancelled != Some(true)
                    && blocks[j].course_id != blocks[i].course_id
                    && overlap(&blocks[i], &blocks[j]).unwrap_or(false)
            })
            .collect();
        let names: Vec<_> = collisions
            .into_iter()
            .map(|j| {
                blocks[j].conflict = Some(true);
                blocks[j].title.clone()
            })
            .collect();
        if !names.is_empty() {
            blocks[i].conflict = Some(true);
            blocks[i].conflict_with = Some(names)
        }
    }
    blocks.sort_by(|a, b| {
        a.start_time
            .cmp(&b.start_time)
            .then((a.kind == OccurrenceKind::Exam).cmp(&(b.kind == OccurrenceKind::Exam)))
            .then(a.key.cmp(&b.key))
    });
    Ok(blocks)
}

fn apply_override(
    block: &mut Occurrence,
    row: &Value,
    teaching_periods: &[TeachingPeriod],
) -> Result<(), ScheduleError> {
    let clock = override_clock(row, teaching_periods)?;
    if let Some(v) = text(row, "date") {
        block.date = v.into()
    }
    if let Some(v) = text(row, "title") {
        block.title = v.into()
    }
    if let Some(v) = text(row, "room") {
        block.room = v.into()
    }
    if let Some(v) = text(row, "courseId") {
        block.course_id = Some(v.into())
    }
    if let Some(v) = text(row, "startTime").or_else(|| clock.map(|v| v.0)) {
        block.start_time = v.into()
    }
    if let Some(v) = text(row, "endTime").or_else(|| clock.map(|v| v.1)) {
        block.end_time = v.into()
    }
    block.overridden = Some(true);
    Ok(())
}

fn override_clock<'a>(
    row: &Value,
    teaching_periods: &'a [TeachingPeriod],
) -> Result<Option<(&'a str, &'a str)>, ScheduleError> {
    match row.get("periods").filter(|value| !value.is_null()) {
        None => Ok(None),
        Some(value) => clock_for_periods(
            teaching_periods,
            periods(Some(value)).ok_or_else(|| {
                ScheduleError::InvalidInput(
                    "Choose periods from 1 to 12 in ascending order.".into(),
                )
            })?,
        )
        .map(Some),
    }
}

/// A `[first, last]` period pair; values outside `u8` are rejected, not wrapped.
fn periods(value: Option<&Value>) -> Option<[u8; 2]> {
    let values = value?.as_array()?;
    let period = |index: usize| u8::try_from(values.get(index)?.as_u64()?).ok();
    (values.len() == 2).then_some([period(0)?, period(1)?])
}

/// Period-backed meetings follow the currently selected teaching table.
/// An empty period list is the stored representation of an explicit custom
/// clock and is returned unchanged.
fn meeting_clock(
    meeting: &Value,
    teaching_periods: &[TeachingPeriod],
) -> Result<(String, String), ScheduleError> {
    let stored = || {
        let start = text(meeting, "startTime").unwrap_or_default();
        let end = text(meeting, "endTime").unwrap_or_default();
        let parsed = NaiveTime::parse_from_str(start, "%H:%M")
            .ok()
            .zip(NaiveTime::parse_from_str(end, "%H:%M").ok());
        if !parsed.is_some_and(|(start, end)| start < end) {
            return Err(ScheduleError::InvalidInput(
                "Meeting start and end times are invalid.".into(),
            ));
        }
        Ok((start.to_string(), end.to_string()))
    };
    let Some(values) = meeting.get("periods").and_then(Value::as_array) else {
        return stored();
    };
    if values.is_empty() {
        return stored();
    }
    let mut values = values.iter().map(|value| {
        value
            .as_u64()
            .and_then(|value| u8::try_from(value).ok())
            .ok_or_else(|| ScheduleError::InvalidInput("Meeting periods are invalid.".into()))
    });
    let first = values
        .next()
        .ok_or_else(|| ScheduleError::InvalidInput("Meeting periods are invalid.".into()))??;
    let mut last = first;
    for value in values {
        let value = value?;
        if value != last.saturating_add(1) {
            return Err(ScheduleError::InvalidInput(
                "Meeting periods must be a contiguous ascending range.".into(),
            ));
        }
        last = value;
    }
    let (start, end) = clock_for_periods(teaching_periods, [first, last])?;
    Ok((start.into(), end.into()))
}

fn plan_civil_start(raw: &str) -> Result<(String, String), ScheduleError> {
    let (date, time) = crate::plans::plan_civil_start(raw)
        .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    Ok((date.to_string(), time.format("%H:%M").to_string()))
}

pub fn next_plan_occurrences(plan: &Value, limit: u16) -> Result<Vec<String>, ScheduleError> {
    if limit == 0 {
        return Ok(vec![]);
    }
    let start = text(plan, "start").ok_or_else(|| {
        ScheduleError::InvalidInput("Plan start must be an ISO date or date-time.".into())
    })?;
    let exdates: Vec<String> = plan
        .get("exdates")
        .and_then(Value::as_array)
        .map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if let Some(rule) = text(plan, "rrule") {
        let civil_start = plan_civil_start(start)?;
        let recurrence_start = format!("{}T{}:00+09:00", civil_start.0, civil_start.1);
        let preview = crate::plans::preview_rrule(&recurrence_start, rule, &exdates, limit)
            .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
        return Ok(preview.occurrences);
    }
    let civil = plan_civil_start(start)?;
    if exdates
        .iter()
        .any(|v| plan_civil_start(v).is_ok_and(|p| p.0 == civil.0))
    {
        Ok(vec![])
    } else {
        Ok(vec![format!("{}T{}:00+09:00", civil.0, civil.1)])
    }
}

pub fn intensive_days(
    teaching_periods: &[TeachingPeriod],
    from: &str,
    to: &str,
    periods: [u8; 2],
) -> Result<Vec<ScheduleIntensiveDay>, ScheduleError> {
    clock_for_periods(teaching_periods, periods)?;
    let start = parse_civil_date(from)?;
    let end = parse_civil_date(to)?;
    let days = (end - start).num_days();
    if !(0..=366).contains(&days) {
        return Err(ScheduleError::InvalidInput(
            "Choose a valid intensive-course date range.".into(),
        ));
    }
    Ok((0..=days)
        .map(|offset| ScheduleIntensiveDay {
            date: (start + Duration::days(offset)).to_string(),
            periods,
        })
        .collect())
}

pub fn validate_academic_profile(profile: &Value) -> Result<Value, ScheduleError> {
    let input = profile
        .as_object()
        .ok_or_else(|| ScheduleError::InvalidInput("Academic profile must be an object".into()))?;
    let name = input
        .get("name")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ScheduleError::InvalidInput("Academic profile needs a name".into()))?
        .trim();
    if input.get("admissionYear").is_some_and(|v| {
        !v.is_null()
            && v.as_i64()
                .is_none_or(|y| !crate::data::ADMISSION_YEARS.contains(&y))
    }) {
        return Err(ScheduleError::InvalidInput(
            "Academic profile has an invalid admission year".into(),
        ));
    }
    let grade_path = required_array(
        input,
        "gradePath",
        "Academic profile needs a grade path array",
    )?;
    let mut grades = HashSet::new();
    let mut normalized_grades = Vec::new();
    for (index, row) in grade_path.iter().enumerate() {
        let object = row.as_object().ok_or_else(|| {
            ScheduleError::InvalidInput(format!("Grade path entry {} must be an object", index + 1))
        })?;
        let grade = object
            .get("grade")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Grade path entry {} needs a grade label",
                    index + 1
                ))
            })?
            .trim();
        let year = object
            .get("academicYear")
            .and_then(Value::as_i64)
            .filter(|v| *v >= 1900)
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Grade path entry {} has an invalid academic year",
                    index + 1
                ))
            })?;
        if !grades.insert(grade.to_owned()) {
            return Err(ScheduleError::InvalidInput(
                "Grade labels must be unique".into(),
            ));
        }
        normalized_grades.push(serde_json::json!({"grade":grade,"academicYear":year}));
    }
    let years = required_array(
        input,
        "years",
        "Academic profile needs a configured years array",
    )?;
    let mut configured = HashSet::new();
    let mut normalized_years = Vec::new();
    for (index, row) in years.iter().enumerate() {
        let object = row.as_object().ok_or_else(|| {
            ScheduleError::InvalidInput(format!("Configured year {} must be an object", index + 1))
        })?;
        let year = object
            .get("academicYear")
            .and_then(Value::as_i64)
            .filter(|v| *v >= 1900)
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Configured year {} has an invalid academic year",
                    index + 1
                ))
            })?;
        if !configured.insert(year) {
            return Err(ScheduleError::InvalidInput(
                "Configured academic years must be unique".into(),
            ));
        }
        let starts = object
            .get("quarterStarts")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                ScheduleError::InvalidInput(format!(
                    "Configured year {} needs quarter starts",
                    index + 1
                ))
            })?;
        let values: Vec<_> = ["1Q", "2Q", "3Q", "4Q"]
            .iter()
            .map(|q| starts.get(*q).and_then(Value::as_str))
            .collect();
        if values
            .iter()
            .any(|v| v.is_none_or(|v| parse_civil_date(v).is_err()))
        {
            return Err(ScheduleError::InvalidInput(format!(
                "Configured year {} needs valid starts for 1Q–4Q",
                index + 1
            )));
        }
        if values.windows(2).any(|w| w[1] <= w[0]) {
            return Err(ScheduleError::InvalidInput(format!(
                "Configured year {} quarter starts must be chronological",
                index + 1
            )));
        }
        normalized_years.push(serde_json::json!({"academicYear":year,"quarterStarts":starts}));
    }
    normalized_years.sort_by_key(|r| r.get("academicYear").and_then(Value::as_i64));
    let mut output = Map::new();
    output.insert("name".into(), Value::String(name.into()));
    if let Some(year) = input.get("admissionYear").filter(|v| !v.is_null()) {
        output.insert("admissionYear".into(), year.clone());
    }
    output.insert("gradePath".into(), Value::Array(normalized_grades));
    output.insert("years".into(), Value::Array(normalized_years));
    let periods: Option<Vec<TeachingPeriod>> =
        serde_json::from_value(input.get("teachingPeriods").cloned().unwrap_or(Value::Null))
            .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    if let Some(table) = &periods {
        super::validate_teaching_periods(table)?;
    }
    output.insert(
        "teachingPeriods".into(),
        serde_json::to_value(periods)
            .map_err(|error| ScheduleError::Compatibility(error.to_string()))?,
    );
    let mut enrollments: Vec<crate::study_profile::StudyEnrollment> = serde_json::from_value(
        input
            .get("studyEnrollments")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| Value::Array(vec![])),
    )
    .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    crate::study_profile::canonicalize_study_enrollments(&mut enrollments)
        .map_err(|error| ScheduleError::InvalidInput(error.to_string()))?;
    output.insert(
        "studyEnrollments".into(),
        serde_json::to_value(enrollments)
            .map_err(|error| ScheduleError::Compatibility(error.to_string()))?,
    );
    Ok(Value::Object(output))
}
fn required_array<'a>(
    input: &'a Map<String, Value>,
    key: &str,
    message: &str,
) -> Result<&'a Vec<Value>, ScheduleError> {
    input
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| ScheduleError::InvalidInput(message.into()))
}
