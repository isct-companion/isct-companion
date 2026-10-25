//! Published exams and personal corrections, shared by lists, agendas and reminders.
//!
//! A personal correction stores only the fields that differ from the published
//! record, so a later source correction still reaches every field the user did
//! not change. The course link is stored beside those fields but edited apart.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::SchoolReads;
use super::{
    array, clock_for_periods, content_language, display_color, localized, teaching_periods,
    term_courses, text, visible, OccurrenceKind, ScheduleError, TeachingPeriod,
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
    /// `exam`, `quiz` (小テスト, only by the user's mark), or `class` for a
    /// class the exam timetable lists in the exam period. A class enters
    /// agendas as a class, not as an exam.
    #[cfg_attr(feature = "ts", ts(type = "\"exam\" | \"quiz\" | \"class\""))]
    pub kind: String,
    /// `published` for a row of the exam timetable, `session` for a class
    /// session the user marked as an exam or quiz; its key is the session's.
    #[cfg_attr(feature = "ts", ts(type = "\"published\" | \"session\""))]
    pub origin: String,
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
        ts(optional, type = "\"exam\" | \"quiz\" | \"cancelled\" | \"class\"")
    )]
    pub mark: Option<String>,
    /// The published record moved away from what the user corrected, or left
    /// the timetable. The view keeps the user's values until they choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub conflict: Option<ExamConflict>,
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
pub struct ExamConflict {
    /// The published facts when the user last corrected the exam. `source`
    /// on the view holds the published facts now.
    pub base: ExamSchedule,
    /// The exam timetable no longer lists this exam.
    pub removed: bool,
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
    school: &SchoolReads,
    key: &str,
    course_id: &str,
) -> Result<ExamAssociation, ScheduleError> {
    let source = published_source(workspace, school, key)?;
    require_term_course(workspace, &source.term, course_id)?;
    let conflicts = exam_terms(school)
        .into_iter()
        .flat_map(|term| exam_views(workspace, school, &term))
        .filter(|other| {
            other.key != key
                && other.kind == "exam"
                && other.course_id.as_deref() == Some(course_id)
        })
        .collect();
    Ok(ExamAssociation { conflicts })
}

/// The stored correction after replacing the exam's date, time and room
/// corrections, or `None` when nothing personal remains. Saving records the
/// published facts it corrects, so a later change to them reads as a conflict.
pub(crate) fn corrected_exam_override(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    correction: ExamCorrection,
) -> Result<Option<Value>, ScheduleError> {
    let Ok(view) = published_source(workspace, school, key) else {
        return corrected_session_override(workspace, school, key, correction);
    };
    let source = &view.source;
    let mut row = override_base(workspace, school, key, &view);
    row.insert("base".into(), base_record(&view, source));
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

/// The same form for a class session the user marked as an exam or quiz: the
/// fields move that session, and the mark stays.
fn corrected_session_override(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    correction: ExamCorrection,
) -> Result<Option<Value>, ScheduleError> {
    let view = existing_override(workspace, key)
        .and_then(|change| session_terms(workspace, change).into_iter().next())
        .and_then(|term| {
            session_exam_views(
                workspace,
                &term,
                &published_exam_views(workspace, school, &term),
            )
            .into_iter()
            .find(|view| view.key == key)
        })
        .ok_or_else(|| ScheduleError::InvalidInput("Exam is no longer available.".into()))?;
    let source = &view.source;
    let mut row = session_override_base(workspace, key);
    for field in ["date", "periods", "startTime", "endTime", "room"] {
        row.remove(field);
    }
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
    Ok(Some(Value::Object(row)))
}

fn filled(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The stored correction after the user marks this exam-timetable row as an
/// exam, a quiz, a class or cancelled. A kind mark equal to the published
/// kind is dropped, so the row follows its source again.
pub(crate) fn marked_exam_override(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    mark: &str,
) -> Result<Option<Value>, ScheduleError> {
    let source = published_source(workspace, school, key)?;
    let mut row = override_base(workspace, school, key, &source);
    keep_schedule_fields(workspace, school, key, &mut row);
    row.remove("mark");
    if mark == "cancelled" || mark != source.kind || source.status == "cancelled" {
        row.insert("mark".into(), mark.into());
    }
    finish_override(row)
}

/// The stored override after the user marks one calendar session. Rows of
/// the exam timetable follow `marked_exam_override`; any other key is a class
/// session (imported or custom course), whose other corrections are kept.
/// Marking a session a plain class it already is removes the mark, so a later
/// calendar change (a holiday, 休講) still reaches it.
pub(crate) fn marked_session_override(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    mark: &str,
) -> Result<Option<Value>, ScheduleError> {
    if !matches!(mark, "exam" | "quiz" | "cancelled" | "class") {
        return Err(ScheduleError::InvalidInput(
            "Mark a session as an exam, a quiz, cancelled or a normal class.".into(),
        ));
    }
    if published_source(workspace, school, key).is_ok() {
        return marked_exam_override(workspace, school, key, mark);
    }
    if key.trim().is_empty() {
        return Err(ScheduleError::InvalidInput(
            "Choose a session to mark.".into(),
        ));
    }
    let mut row = session_override_base(workspace, key);
    if mark == "class" && !source_cancels_session(workspace, key)? {
        let moved = ["date", "periods", "startTime", "endTime", "room", "title"]
            .iter()
            .any(|field| row.contains_key(*field));
        if !moved && row.get("hidden").and_then(Value::as_bool) != Some(true) {
            return Ok(None);
        }
    } else {
        row.insert("mark".into(), mark.into());
    }
    Ok(Some(Value::Object(row)))
}

/// A class session's override without its mark: identity and the
/// corrections the user made to it.
fn session_override_base(workspace: &Value, key: &str) -> Map<String, Value> {
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
    row
}

/// Whether the published sources alone (calendar closures, 休講, the 演習
/// rotation) cancel this session, judged without the user's own record.
fn source_cancels_session(workspace: &Value, key: &str) -> Result<bool, ScheduleError> {
    let Some(date) = key_date(key) else {
        return Ok(false);
    };
    let mut without = workspace.clone();
    if let Some(rows) = without
        .get_mut("scheduleOverrides")
        .and_then(Value::as_array_mut)
    {
        rows.retain(|row| text(row, "targetKey") != Some(key));
    }
    for term in super::placement_terms(&without)
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
    {
        let options = super::AgendaOptions {
            include_personal: false,
            include_cancelled_courses: true,
            ..super::AgendaOptions::default()
        };
        if let Some(block) = super::occurrence::occurrences_on_date(&without, &term, date, options)?
            .into_iter()
            .find(|block| block.key == key)
        {
            return Ok(block.cancelled == Some(true));
        }
    }
    Ok(false)
}

/// The civil date a class session key ends with.
fn key_date(key: &str) -> Option<&str> {
    key.get(key.len().saturating_sub(10)..)
        .filter(|value| super::parse_civil_date(value).is_ok())
}

/// The stored correction after linking the exam to `course_id`, or to no
/// course. Choosing the course code matching would pick returns to automatic
/// matching.
pub(crate) fn linked_exam_override(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    course_id: Option<&str>,
) -> Result<Option<Value>, ScheduleError> {
    let source = published_source(workspace, school, key)?;
    if let Some(course_id) = course_id {
        require_term_course(workspace, &source.term, course_id)?;
    }
    let mut row = override_base(workspace, school, key, &source);
    keep_schedule_fields(workspace, school, key, &mut row);
    row.remove("courseId");
    if course_id != source.course_id.as_deref() {
        row.insert("courseId".into(), course_id.unwrap_or_default().into());
    }
    finish_override(row)
}

pub(super) fn exam_terms(school: &SchoolReads) -> std::collections::BTreeSet<String> {
    school.exams.terms().map(str::to_owned).collect()
}

/// Whether quizzes count as exams for Today and reminders (on by default).
pub(crate) fn quizzes_count(workspace: &Value) -> bool {
    workspace
        .pointer("/settings/notifications/quizzes")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

pub(super) fn upcoming_exams(workspace: &Value, school: &SchoolReads, from: &str) -> Vec<ExamView> {
    let quizzes = quizzes_count(workspace);
    let mut terms = exam_terms(school);
    terms.extend(super::placement_terms(workspace).map(str::to_owned));
    let mut exams: Vec<_> = terms
        .into_iter()
        .flat_map(|term| exam_views(workspace, school, &term))
        .filter(|exam| {
            (exam.kind == "exam" || (quizzes && exam.kind == "quiz"))
                && exam.course_id.is_some()
                && !exam.hidden
                && exam.status == "scheduled"
                && exam.date.as_str() >= from
        })
        .collect();
    sort_views(&mut exams);
    exams
}

/// Every exam of the quarter: the exam timetable with the user's corrections,
/// and the class sessions the user marked as an exam or a quiz.
pub fn exam_views(workspace: &Value, school: &SchoolReads, term: &str) -> Vec<ExamView> {
    let mut result = published_exam_views(workspace, school, term);
    let sessions = session_exam_views(workspace, term, &result);
    result.extend(sessions);
    sort_views(&mut result);
    result
}

/// The exam timetable's rows of `term` with the user's corrections, and the
/// corrections whose exam left the timetable. Agendas draw these; a marked
/// class session is drawn from the class itself.
pub(crate) fn published_exam_views(
    workspace: &Value,
    school: &SchoolReads,
    term: &str,
) -> Vec<ExamView> {
    let locale = content_language(workspace);
    let teaching = teaching_periods(workspace).unwrap_or_default();
    let courses = term_courses(workspace, term);
    let codes = course_code_keys(&courses);
    let orphans = Orphans::read(workspace, school);
    let mut result = Vec::new();
    for exam in school.exams.of(term) {
        let mut view = source_view_with(workspace, exam, term, locale, &teaching, &courses, &codes);
        let change = existing_override(workspace, &view.key)
            .or_else(|| orphans.adopted_by(workspace, school, exam, term));
        if let Some(change) = change {
            apply_override(&mut view, change, &courses, &teaching);
        }
        result.push(view);
    }
    for change in orphans.removed(workspace, school, term) {
        if let Some(mut view) = removed_view(change, term, &courses) {
            apply_override(&mut view, change, &courses, &teaching);
            result.push(view);
        }
    }
    sort_views(&mut result);
    result
}

/// Class sessions of `term` the user marked as an exam or a quiz, read from
/// the day agenda so they show the session's own corrections. A session whose
/// course has a published exam that day gives way to it.
fn session_exam_views(workspace: &Value, term: &str, published: &[ExamView]) -> Vec<ExamView> {
    let range = super::calendar::quarter_range(workspace, term)
        .ok()
        .flatten();
    let options = || super::AgendaOptions {
        include_personal: false,
        include_cancelled_courses: true,
        ..super::AgendaOptions::default()
    };
    let mut result = Vec::new();
    for change in array(workspace, "scheduleOverrides").iter().filter(|row| {
        visible(row)
            && text(row, "targetKind") == Some("course")
            && matches!(text(row, "mark"), Some("exam" | "quiz"))
            && row.get("hidden").and_then(Value::as_bool) != Some(true)
    }) {
        let key = text(change, "targetKey").unwrap_or_default();
        let Some(source_date) = key_date(key) else {
            continue;
        };
        let date = text(change, "date").unwrap_or(source_date);
        if range.as_ref().is_some_and(|range| {
            source_date < range.from.as_str() || source_date > range.to.as_str()
        }) {
            continue;
        }
        let Some(block) =
            super::occurrence::resolve_day_agenda_with(workspace, term, date, options(), published)
                .ok()
                .and_then(|blocks| {
                    blocks
                        .into_iter()
                        .find(|block| block.key == key && block.kind == OccurrenceKind::Exam)
                })
        else {
            continue;
        };
        let source =
            super::occurrence::occurrences_on_date(workspace, term, source_date, options())
                .ok()
                .and_then(|blocks| blocks.into_iter().find(|row| row.key == key))
                .map(|row| ExamSchedule {
                    date: row.date,
                    start_time: Some(row.start_time),
                    end_time: Some(row.end_time),
                    room: row.room,
                })
                .unwrap_or_else(|| ExamSchedule {
                    date: block.date.clone(),
                    start_time: Some(block.start_time.clone()),
                    end_time: Some(block.end_time.clone()),
                    room: block.room.clone(),
                });
        let edited = block.date != source.date
            || Some(&block.start_time) != source.start_time.as_ref()
            || Some(&block.end_time) != source.end_time.as_ref()
            || block.room != source.room;
        let mark = text(change, "mark").unwrap_or("exam");
        result.push(ExamView {
            id: key.into(),
            key: key.into(),
            kind: mark.into(),
            origin: "session".into(),
            term: term.into(),
            course_id: block.course_id,
            auto_linked: false,
            code: super::search::course_code_key(block.subtitle.as_deref().unwrap_or_default()),
            title: block.title,
            date: block.date,
            start_time: Some(block.start_time),
            end_time: Some(block.end_time),
            periods: None,
            room: block.room,
            notes: None,
            color: block.color,
            status: "scheduled".into(),
            source,
            edited,
            hidden: false,
            mark: Some(mark.into()),
            conflict: None,
        });
    }
    result
}

/// The quarters a marked session may belong to: those whose dates hold it.
fn session_terms(workspace: &Value, change: &Value) -> Vec<String> {
    let Some(date) = text(change, "targetKey").and_then(key_date) else {
        return Vec::new();
    };
    super::placement_terms(workspace)
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|term| {
            super::calendar::quarter_range(workspace, term)
                .ok()
                .flatten()
                .is_none_or(|range| date >= range.from.as_str() && date <= range.to.as_str())
        })
        .collect()
}

/// Corrections of published exams whose row is gone. The exam timetable names
/// a row after its date, so an exam that moves comes back under a new key;
/// its correction follows it when exactly one uncorrected row of the same
/// quarter, course code and kind remains, and otherwise reads as removed.
struct Orphans<'a> {
    rows: Vec<(&'a Value, Identity)>,
}

#[derive(PartialEq)]
struct Identity {
    term: String,
    code: String,
    kind: String,
}

impl<'a> Orphans<'a> {
    fn read(workspace: &'a Value, school: &SchoolReads) -> Self {
        let rows = array(workspace, "scheduleOverrides")
            .iter()
            .filter(|row| visible(row) && text(row, "targetKind") == Some("exam"))
            .filter(|row| {
                published_row(school, text(row, "targetKey").unwrap_or_default()).is_none()
            })
            .filter_map(|row| {
                let base = row.get("base")?;
                Some((
                    row,
                    Identity {
                        term: text(base, "term")?.into(),
                        code: text(base, "code")?.into(),
                        kind: text(base, "kind").unwrap_or("exam").into(),
                    },
                ))
            })
            .collect();
        Self { rows }
    }

    fn adopted_by(
        &self,
        workspace: &Value,
        school: &SchoolReads,
        exam: &Value,
        term: &str,
    ) -> Option<&'a Value> {
        if self.rows.is_empty() {
            return None;
        }
        let identity = row_identity(exam, term);
        let candidates: Vec<_> = self.rows.iter().filter(|(_, id)| *id == identity).collect();
        let [(orphan, _)] = candidates.as_slice() else {
            return None;
        };
        (uncorrected_rows(workspace, school, &identity) == 1).then_some(*orphan)
    }

    fn removed(&self, workspace: &Value, school: &SchoolReads, term: &str) -> Vec<&'a Value> {
        self.rows
            .iter()
            .filter(|(_, id)| id.term == term)
            .filter(|(_, id)| {
                self.rows.iter().filter(|(_, other)| other == id).count() != 1
                    || uncorrected_rows(workspace, school, id) != 1
            })
            .map(|(row, _)| *row)
            .collect()
    }
}

fn row_identity(exam: &Value, term: &str) -> Identity {
    Identity {
        term: term.into(),
        code: super::search::course_code_key(text(exam, "courseCode").unwrap_or_default()),
        kind: if text(exam, "kind") == Some("class") {
            "class"
        } else {
            "exam"
        }
        .into(),
    }
}

fn uncorrected_rows(workspace: &Value, school: &SchoolReads, identity: &Identity) -> usize {
    school
        .exams
        .of(&identity.term)
        .filter(|exam| {
            row_identity(exam, &identity.term) == *identity
                && existing_override(workspace, exam_key(exam)).is_none()
        })
        .count()
}

/// The correction that applies to this published exam: its own, or the one
/// it adopted from the row it replaced.
fn exam_override<'a>(workspace: &'a Value, school: &SchoolReads, key: &str) -> Option<&'a Value> {
    existing_override(workspace, key).or_else(|| {
        let (exam, term) = published_row(school, key)?;
        Orphans::read(workspace, school).adopted_by(workspace, school, exam, &term)
    })
}

/// The id of a correction this exam adopted from a row that left the
/// timetable, which a save replaces with one under the exam's own key.
pub(crate) fn adopted_exam_override(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
) -> Option<String> {
    exam_override(workspace, school, key)
        .filter(|row| text(row, "targetKey") != Some(key))
        .and_then(|row| text(row, "id"))
        .map(str::to_owned)
}

/// A corrected exam that left the timetable, rebuilt from the published facts
/// its correction recorded.
fn removed_view(change: &Value, term: &str, courses: &[&Value]) -> Option<ExamView> {
    let key = text(change, "targetKey")?;
    let base = change.get("base")?;
    let schedule = ExamSchedule {
        date: text(base, "date")?.into(),
        start_time: text(base, "startTime").map(str::to_owned),
        end_time: text(base, "endTime").map(str::to_owned),
        room: text(base, "room").unwrap_or_default().into(),
    };
    let course = text(base, "courseId").and_then(|id| {
        courses
            .iter()
            .copied()
            .find(|course| text(course, "id") == Some(id))
    });
    Some(ExamView {
        id: key.into(),
        key: key.into(),
        kind: text(base, "kind").unwrap_or("exam").into(),
        origin: "published".into(),
        term: term.into(),
        course_id: course.and_then(|row| text(row, "id")).map(str::to_owned),
        auto_linked: course.is_some(),
        code: text(base, "code").unwrap_or_default().into(),
        title: text(base, "title").unwrap_or_default().into(),
        date: schedule.date.clone(),
        start_time: schedule.start_time.clone(),
        end_time: schedule.end_time.clone(),
        periods: None,
        room: schedule.room.clone(),
        notes: None,
        color: course
            .map(display_color)
            .unwrap_or_else(|| "#c98245".into()),
        status: "scheduled".into(),
        source: schedule.clone(),
        edited: false,
        hidden: false,
        mark: None,
        conflict: Some(ExamConflict {
            base: schedule,
            removed: true,
        }),
    })
}

/// The exam as published, before the user's corrections: a row of the
/// timetable, or the last published facts of a corrected exam that left it.
fn published_source(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
) -> Result<ExamView, ScheduleError> {
    if let Some((exam, term)) = published_row(school, key) {
        return Ok(source_view(workspace, exam, &term));
    }
    existing_override(workspace, key)
        .filter(|row| text(row, "targetKind") == Some("exam"))
        .and_then(|row| {
            let term = row.pointer("/base/term").and_then(Value::as_str)?;
            removed_view(row, term, &term_courses(workspace, term))
        })
        .ok_or_else(|| ScheduleError::InvalidInput("Exam is no longer available.".into()))
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
        origin: "published".into(),
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
        conflict: None,
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
    let timed = if let (Some(start), Some(end)) = (filled("startTime"), filled("endTime")) {
        view.start_time = Some(start.into());
        view.end_time = Some(end.into());
        view.periods = None;
        true
    } else if let Some(range) = period_range(change, teaching) {
        let clock = clock(teaching, range);
        view.periods = Some(range);
        view.start_time = clock.as_ref().map(|pair| pair.0.clone());
        view.end_time = clock.map(|pair| pair.1);
        true
    } else {
        false
    };
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
        Some(kind @ ("exam" | "quiz" | "class")) => {
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
    // A corrected field conflicts when the published value moved away from
    // the one the user corrected and does not already agree with the user.
    if view.conflict.is_none() {
        if let Some(base) = change.get("base").and_then(base_schedule) {
            let source = &view.source;
            let date =
                filled("date").is_some() && base.date != source.date && source.date != view.date;
            let time = timed
                && (&base.start_time, &base.end_time) != (&source.start_time, &source.end_time)
                && (&source.start_time, &source.end_time) != (&view.start_time, &view.end_time);
            let room =
                filled("room").is_some() && base.room != source.room && source.room != view.room;
            if date || time || room {
                view.conflict = Some(ExamConflict {
                    base,
                    removed: false,
                });
            }
        }
    }
}

fn base_schedule(base: &Value) -> Option<ExamSchedule> {
    Some(ExamSchedule {
        date: text(base, "date")?.into(),
        start_time: text(base, "startTime").map(str::to_owned),
        end_time: text(base, "endTime").map(str::to_owned),
        room: text(base, "room").unwrap_or_default().into(),
    })
}

/// What a correction records about the exam it corrects: the published
/// schedule it was made against, and who the exam is, to follow it when the
/// timetable renames its row and to show it if the row is removed.
fn base_record(view: &ExamView, schedule: &ExamSchedule) -> Value {
    json!({
        "date": schedule.date,
        "startTime": schedule.start_time,
        "endTime": schedule.end_time,
        "room": schedule.room,
        "term": view.term,
        "code": view.code,
        "kind": if view.kind == "class" { "class" } else { "exam" },
        "title": view.title,
        "courseId": view.course_id,
    })
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

fn published_row<'a>(school: &'a SchoolReads, key: &str) -> Option<(&'a Value, String)> {
    school
        .exams
        .rows()
        .iter()
        .filter(|exam| exam_key(exam) == key)
        .find_map(|exam| Some((exam, exam_term(exam)?)))
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

fn keep_schedule_fields(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    row: &mut Map<String, Value>,
) {
    for field in ["date", "startTime", "endTime", "periods", "room"] {
        if let Some(value) = exam_override(workspace, school, key).and_then(|row| row.get(field)) {
            row.insert(field.into(), value.clone());
        }
    }
}

/// Identity and the fields the caller does not replace. The recorded
/// published schedule stays until the user edits the schedule itself, so a
/// conflict outlives marking or linking the exam.
fn override_base(
    workspace: &Value,
    school: &SchoolReads,
    key: &str,
    source: &ExamView,
) -> Map<String, Value> {
    let existing = exam_override(workspace, school, key);
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
    let schedule = existing
        .and_then(|row| row.get("base"))
        .and_then(base_schedule)
        .unwrap_or_else(|| source.source.clone());
    row.insert("base".into(), base_record(source, &schedule));
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

pub(super) fn exam_term(exam: &Value) -> Option<String> {
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
