use super::SchoolReads;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::agenda_layout::finalize_agenda;
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
    Exam,
}

/// What one session is, as `MarkSession` sets it: a normal class, an exam, a
/// quiz (小テスト) or cancelled (休講).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum SessionMark {
    Class,
    Exam,
    Quiz,
    Cancelled,
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
    /// A class or exam of a course only planned for the block's quarter: no
    /// membership there, so it is drawn as an outline. Judged against the
    /// block's own quarter, so a range across quarters reads each correctly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned: Option<bool>,
    /// An exam block the user marked as a quiz (小テスト) held in class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiz: Option<bool>,
    /// The published exam changed after the user corrected it, or left the
    /// exam timetable; the block keeps the user's values until they choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_changed: Option<bool>,
    /// Where the block sits on the period grid, when that differs from its
    /// own times or it shares the time with another block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid: Option<GridSlot>,
    /// The session's current mark, for the menu that changes it; none on a
    /// block the person cannot mark: an event, or a planned course's session
    /// that has no mark to clear (it has no sessions of its own yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<SessionMark>,
}

/// A timetable block's place on the period grid. A block whose times fall
/// inside periods (an exam at 09:30–10:20) fills the periods it touches, and
/// blocks that overlap sit side by side; the block keeps its own times for
/// its label.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct GridSlot {
    pub start_time: String,
    pub end_time: String,
    /// The block's column among the blocks it overlaps, from 0 on the left.
    pub lane: u8,
    /// How many columns the overlapping blocks share; 1 when it is alone.
    pub lanes: u8,
}

impl Occurrence {
    /// Its course, or both courses of a shared 演習・実験 block.
    pub fn course_ids(&self) -> impl Iterator<Item = &str> {
        let pair = self
            .alternation
            .as_ref()
            .and_then(|row| row.pair.as_deref())
            .unwrap_or_default();
        self.course_id
            .as_deref()
            .into_iter()
            .filter(move |_| pair.is_empty())
            .chain(pair.iter().map(|course| course.course_id.as_str()))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ScheduleProblemKind {
    CourseMeeting,
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
    pub courses: Option<Arc<[Value]>>,
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
    /// The latest earlier quarter with weekly events, offered while this
    /// quarter has none of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub carry_over: Option<EventCarryOver>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EventCarryOver {
    pub term_id: String,
    pub count: usize,
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
        .filter(|row| visible(row) && text(row, "termId") == Some(term))
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
        alternation::pair_blocks(&mut blocks)?;
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
                color: super::event_color(event),
                conflict: None,
                dim: None,
                cancelled: None,
                time_label: None,
                conflict_with: None,
                overridden: None,
                alternation: None,
                planned: None,
                quiz: None,
                source_changed: None,
                grid: None,
                mark: None,
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
    let carry_over = if events.is_empty() {
        let mut earlier = std::collections::BTreeMap::<&str, usize>::new();
        for event in array(workspace, "events").iter().filter(|row| visible(row)) {
            if let Some(from) = text(event, "termId").filter(|from| *from < term) {
                *earlier.entry(from).or_default() += 1;
            }
        }
        earlier
            .last_key_value()
            .map(|(from, count)| EventCarryOver {
                term_id: (*from).into(),
                count: *count,
            })
    } else {
        None
    };
    Ok(PlanningWeek {
        courses,
        days,
        conflict_count,
        conflicts,
        carry_over,
    })
}

/// The course's meetings around `now` across every quarter it belongs to, so a
/// 3Q–4Q course keeps its class times when the quarter changes.
pub fn course_timeline(
    workspace: &Value,
    school: &SchoolReads,
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
                school,
                term,
                &date,
                AgendaOptions {
                    include_personal: false,
                    ..AgendaOptions::default()
                },
            )?
            .into_iter()
            .filter(|row| row.course_ids().any(|id| id == course_id))
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

/// The course sources and single-session changes used by every dated read of a term.
pub(super) struct CourseSessions<'a> {
    workspace: &'a Value,
    pub(super) term: &'a str,
    courses: Arc<[Value]>,
    pub(super) range: Option<super::QuarterRange>,
    periods: Vec<TeachingPeriod>,
    changes: HashMap<&'a str, &'a Value>,
    moved: HashMap<&'a str, Vec<(&'a str, &'a Value)>>,
    enrolled: HashSet<&'a str>,
}

impl<'a> CourseSessions<'a> {
    pub(super) fn new(
        workspace: &'a Value,
        term: &'a str,
        courses: Option<Arc<[Value]>>,
    ) -> Result<Self, ScheduleError> {
        let changes: HashMap<_, _> = array(workspace, "scheduleOverrides")
            .iter()
            .filter(|row| visible(row) && text(row, "targetKind") == Some("course"))
            .filter_map(|row| Some((text(row, "targetKey")?, row)))
            .collect();
        let mut moved: HashMap<_, Vec<_>> = HashMap::new();
        for (&key, &row) in &changes {
            if let Some(date) = text(row, "date").filter(|date| Some(*date) != key_date(key)) {
                moved.entry(date).or_default().push((key, row));
            }
        }
        Ok(Self {
            workspace,
            term,
            courses: courses.unwrap_or_else(|| scheduled_courses(workspace, term).into()),
            range: quarter_range(workspace, term)?,
            periods: teaching_periods(workspace)?,
            changes,
            moved,
            enrolled: array(workspace, "memberships")
                .iter()
                .filter(|row| visible(row) && text(row, "termId") == Some(term))
                .filter_map(|row| text(row, "courseId"))
                .collect(),
        })
    }

    /// Resolve a stored key at its original date, even if that date is no
    /// longer a teaching day. Only an explicit change can use that source.
    pub(super) fn original(&self, key: &str) -> Result<Option<Occurrence>, ScheduleError> {
        let Some(date) = key_date(key) else {
            return Ok(None);
        };
        Ok(self.sources(date, Some(key))?.pop())
    }

    pub(super) fn edited(&self, mut block: Occurrence, change: &Value) -> Option<Occurrence> {
        if change.get("hidden").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        // An invalid correction leaves the class as scheduled.
        // `schedule_problems` reports the record.
        if apply_override(&mut block, change, &self.periods).is_ok() {
            apply_mark(&mut block, change);
        }
        Some(block)
    }

    fn sources(&self, date: &str, only: Option<&str>) -> Result<Vec<Occurrence>, ScheduleError> {
        let day = weekday(date)?;
        let Some(range) = self.range.as_ref().filter(|range| range.contains(date)) else {
            return Ok(Vec::new());
        };
        let state = calendar_state(self.workspace, date);
        let state_kind = state.as_ref().and_then(|row| text(row, "type"));
        let closed = matches!(state_kind, Some("holiday" | "no-class" | "break"));
        let replacement = state_kind == Some("replacement");
        let teaching_day = if replacement {
            state.as_ref().and_then(replacement_weekday).unwrap_or(day)
        } else {
            day
        };
        let outside_teaching = !replacement && range.outside_teaching(date);
        let mut blocks = Vec::new();
        for course in self.courses.iter().filter(|row| visible(row)) {
            let course_id = text(course, "id").unwrap_or_default();
            let alternation = alternation::resolve(self.workspace, course, self.term, date, true);
            for (index, meeting) in course
                .get("meetings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let key = format!("{course_id}-weekly-{index}-{date}");
                if only.is_some_and(|only| only != key)
                    || text(meeting, "validFrom").is_some_and(|from| date < from)
                    || text(meeting, "validTo").is_some_and(|to| date > to)
                {
                    continue;
                }
                let on = |weekday| {
                    meeting["days"].as_array().is_some_and(|days| {
                        days.iter().any(|v| v.as_u64() == Some(u64::from(weekday)))
                    })
                };
                let mark = self
                    .changes
                    .get(key.as_str())
                    .and_then(|row| text(row, "mark"));
                let (marker, cancelled) = match &alternation {
                    Resolution::Show {
                        alternation,
                        cancelled,
                    } if on(day) => (Some(alternation.clone()), *cancelled),
                    Resolution::Weekly(marker)
                        if (on(teaching_day) || mark == Some("class") && on(day))
                            && (!outside_teaching || only.is_some() || mark.is_some()) =>
                    {
                        (marker.clone(), closed || outside_teaching)
                    }
                    _ => continue,
                };
                let Ok((start, end)) = meeting_clock(meeting, &self.periods) else {
                    continue;
                };
                let mut block = course_occurrence(
                    course,
                    key,
                    date,
                    text(meeting, "room").unwrap_or_default(),
                    &start,
                    &end,
                    cancelled.then_some(true),
                );
                block.alternation = marker;
                blocks.push(block);
            }
            for intensive in array(self.workspace, "intensiveSchedules")
                .iter()
                .filter(|row| {
                    visible(row)
                        && text(row, "courseId") == Some(course_id)
                        && text(row, "from").is_some_and(|from| date >= from)
                        && text(row, "to").is_some_and(|to| date <= to)
                })
            {
                let key = format!("{}-{date}", text(intensive, "id").unwrap_or_default());
                if only.is_some_and(|only| only != key) {
                    continue;
                }
                let Some(periods) = intensive["days"]
                    .as_array()
                    .and_then(|days| days.iter().find(|row| text(row, "date") == Some(date)))
                    .and_then(|row| periods(row.get("periods")))
                else {
                    continue;
                };
                let (start, end) = clock_for_periods(&self.periods, periods)?;
                blocks.push(course_occurrence(
                    course,
                    key,
                    date,
                    text(intensive, "room").unwrap_or_default(),
                    start,
                    end,
                    None,
                ));
            }
        }
        Ok(blocks)
    }

    pub(super) fn blocks_on(
        &self,
        date: &str,
        published: &[super::ExamView],
        include_cancelled: bool,
    ) -> Result<Vec<Occurrence>, ScheduleError> {
        let mut blocks = Vec::new();
        for original in self.sources(date, None)? {
            let change = self.changes.get(original.key.as_str()).copied();
            let block = match change {
                Some(change) => self.edited(original, change),
                None => Some(original),
            };
            blocks.extend(block.filter(|block| block.date == date));
        }
        for &(key, change) in self.moved.get(date).into_iter().flatten() {
            if let Some(original) = self.original(key)? {
                blocks.extend(
                    self.edited(original, change)
                        .filter(|block| block.date == date),
                );
            }
        }
        blocks.extend(
            published
                .iter()
                .filter(|exam| exam.date == date)
                .filter_map(|exam| published_block(exam, include_cancelled)),
        );
        for block in &mut blocks {
            block.term_id = Some(self.term.into());
            block.planned = block
                .course_id
                .as_deref()
                .is_some_and(|id| !self.enrolled.contains(id))
                .then_some(true);
        }
        if !include_cancelled {
            blocks.retain(|block| block.cancelled != Some(true));
        }
        Ok(blocks)
    }

    pub(super) fn agenda_on(
        &self,
        date: &str,
        published: &[super::ExamView],
        options: AgendaOptions,
    ) -> Result<Vec<Occurrence>, ScheduleError> {
        let mut blocks = self.blocks_on(date, published, options.include_cancelled_courses)?;
        if options.include_personal {
            let events = options.events.unwrap_or_else(|| {
                array(self.workspace, "events")
                    .iter()
                    .filter(|row| text(row, "termId") == Some(self.term))
                    .cloned()
                    .collect()
            });
            blocks.extend(personal_occurrences(date, &events)?);
        }
        finalize_agenda(blocks, &self.periods)
    }
}

/// The civil date a class session key ends with.
pub(super) fn key_date(key: &str) -> Option<&str> {
    key.get(key.len().saturating_sub(10)..)
        .filter(|date| parse_civil_date(date).is_ok())
}

fn replacement_weekday(row: &Value) -> Option<u8> {
    u8::try_from(row.get("replacementDay")?.as_u64()?).ok()
}

#[cfg(test)]
pub(crate) fn occurrences_on_date(
    workspace: &Value,
    term: &str,
    date: &str,
    options: AgendaOptions,
) -> Result<Vec<Occurrence>, ScheduleError> {
    let sessions = CourseSessions::new(workspace, term, options.courses.clone())?;
    sessions.agenda_on(date, &[], options)
}

/// The weekly personal events, of the term the caller picked, that fall on `date`.
pub(crate) fn personal_occurrences(
    date: &str,
    events: &[Value],
) -> Result<Vec<Occurrence>, ScheduleError> {
    let day = weekday(date)?;
    let mut blocks = Vec::new();
    for event in events.iter().filter(|row| visible(row)) {
        let occurs = event
            .get("days")
            .and_then(Value::as_array)
            .is_some_and(|days| days.iter().any(|v| v.as_u64() == Some(u64::from(day))));
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
            color: super::event_color(event),
            conflict: None,
            dim: None,
            cancelled: None,
            time_label: None,
            conflict_with: None,
            overridden: None,
            alternation: None,
            planned: None,
            quiz: None,
            source_changed: None,
            grid: None,
            mark: None,
        });
    }
    Ok(blocks)
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
        planned: None,
        quiz: None,
        source_changed: None,
        grid: None,
        mark: None,
    }
}

pub(super) fn mark_conflicts(blocks: &mut [Occurrence]) -> Result<(), ScheduleError> {
    for i in 0..blocks.len() {
        for j in i + 1..blocks.len() {
            if blocks[i].cancelled == Some(true)
                || blocks[j].cancelled == Some(true)
                || alternation::alternate(&blocks[i], &blocks[j])
            {
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
pub(crate) fn overlap(a: &Occurrence, b: &Occurrence) -> Result<bool, ScheduleError> {
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
    school: &SchoolReads,
    term: &str,
    date: &str,
    options: AgendaOptions,
) -> Result<Vec<Occurrence>, ScheduleError> {
    let sessions = CourseSessions::new(workspace, term, options.courses.clone())?;
    let published = super::exams::published_exam_views(workspace, school, term);
    sessions.agenda_on(date, &published, options)
}

fn published_block(exam: &super::ExamView, include_cancelled: bool) -> Option<Occurrence> {
    let cancelled = exam.status != "scheduled";
    if cancelled && !(include_cancelled && exam.mark.as_deref() == Some("cancelled")) {
        return None;
    }
    let (id, start, end) = (
        exam.course_id.as_ref()?,
        exam.start_time.as_ref()?,
        exam.end_time.as_ref()?,
    );
    Some(Occurrence {
        key: exam.key.clone(),
        id: id.clone(),
        kind: if exam.kind == "class" {
            OccurrenceKind::Course
        } else {
            OccurrenceKind::Exam
        },
        term_id: None,
        course_id: Some(id.clone()),
        date: exam.date.clone(),
        title: exam.title.clone(),
        subtitle: Some(exam.code.clone()),
        room: exam.room.clone(),
        time_label: Some(exam.periods.map_or_else(
            || format!("{start}–{end}"),
            |p| format!("{}–{}限", p[0], p[1]),
        )),
        start_time: start.clone(),
        end_time: end.clone(),
        end_date: None,
        color: exam.color.clone(),
        conflict: None,
        dim: None,
        cancelled: cancelled.then_some(true),
        conflict_with: None,
        overridden: exam.edited.then_some(true),
        alternation: None,
        planned: None,
        quiz: (exam.kind == "quiz").then_some(true),
        source_changed: exam.conflict.is_some().then_some(true),
        grid: None,
        mark: None,
    })
}

/// The user's mark on a class session: an exam or quiz takes the class's
/// place, a cancelled class stays visible to undo, and `class` runs anyway.
fn apply_mark(block: &mut Occurrence, row: &Value) {
    match text(row, "mark") {
        Some("cancelled") => block.cancelled = Some(true),
        Some(mark @ ("exam" | "quiz")) => {
            block.kind = OccurrenceKind::Exam;
            block.cancelled = None;
            block.quiz = (mark == "quiz").then_some(true);
        }
        Some("class") => block.cancelled = None,
        _ => {}
    }
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
                .is_none_or(|y| !crate::study_profile::ADMISSION_YEARS.contains(&y))
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
        // Like the published calendars: a year's quarters start within that
        // academic year, so quarters of different years never overlap.
        let (first, next) = (format!("{year}-04-01"), format!("{}-04-01", year + 1));
        if values
            .iter()
            .flatten()
            .any(|v| *v < first.as_str() || *v >= next.as_str())
        {
            return Err(ScheduleError::InvalidInput(format!(
                "Configured year {} quarter starts must fall within academic year {year}",
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
