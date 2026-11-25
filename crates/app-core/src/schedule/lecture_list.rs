//! The quarter's lecture list, ordered by each course's next teaching session.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{
    add_days, array, course_scope::lecture_courses, credits::earned_credits_by_term, exam_views,
    range::calendar_between_with_personal, text, today::at, tokyo_date, workspace_terms, ExamView,
    OccurrenceKind, ScheduleError, SchoolReads,
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
    /// `exams` by key, in the order the exam list shows them.
    pub exam_groups: LectureExamGroups,
    pub all_terms: Vec<String>,
}

/// The exam list's groups. Exams still to come lead, then the exam-week
/// classes still to come, both earliest first; what is over follows, the
/// latest first, so the exam just sat comes before older ones.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LectureExamGroups {
    /// Exams and quizzes that are not over.
    pub upcoming_keys: Vec<String>,
    /// Exam-week classes that are not over.
    pub class_keys: Vec<String>,
    /// Exams, quizzes and exam-week classes that are over.
    pub ended_keys: Vec<String>,
}

impl LectureExamGroups {
    /// `exams` is in date order, as `exam_views` returns it.
    fn of(exams: &[ExamView], now: DateTime<FixedOffset>) -> Self {
        let mut groups = Self::default();
        for exam in exams {
            let key = exam.key.clone();
            if exam.ended(now) {
                groups.ended_keys.push(key);
            } else if exam.kind == "class" {
                groups.class_keys.push(key);
            } else {
                groups.upcoming_keys.push(key);
            }
        }
        groups.ended_keys.reverse();
        groups
    }
}

pub(super) fn lecture_list(
    workspace: &Value,
    school: &SchoolReads,
    term: &str,
    now: DateTime<FixedOffset>,
) -> Result<LectureList, ScheduleError> {
    let graded_terms = earned_credits_by_term(array(workspace, "grades")).into_keys();
    let terms = workspace_terms(workspace, school, term, graded_terms)?;
    let term = terms.shown.to_string();
    let date = tokyo_date(now);
    let agendas =
        calendar_between_with_personal(workspace, school, &date, &add_days(&date, 7)?, false)?;
    let mut next = BTreeMap::<String, LectureMeeting>::new();
    for day in agendas {
        for block in day.occurrences.into_iter().filter(|block| {
            block.is_active_class()
                && block.kind == OccurrenceKind::Course
                && block.term_id.as_deref() == Some(term.as_str())
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
    let mut courses: Vec<_> = lecture_courses(workspace, Some(&term))
        .into_iter()
        .filter_map(|course| {
            Some(LectureCourse {
                id: text(course, "id")?.into(),
                title: text(course, "title").unwrap_or_default().into(),
                code: text(course, "code").unwrap_or_default().into(),
                color: super::display_color(course),
            })
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
    let exams = exam_views(workspace, school, &term);
    Ok(LectureList {
        courses,
        next_by_course: next,
        exam_groups: LectureExamGroups::of(&exams, now),
        exams,
        all_terms: terms.available,
    })
}
