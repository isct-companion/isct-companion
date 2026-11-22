//! The quarter's lecture list, ordered by each course's next teaching session.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    add_days, exam_views, range::calendar_between_with_personal, text, today::at, tokyo_date,
    workspace_overview, ExamView, OccurrenceKind, ScheduleError, SchoolReads,
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
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
pub struct LectureList {
    #[cfg_attr(feature = "ts", ts(type = "unknown[]"))]
    pub courses: Vec<Value>,
    pub next_by_course: BTreeMap<String, LectureMeeting>,
    pub exams: Vec<ExamView>,
    pub all_terms: Vec<String>,
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
        for block in day
            .occurrences
            .into_iter()
            .filter(|block| block.is_active_class() && block.kind == OccurrenceKind::Course)
        {
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
    let mut courses = overview.courses;
    courses.sort_by_cached_key(|course| {
        let meeting = text(course, "id").and_then(|id| next.get(id));
        let upcoming = meeting.filter(|item| !item.in_class);
        (
            meeting.is_none(),
            upcoming.map(|item| (item.date.clone(), item.start_time.clone())),
            text(course, "title").unwrap_or_default().to_owned(),
        )
    });
    Ok(LectureList {
        courses,
        next_by_course: next,
        exams: exam_views(workspace, school, term),
        all_terms: overview.all_terms,
    })
}
