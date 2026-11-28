pub(crate) use super::profile_validation::validate_academic_profile;
pub(crate) use planning::planning_week;
pub use planning::PlanningWeek;
#[cfg(feature = "ts")]
pub(crate) use {planning::CourseConflict, planning::EventCarryOver, planning::PlanningDay};

use super::SchoolReads;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, NaiveTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

use super::agenda_layout::finalize_agenda;
use super::alternation::{self, Alternation, Resolution};
use super::calendar::{calendar_state, quarter_range};
use super::civil::weekday;
use super::colors::display_color;
use super::course_scope::{course_terms, placements};
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

pub(crate) struct CourseSlot<'a> {
    pub course_id: &'a str,
    pub(crate) start_time: &'a str,
    pub(crate) end_time: &'a str,
    pub room: &'a str,
}

impl Occurrence {
    /// A teaching session which still takes place, including exams.
    pub(crate) fn is_active_class(&self) -> bool {
        matches!(self.kind, OccurrenceKind::Course | OccurrenceKind::Exam)
            && self.cancelled != Some(true)
    }

    /// Each course keeps its own time and room in a shared 演習・実験 block.
    pub(crate) fn course_slots(&self) -> impl Iterator<Item = CourseSlot<'_>> {
        let pair = self
            .alternation
            .as_ref()
            .and_then(|row| row.pair.as_deref())
            .unwrap_or_default();
        self.course_id
            .as_deref()
            .filter(|_| pair.is_empty())
            .map(|course_id| CourseSlot {
                course_id,
                start_time: &self.start_time,
                end_time: &self.end_time,
                room: &self.room,
            })
            .into_iter()
            .chain(pair.iter().map(|course| CourseSlot {
                course_id: &course.course_id,
                start_time: &course.start_time,
                end_time: &course.end_time,
                room: &course.room,
            }))
    }

    pub(crate) fn course_ids(&self) -> impl Iterator<Item = &str> {
        self.course_slots().map(|slot| slot.course_id)
    }
}

#[cfg(any(test, feature = "agent"))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ScheduleProblemKind {
    CourseMeeting,
    ScheduleOverride,
}

#[cfg(any(test, feature = "agent"))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduleProblem {
    pub(crate) kind: ScheduleProblemKind,
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AgendaOptions {
    pub include_personal: bool,
    pub(crate) include_cancelled_courses: bool,
    pub courses: Option<Arc<[Value]>>,
    pub events: Option<Vec<Value>>,
}

mod planning;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct CourseTimeline {
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

/// The course's meetings around `now` across every quarter it belongs to, so a
/// 3Q–4Q course keeps its class times when the quarter changes.
pub(crate) fn course_timeline(
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
            )? {
                if let Some(slot) = row.course_slots().find(|slot| slot.course_id == course_id) {
                    starts.push(format!("{}T{}", row.date, slot.start_time));
                }
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

/// Courses on the timetable of `term`: those with a live membership or plan
/// item there. A course running 3Q–4Q is one course (its `termId` is where it
/// was added) with a row in each quarter.
pub(crate) fn scheduled_courses(workspace: &Value, term: &str) -> Vec<Value> {
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
            .filter(|row| visible(row) && names_session(row))
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

    /// The session a stored key names, at its original date.
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
        let source_date = block.date.clone();
        if apply_override(&mut block, change, &self.periods).is_ok() {
            // Moved to another day, it takes place there, whatever cancelled
            // it on its own day.
            if block.date != source_date {
                block.cancelled = None;
            }
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
                    // Exam weeks and closed days keep the weekly session as
                    // cancelled, so the person can mark it as running.
                    Resolution::Weekly(marker)
                        if on(teaching_day) || mark == Some("class") && on(day) =>
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
        // `approach/day-source-precedence`: the exam timetable states what a
        // course does that day, so its rows replace the weekly sessions; only
        // a session the person says still runs stays beside them.
        let official: HashSet<&str> = published
            .iter()
            .filter(|exam| exam.date == date)
            .filter_map(super::ExamView::replaced_course)
            .collect();
        blocks.retain(|block| {
            runs_anyway(block)
                || !weekly_session(&block.key)
                || !block
                    .course_id
                    .as_deref()
                    .is_some_and(|id| official.contains(id))
        });
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

/// Whether `key` names a session of a course's weekly meetings, as
/// `CourseSessions::sources` keys them; an intensive day is dated on its own.
fn weekly_session(key: &str) -> bool {
    key.contains("-weekly-")
}

/// A stored change to a class session. Rows written without `targetKind`
/// name a session too; `exam` rows belong to the exam timetable.
pub(super) fn names_session(row: &Value) -> bool {
    text(row, "targetKind").is_none_or(|kind| kind == "course")
}

/// A class session the person moved, re-roomed or marked as running. A
/// session they marked as an exam or a quiz says what the timetable says and
/// gives way to it.
fn runs_anyway(block: &Occurrence) -> bool {
    block.kind == OccurrenceKind::Course
        && block.overridden == Some(true)
        && block.cancelled != Some(true)
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
    Ok(events
        .iter()
        .filter(|event| visible(event) && occurs_on_weekday(event, day))
        .filter_map(|event| personal_occurrence(event, date, date))
        .collect())
}

fn occurs_on_weekday(row: &Value, day: u8) -> bool {
    row.get("days")
        .and_then(Value::as_array)
        .is_some_and(|days| days.iter().any(|v| v.as_u64() == Some(u64::from(day))))
}

fn personal_occurrence(event: &Value, date: &str, key: impl std::fmt::Display) -> Option<Occurrence> {
    let id = text(event, "id").unwrap_or_default();
    let start_time = super::timing::canonical_clock(text(event, "startTime")?)?;
    let end_time = super::timing::canonical_clock(text(event, "endTime")?)?;
    if end_time <= start_time {
        return None;
    }
    Some(Occurrence {
        key: format!("{id}-{key}"),
        id: id.into(),
        kind: OccurrenceKind::Event,
        term_id: None,
        course_id: None,
        date: date.into(),
        title: text(event, "title").unwrap_or_default().into(),
        subtitle: None,
        room: text(event, "room").unwrap_or_default().into(),
        start_time,
        end_time,
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
    })
}

/// Stored user records the schedule skips or leaves unapplied. Read models
/// stay available; the agent lists these so the person can repair or delete
/// them.
#[cfg(any(test, feature = "agent"))]
pub(crate) fn schedule_problems(workspace: &Value) -> Vec<ScheduleProblem> {
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

pub(crate) fn resolve_day_agenda(
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
    let id = exam.course_id.as_ref()?;
    let start = super::timing::canonical_clock(exam.start_time.as_deref()?)?;
    let end = super::timing::canonical_clock(exam.end_time.as_deref()?)?;
    if start >= end {
        return None;
    }
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
    let canonical = |raw| super::timing::canonical_clock(raw)
        .ok_or_else(|| ScheduleError::InvalidInput("Override time is invalid.".into()));
    let start = text(row, "startTime").or_else(|| clock.map(|v| v.0)).map(canonical).transpose()?;
    let end = text(row, "endTime").or_else(|| clock.map(|v| v.1)).map(canonical).transpose()?;
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
    if let Some(v) = start {
        block.start_time = v
    }
    if let Some(v) = end {
        block.end_time = v
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
        let Some((start, end)) = parsed.filter(|(start, end)| start < end) else {
            return Err(ScheduleError::InvalidInput(
                "Meeting start and end times are invalid.".into(),
            ));
        };
        Ok((start.format("%H:%M").to_string(), end.format("%H:%M").to_string()))
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

pub(crate) fn intensive_days(
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
