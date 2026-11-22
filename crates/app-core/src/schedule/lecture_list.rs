//! The quarter's lecture list, ordered by each course's next teaching session.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    add_days, exam_views, range::calendar_between_with_personal, text, today::at, tokyo_date,
    workspace_overview, ExamView, OccurrenceKind, ScheduleError, SchoolReads,
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LectureMeeting {
    pub date: String,
    pub start_time: String,
    pub room: String,
    pub in_class: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LectureCourse {
    pub id: String,
    pub title: String,
    pub code: String,
    pub color: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LectureList {
    pub courses: Vec<LectureCourse>,
    pub next_by_course: BTreeMap<String, LectureMeeting>,
    pub exams: Vec<ExamView>,
    pub all_terms: Vec<String>,
}

/// A saved catalog description outlives the plan that placed it. Only a live
/// placement or LMS quarter includes a course in the lecture roster.
pub(super) fn lecture_courses<'a>(workspace: &'a Value, term: Option<&str>) -> Vec<&'a Value> {
    let in_term = |row: &Value| term.is_none_or(|term| text(row, "termId") == Some(term));
    let placed: HashSet<_> = super::occurrence::placements(workspace)
        .filter(|row| in_term(row) && text(row, "status") != Some("withdrawn"))
        .filter_map(|row| text(row, "courseId"))
        .collect();
    let mut lms_ids = HashSet::new();
    let mut offerings = HashSet::new();
    for row in super::array(workspace, "lmsCourses").iter().filter(|row| {
        super::visible(row)
            && row["hidden"] != true
            && crate::data::lms_terms(row)
                .iter()
                .any(|quarter| term.is_none_or(|t| t == quarter))
    }) {
        lms_ids.extend(row["lmsId"].as_u64());
        offerings.extend(text(row, "offeringId").filter(|id| !id.is_empty()));
    }
    super::array(workspace, "courses")
        .iter()
        .filter(|course| {
            super::visible(course)
                && (text(course, "id").is_some_and(|id| placed.contains(id))
                    || course["lmsCourseId"]
                        .as_u64()
                        .is_some_and(|id| lms_ids.contains(&id))
                    || text(course, "offeringId").is_some_and(|id| offerings.contains(id)))
        })
        .collect()
}

pub(super) fn lecture_list(
    workspace: &Value,
    school: &SchoolReads,
    term: &str,
    now: DateTime<FixedOffset>,
) -> Result<LectureList, ScheduleError> {
    let overview = workspace_overview(workspace, school, term)?;
    let date = tokyo_date(now);
    let agendas =
        calendar_between_with_personal(workspace, school, &date, &add_days(&date, 7)?, false)?;
    let mut next = BTreeMap::<String, LectureMeeting>::new();
    for day in agendas {
        for block in day.occurrences.into_iter().filter(|block| {
            block.is_active_class()
                && block.kind == OccurrenceKind::Course
                && block.term_id.as_deref() == Some(term)
        }) {
            for slot in block.course_slots() {
                if now
                    >= at(
                        block.end_date.as_deref().unwrap_or(&day.date),
                        slot.end_time,
                    )?
                {
                    continue;
                }
                if next.get(slot.course_id).is_some_and(|prior| {
                    (prior.date.as_str(), prior.start_time.as_str())
                        <= (day.date.as_str(), slot.start_time)
                }) {
                    continue;
                }
                next.insert(
                    slot.course_id.to_owned(),
                    LectureMeeting {
                        date: day.date.clone(),
                        start_time: slot.start_time.to_owned(),
                        room: slot.room.to_owned(),
                        in_class: at(&day.date, slot.start_time)? <= now,
                    },
                );
            }
        }
    }
    let ids: HashSet<_> = overview
        .term_course_ids
        .iter()
        .map(String::as_str)
        .collect();
    let mut courses: Vec<_> = super::array(workspace, "courses")
        .iter()
        .filter(|course| {
            super::visible(course) && text(course, "id").is_some_and(|id| ids.contains(id))
        })
        .map(|course| LectureCourse {
            id: text(course, "id").unwrap_or_default().into(),
            title: text(course, "title").unwrap_or_default().into(),
            code: text(course, "code").unwrap_or_default().into(),
            color: super::display_color(course),
        })
        .collect();
    courses.sort_by_cached_key(|course| {
        let meeting = next.get(&course.id);
        let upcoming = meeting.filter(|item| !item.in_class);
        (
            meeting.is_none(),
            upcoming.map(|item| (item.date.clone(), item.start_time.clone())),
            course.title.clone(),
        )
    });
    Ok(LectureList {
        courses,
        next_by_course: next,
        exams: exam_views(workspace, school, term),
        all_terms: overview.all_terms,
    })
}
