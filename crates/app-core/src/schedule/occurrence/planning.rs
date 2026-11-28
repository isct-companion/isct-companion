use super::{
    course_occurrence, mark_conflicts, meeting_clock, occurs_on_weekday, overlap,
    personal_occurrence, scheduled_courses, Occurrence, OccurrenceKind,
};
use crate::schedule::{
    add_days, alternation, array, teaching_periods, text, visible, ScheduleError, Term,
    TimetableAxis,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

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
    pub axis: TimetableAxis,
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

pub(crate) fn planning_week(workspace: &Value, term: &str) -> Result<PlanningWeek, ScheduleError> {
    Term::parse(term)?;
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
            if occurs_on_weekday(event, day) {
                blocks.extend(personal_occurrence(event, &date, day));
            }
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
    let axis = crate::schedule::timing::axis_for_occurrences(
        &teaching_periods,
        days.iter().flat_map(|day| &day.blocks),
    )?;
    Ok(PlanningWeek {
        courses,
        days,
        axis,
        conflict_count,
        conflicts,
        carry_over,
    })
}
