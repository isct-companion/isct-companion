//! Published exams and personal corrections, shared by lists, agendas and reminders.
//!
//! A personal correction stores only the fields that differ from the published
//! record, so a later source correction still reaches every field the user did
//! not change. The course link is stored beside those fields but edited apart.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::{
    array, clock_for_periods, content_language, display_color, localized, teaching_periods,
    term_courses, text, visible, ScheduleError, TeachingPeriod,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamView {
    pub id: String,
    pub key: String,
    /// `exam`, or `class` for a class the exam timetable lists in the exam
    /// period. A class enters agendas as a class, not as an exam.
    #[cfg_attr(feature = "ts", ts(type = "\"exam\" | \"class\""))]
    pub kind: String,
    pub term: String,
    pub course_id: Option<String>,
    /// The course came from code matching, not from a personal link.
    pub auto_linked: bool,
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
    /// Published facts before personal corrections.
    pub source: ExamSchedule,
    /// Date, time, room or title differs from the published record.
    pub edited: bool,
    pub hidden: bool,
    /// The user's mark, which outranks the published kind and status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "ts",
        ts(optional, type = "\"exam\" | \"cancelled\" | \"class\"")
    )]
    pub mark: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamSchedule {
    pub date: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub room: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ExamAssociation {
    /// Other exams (not exam-week classes) already linked to the course.
    pub conflicts: Vec<ExamView>,
}

/// Values from the exam form. An empty or source-equal field follows the
/// published record.
#[derive(Clone, Debug, Default)]
pub(crate) struct ExamCorrection {
    pub date: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub room: Option<String>,
    pub hidden: bool,
}

pub fn exam_association(
    workspace: &Value,
    key: &str,
    course_id: &str,
) -> Result<ExamAssociation, ScheduleError> {
    let (_, term) = published_exam(workspace, key)?;
    require_term_course(workspace, &term, course_id)?;
    let conflicts = exam_terms(workspace)
        .into_iter()
        .flat_map(|term| exam_views(workspace, &term))
        .filter(|other| {
            other.key != key
                && other.kind == "exam"
                && other.course_id.as_deref() == Some(course_id)
        })
        .collect();
    Ok(ExamAssociation { conflicts })
}

/// The stored correction after replacing the exam's date, time and room
/// corrections, or `None` when nothing personal remains.
pub(crate) fn corrected_exam_override(
    workspace: &Value,
    key: &str,
    correction: ExamCorrection,
) -> Result<Option<Value>, ScheduleError> {
    let (exam, term) = published_exam(workspace, key)?;
    let source = source_view(workspace, exam, &term).source;
    let filled = |value: Option<String>| {
        value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let mut row = override_base(workspace, key);
    if let Some(date) = filled(correction.date).filter(|date| *date != source.date) {
        super::parse_civil_date(&date)?;
        row.insert("date".into(), date.into());
    }
    match (filled(correction.start_time), filled(correction.end_time)) {
        (None, None) => {}
        (Some(start), Some(end)) => {
            if (Some(&start), Some(&end)) != (source.start_time.as_ref(), source.end_time.as_ref())
            {
                row.insert("startTime".into(), start.into());
                row.insert("endTime".into(), end.into());
            }
        }
        _ => {
            return Err(ScheduleError::InvalidInput(
                "Enter both a start and an end time.".into(),
            ))
        }
    }
    if let Some(room) = filled(correction.room).filter(|room| *room != source.room) {
        row.insert("room".into(), room.into());
    }
    row.insert("hidden".into(), correction.hidden.into());
    finish_override(row)
}

/// The stored correction after the user marks this exam-timetable row as an
/// exam, a class or cancelled. A kind mark equal to the published kind is
/// dropped, so the row follows its source again.
pub(crate) fn marked_exam_override(
    workspace: &Value,
    key: &str,
    mark: &str,
) -> Result<Option<Value>, ScheduleError> {
    let (exam, term) = published_exam(workspace, key)?;
    let source = source_view(workspace, exam, &term);
    let mut row = override_base(workspace, key);
    keep_schedule_fields(workspace, key, &mut row);
    row.remove("mark");
    if mark == "cancelled" || mark != source.kind || source.status == "cancelled" {
        row.insert("mark".into(), mark.into());
    }
    finish_override(row)
}

/// The stored override after the user marks one calendar session. Rows of
/// the exam timetable follow `marked_exam_override`; any other key is a class
/// session (imported or custom course), whose other corrections are kept.
pub(crate) fn marked_session_override(
    workspace: &Value,
    key: &str,
    mark: &str,
) -> Result<Option<Value>, ScheduleError> {
    if !matches!(mark, "exam" | "cancelled" | "class") {
        return Err(ScheduleError::InvalidInput(
            "Mark a session as an exam, cancelled or a normal class.".into(),
        ));
    }
    if published_exam(workspace, key).is_ok() {
        return marked_exam_override(workspace, key, mark);
    }
    if key.trim().is_empty() {
        return Err(ScheduleError::InvalidInput(
            "Choose a session to mark.".into(),
        ));
    }
    let mut row = Map::new();
    if let Some(existing) = existing_override(workspace, key).and_then(Value::as_object) {
        for field in [
            "courseId",
            "date",
            "periods",
            "startTime",
            "endTime",
            "room",
            "title",
            "hidden",
        ] {
            if let Some(value) = existing.get(field).filter(|value| !value.is_null()) {
                row.insert(field.into(), value.clone());
            }
        }
    }
    row.insert("targetKey".into(), key.into());
    row.insert("targetKind".into(), "course".into());
    row.entry("hidden").or_insert(json!(false));
    row.insert("mark".into(), mark.into());
    Ok(Some(Value::Object(row)))
}

/// The stored correction after linking the exam to `course_id`, or to no
/// course. Choosing the course code matching would pick returns to automatic
/// matching.
pub(crate) fn linked_exam_override(
    workspace: &Value,
    key: &str,
    course_id: Option<&str>,
) -> Result<Option<Value>, ScheduleError> {
    let (exam, term) = published_exam(workspace, key)?;
    if let Some(course_id) = course_id {
        require_term_course(workspace, &term, course_id)?;
    }
    let automatic = source_view(workspace, exam, &term).course_id;
    let mut row = override_base(workspace, key);
    keep_schedule_fields(workspace, key, &mut row);
    row.remove("courseId");
    if course_id != automatic.as_deref() {
        row.insert("courseId".into(), course_id.unwrap_or_default().into());
    }
    finish_override(row)
}

pub(super) fn exam_terms(workspace: &Value) -> std::collections::BTreeSet<String> {
    array(workspace, "exams")
        .iter()
        .filter(|row| visible(row))
        .filter_map(exam_term)
        .collect()
}

pub(super) fn upcoming_exams(workspace: &Value, from: &str) -> Vec<ExamView> {
    let mut exams: Vec<_> = exam_terms(workspace)
        .into_iter()
        .flat_map(|term| exam_views(workspace, &term))
        .filter(|exam| {
            exam.kind == "exam"
                && exam.course_id.is_some()
                && !exam.hidden
                && exam.status == "scheduled"
                && exam.date.as_str() >= from
        })
        .collect();
    sort_views(&mut exams);
    exams
}

pub fn exam_views(workspace: &Value, term: &str) -> Vec<ExamView> {
    let locale = content_language(workspace);
    let teaching = teaching_periods(workspace).unwrap_or_default();
    let courses = term_courses(workspace, term);
    let codes = course_code_keys(&courses);
    let mut result = Vec::new();
    for exam in array(workspace, "exams")
        .iter()
        .filter(|exam| visible(exam) && exam_term(exam).as_deref() == Some(term))
    {
        let mut view = source_view_with(workspace, exam, term, locale, &teaching, &courses, &codes);
        if let Some(change) = existing_override(workspace, &view.key) {
            apply_override(&mut view, change, &courses, &teaching);
        }
        result.push(view);
    }
    sort_views(&mut result);
    result
}

fn source_view(workspace: &Value, exam: &Value, term: &str) -> ExamView {
    let teaching = teaching_periods(workspace).unwrap_or_default();
    let courses = term_courses(workspace, term);
    source_view_with(
        workspace,
        exam,
        term,
        content_language(workspace),
        &teaching,
        &courses,
        &course_code_keys(&courses),
    )
}

/// Each of `courses`' code as exam matching compares it, in their order.
fn course_code_keys(courses: &[&Value]) -> Vec<String> {
    courses
        .iter()
        .map(|course| {
            text(course, "code")
                .map(super::search::course_code_key)
                .unwrap_or_default()
        })
        .collect()
}

fn source_view_with(
    workspace: &Value,
    exam: &Value,
    term: &str,
    locale: &str,
    teaching: &[TeachingPeriod],
    courses: &[&Value],
    codes: &[String],
) -> ExamView {
    let id = text(exam, "id").unwrap_or_default();
    // A feed-resolved offering names the course; otherwise match by code.
    let course = match text(exam, "courseId") {
        Some(id) => courses
            .iter()
            .copied()
            .find(|course| text(course, "id") == Some(id)),
        None => matched_course(workspace, courses, codes, term, exam),
    };
    let periods = period_range(exam, teaching);
    let (start_time, end_time) = match periods.and_then(|range| clock(teaching, range)) {
        Some((start, end)) => (Some(start), Some(end)),
        None => (
            text(exam, "startTime").map(str::to_owned),
            text(exam, "endTime").map(str::to_owned),
        ),
    };
    let source = ExamSchedule {
        date: text(exam, "date").unwrap_or_default().into(),
        start_time,
        end_time,
        room: localized(exam.get("room"), locale, ""),
    };
    ExamView {
        id: id.into(),
        key: exam_key(exam).into(),
        kind: if text(exam, "kind") == Some("class") {
            "class"
        } else {
            "exam"
        }
        .into(),
        term: term.into(),
        course_id: course.and_then(|row| text(row, "id")).map(str::to_owned),
        auto_linked: course.is_some(),
        code: super::search::course_code_key(text(exam, "courseCode").unwrap_or_default()),
        title: localized(
            exam.get("label"),
            locale,
            course
                .and_then(|row| text(row, "title"))
                .unwrap_or_default(),
        ),
        date: source.date.clone(),
        start_time: source.start_time.clone(),
        end_time: source.end_time.clone(),
        periods,
        room: source.room.clone(),
        notes: exam
            .get("notes")
            .filter(|value| !value.is_null())
            .map(|value| localized(Some(value), locale, ""))
            .filter(|value| !value.is_empty()),
        color: course
            .map(display_color)
            .unwrap_or_else(|| "#c98245".into()),
        status: text(exam, "status").unwrap_or("scheduled").into(),
        source,
        edited: false,
        hidden: false,
        mark: None,
    }
}

fn apply_override(
    view: &mut ExamView,
    change: &Value,
    courses: &[&Value],
    teaching: &[TeachingPeriod],
) {
    if let Some(course_id) = text(change, "courseId") {
        let course = courses
            .iter()
            .copied()
            .find(|course| text(course, "id") == Some(course_id));
        view.course_id = course.and_then(|row| text(row, "id")).map(str::to_owned);
        view.auto_linked = false;
        view.color = course
            .map(display_color)
            .unwrap_or_else(|| "#c98245".into());
    }
    let filled = |name| text(change, name).filter(|value| !value.trim().is_empty());
    if let Some(date) = filled("date") {
        view.date = date.into();
    }
    if let (Some(start), Some(end)) = (filled("startTime"), filled("endTime")) {
        view.start_time = Some(start.into());
        view.end_time = Some(end.into());
        view.periods = None;
    } else if let Some(range) = period_range(change, teaching) {
        let clock = clock(teaching, range);
        view.periods = Some(range);
        view.start_time = clock.as_ref().map(|pair| pair.0.clone());
        view.end_time = clock.map(|pair| pair.1);
    }
    if let Some(room) = filled("room") {
        view.room = room.into();
    }
    if let Some(title) = filled("title") {
        view.title = title.into();
    }
    view.hidden = change["hidden"].as_bool().unwrap_or(false);
    view.mark = text(change, "mark").map(str::to_owned);
    match view.mark.as_deref() {
        Some("cancelled") => view.status = "cancelled".into(),
        Some(kind @ ("exam" | "class")) => {
            view.kind = kind.into();
            view.status = "scheduled".into();
        }
        _ => {}
    }
    view.edited = filled("title").is_some()
        || view.date != view.source.date
        || view.start_time != view.source.start_time
        || view.end_time != view.source.end_time
        || view.room != view.source.room;
}

/// A class suffix is part of the identity. Never guess a neighbouring class or
/// a different academic year's course from the base code.
/// `codes` are `courses`' code keys, in their order. Code is a lookup key
/// inside one term, and only the canonical full code counts: a historical
/// LMS alias cannot silently bind an exam.
fn matched_course<'a>(
    workspace: &Value,
    courses: &[&'a Value],
    codes: &[String],
    term: &str,
    exam: &Value,
) -> Option<&'a Value> {
    let code = super::search::course_code_key(text(exam, "courseCode").unwrap_or_default());
    let offering = text(exam, "syllabusCourseId");
    let candidates: Vec<_> = courses
        .iter()
        .copied()
        .zip(codes)
        .filter(|(course, course_code)| {
            (if let Some(offering) = offering {
                text(course, "offeringId") == Some(offering)
            } else {
                !code.is_empty() && **course_code == code
            }) && array(workspace, "memberships").iter().any(|row| {
                visible(row)
                    && text(row, "termId") == Some(term)
                    && text(row, "courseId") == text(course, "id")
                    && text(row, "status") != Some("withdrawn")
                    && row.pointer("/evidence/provider").and_then(Value::as_str) != Some("manual")
            })
        })
        .map(|(course, _)| course)
        .collect();
    candidates
        .first()
        .filter(|_| candidates.len() == 1)
        .copied()
}

fn published_exam<'a>(
    workspace: &'a Value,
    key: &str,
) -> Result<(&'a Value, String), ScheduleError> {
    array(workspace, "exams")
        .iter()
        .filter(|exam| visible(exam) && exam_key(exam) == key)
        .find_map(|exam| Some((exam, exam_term(exam)?)))
        .ok_or_else(|| ScheduleError::InvalidInput("Exam is no longer available.".into()))
}

fn require_term_course(
    workspace: &Value,
    term: &str,
    course_id: &str,
) -> Result<(), ScheduleError> {
    if term_courses(workspace, term)
        .iter()
        .any(|course| text(course, "id") == Some(course_id))
    {
        Ok(())
    } else {
        Err(ScheduleError::InvalidInput(
            "Choose a course from the exam's quarter.".into(),
        ))
    }
}

fn existing_override<'a>(workspace: &'a Value, key: &str) -> Option<&'a Value> {
    array(workspace, "scheduleOverrides")
        .iter()
        .find(|row| visible(row) && text(row, "targetKey") == Some(key))
}

fn keep_schedule_fields(workspace: &Value, key: &str, row: &mut Map<String, Value>) {
    for field in ["date", "startTime", "endTime", "periods", "room"] {
        if let Some(value) = existing_override(workspace, key).and_then(|row| row.get(field)) {
            row.insert(field.into(), value.clone());
        }
    }
}

/// Identity and the fields the caller does not replace.
fn override_base(workspace: &Value, key: &str) -> Map<String, Value> {
    let existing = existing_override(workspace, key);
    let mut row = Map::new();
    row.insert("targetKey".into(), key.into());
    row.insert("targetKind".into(), "exam".into());
    for field in ["courseId", "title", "hidden", "mark"] {
        if let Some(value) = existing
            .and_then(|row| row.get(field))
            .filter(|v| !v.is_null())
        {
            row.insert(field.into(), value.clone());
        }
    }
    row.entry("hidden").or_insert(json!(false));
    row
}

fn finish_override(row: Map<String, Value>) -> Result<Option<Value>, ScheduleError> {
    let personal = row.contains_key("courseId")
        || row.contains_key("mark")
        || row.get("hidden").and_then(Value::as_bool) == Some(true)
        || ["date", "startTime", "endTime", "periods", "room", "title"]
            .iter()
            .any(|field| row.contains_key(*field));
    if !personal {
        return Ok(None);
    }
    // The record write applies `ScheduleOverride::validate`.
    Ok(Some(Value::Object(row)))
}

fn exam_key(exam: &Value) -> &str {
    text(exam, "sourceKey")
        .or_else(|| text(exam, "id"))
        .unwrap_or_default()
}

fn exam_term(exam: &Value) -> Option<String> {
    Some(format!(
        "{}-{}",
        exam["academicYear"].as_i64()?,
        text(exam, "quarter")?
    ))
}

fn period_range(row: &Value, teaching: &[TeachingPeriod]) -> Option<[u8; 2]> {
    let values = row.get("periods")?.as_array()?;
    if values.len() != 2 {
        return None;
    }
    let range = [
        u8::try_from(values[0].as_u64()?).ok()?,
        u8::try_from(values[1].as_u64()?).ok()?,
    ];
    clock_for_periods(teaching, range).ok()?;
    Some(range)
}

fn clock(teaching: &[TeachingPeriod], range: [u8; 2]) -> Option<(String, String)> {
    clock_for_periods(teaching, range)
        .ok()
        .map(|(start, end)| (start.to_owned(), end.to_owned()))
}

fn sort_views(views: &mut [ExamView]) {
    views.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then(a.start_time.cmp(&b.start_time))
            .then(a.key.cmp(&b.key))
    });
}
