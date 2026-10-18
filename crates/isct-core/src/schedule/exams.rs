//! Published exams and personal corrections, shared by lists, agendas and reminders.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::{
    array, clock_for_periods, content_language, display_color, localized, teaching_periods,
    term_courses, text, visible,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamView {
    pub id: String,
    pub key: String,
    pub term: String,
    pub course_id: Option<String>,
    pub code: String,
    pub title: String,
    pub date: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub periods: Option<[u8; 2]>,
    pub room: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notes: Option<String>,
    pub color: String,
    pub status: String,
    pub edited: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamAssociation {
    pub requires_confirmation: bool,
    pub conflicts: Vec<ExamView>,
}

pub fn exam_association(
    workspace: &Value,
    key: &str,
    course_id: &str,
) -> Result<ExamAssociation, super::ScheduleError> {
    let all: Vec<_> = exam_terms(workspace)
        .into_iter()
        .flat_map(|term| exam_views(workspace, &term))
        .collect();
    let exam = all
        .iter()
        .find(|exam| exam.key == key)
        .ok_or_else(|| super::ScheduleError::InvalidInput("Exam is no longer available.".into()))?;
    if !term_courses(workspace, &exam.term)
        .iter()
        .any(|course| text(course, "id") == Some(course_id))
    {
        return Err(super::ScheduleError::InvalidInput(
            "Choose a course from the exam's quarter.".into(),
        ));
    }
    let conflicts: Vec<_> = all
        .iter()
        .filter(|other| other.key != key && other.course_id.as_deref() == Some(course_id))
        .cloned()
        .collect();
    Ok(ExamAssociation {
        requires_confirmation: exam.course_id.as_deref() != Some(course_id)
            && !conflicts.is_empty(),
        conflicts,
    })
}

pub(super) fn exam_terms(workspace: &Value) -> std::collections::BTreeSet<String> {
    array(workspace, "exams")
        .iter()
        .filter(|row| visible(row))
        .filter_map(|exam| {
            Some(format!(
                "{}-{}",
                exam["academicYear"].as_i64()?,
                text(exam, "quarter")?
            ))
        })
        .collect()
}

pub(super) fn upcoming_exams(workspace: &Value, from: &str) -> Vec<ExamView> {
    let mut exams: Vec<_> = exam_terms(workspace)
        .into_iter()
        .flat_map(|term| exam_views(workspace, &term))
        .filter(|exam| {
            exam.course_id.is_some()
                && !exam.hidden
                && exam.status == "scheduled"
                && exam.date.as_str() >= from
        })
        .collect();
    exams.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.start_time.cmp(&b.start_time))
            .then(a.key.cmp(&b.key))
    });
    exams
}

pub fn exam_views(workspace: &Value, term: &str) -> Vec<ExamView> {
    let locale = content_language(workspace);
    let teaching = teaching_periods(workspace).unwrap_or_default();
    let courses = term_courses(workspace, term);
    let mut result = Vec::new();
    for exam in array(workspace, "exams").iter().filter(|exam| {
        visible(exam)
            && format!(
                "{}-{}",
                exam["academicYear"].as_i64().unwrap_or_default(),
                text(exam, "quarter").unwrap_or_default()
            ) == term
    }) {
        let id = text(exam, "id").unwrap_or_default();
        let key = text(exam, "sourceKey").unwrap_or(id);
        let change = array(workspace, "scheduleOverrides")
            .iter()
            .find(|row| visible(row) && text(row, "targetKey") == Some(key));
        let explicit = change
            .and_then(|row| text(row, "courseId"))
            .or_else(|| text(exam, "courseId"));
        let code = super::search::course_code_key(text(exam, "courseCode").unwrap_or_default());
        let code_candidates = super::search::courses_by_code(workspace, term, &code);
        let offering = text(exam, "syllabusCourseId");
        // A class suffix is part of the identity. Never guess a neighbouring
        // class or a different academic year's course from the base code.
        let candidates: Vec<_> = courses
            .iter()
            .copied()
            .filter(|course| {
                if let Some(id) = explicit {
                    return text(course, "id") == Some(id);
                }
                (if let Some(offering) = offering {
                    text(course, "offeringId") == Some(offering)
                } else {
                    code_candidates
                        .iter()
                        .any(|candidate| text(candidate, "id") == text(course, "id"))
                }) && array(workspace, "memberships").iter().any(|row| {
                    visible(row)
                        && text(row, "termId") == Some(term)
                        && text(row, "courseId") == text(course, "id")
                        && text(row, "status") != Some("withdrawn")
                        && row.pointer("/evidence/provider").and_then(Value::as_str)
                            != Some("manual")
                })
            })
            .collect();
        let course = candidates
            .first()
            .filter(|_| candidates.len() == 1)
            .copied();
        let period_range = |row: &Value| -> Option<[u8; 2]> {
            let values = row.get("periods")?.as_array()?;
            if values.len() != 2 {
                return None;
            }
            let range = [
                u8::try_from(values[0].as_u64()?).ok()?,
                u8::try_from(values[1].as_u64()?).ok()?,
            ];
            clock_for_periods(&teaching, range).ok()?;
            Some(range)
        };
        let periods = change.and_then(period_range).or_else(|| period_range(exam));
        let clock = periods.and_then(|range| clock_for_periods(&teaching, range).ok());
        let field = |name| {
            change
                .and_then(|row| text(row, name))
                .or_else(|| text(exam, name))
        };
        result.push(ExamView {
            id: id.into(),
            key: key.into(),
            term: term.into(),
            course_id: course.and_then(|row| text(row, "id")).map(str::to_owned),
            code,
            title: change
                .and_then(|row| text(row, "title"))
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    localized(
                        exam.get("label"),
                        locale,
                        course
                            .and_then(|row| text(row, "title"))
                            .unwrap_or_default(),
                    )
                }),
            date: field("date").unwrap_or_default().into(),
            start_time: change
                .and_then(|row| text(row, "startTime"))
                .or_else(|| clock.map(|pair| pair.0))
                .or_else(|| text(exam, "startTime"))
                .map(str::to_owned),
            end_time: change
                .and_then(|row| text(row, "endTime"))
                .or_else(|| clock.map(|pair| pair.1))
                .or_else(|| text(exam, "endTime"))
                .map(str::to_owned),
            periods: if change.is_some_and(|row| {
                text(row, "startTime").is_some() || text(row, "endTime").is_some()
            }) {
                None
            } else {
                periods
            },
            room: change
                .and_then(|row| text(row, "room"))
                .map(str::to_owned)
                .unwrap_or_else(|| localized(exam.get("room"), locale, "")),
            notes: exam
                .get("notes")
                .filter(|value| !value.is_null())
                .map(|value| localized(Some(value), locale, ""))
                .filter(|value| !value.is_empty()),
            color: course
                .map(display_color)
                .unwrap_or_else(|| "#c98245".into()),
            status: text(exam, "status").unwrap_or("scheduled").into(),
            edited: change.is_some(),
            hidden: change
                .and_then(|row| row["hidden"].as_bool())
                .unwrap_or(false),
        });
    }
    result.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.start_time.cmp(&b.start_time))
            .then(a.key.cmp(&b.key))
    });
    result
}
