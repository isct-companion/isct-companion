//! What tools answer. Each type lists the fields an agent gets, read out of
//! a stored row by name: sync bookkeeping, and any field the store adds
//! later, never reaches a client unless it is named here. School HTML
//! arrives as plain text.
use super::AgentError;
use crate::html_text::plain_text;
use schemars::JsonSchema;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;

/// The named fields of `row`.
pub(super) fn pick<T: DeserializeOwned>(row: &Value) -> Result<T, AgentError> {
    T::deserialize(row).map_err(|error| AgentError::Compatibility(error.to_string()))
}

pub(super) fn pick_all<T: DeserializeOwned>(rows: &[Value]) -> Result<Vec<T>, AgentError> {
    rows.iter().map(pick).collect()
}

/// Text in the school's languages.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub(super) struct Text {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub en: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub(super) struct Credits {
    pub lecture: f64,
    pub exercise: f64,
    pub lab: f64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
/// A weekly meeting of a course: a template. It does not say what is held
/// on a given date (holidays, cancellations, moved classes, exams); the
/// `schedule` tool answers that.
pub(super) struct Meeting {
    /// 1 = Monday … 7 = Sunday.
    pub days: Vec<u8>,
    pub periods: Vec<u8>,
    /// HH:MM, Japan time.
    pub start_time: String,
    pub end_time: String,
    pub room: String,
    /// The quarters it applies to, when not every quarter of the course.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term_ids: Option<Vec<String>>,
    /// First date it applies, YYYY-MM-DD; absent means from the quarter's start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    /// Last date it applies; absent means to the quarter's end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Course {
    pub id: String,
    /// YYYY-nQ.
    pub term_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offering_id: Option<String>,
    pub code: String,
    pub title: String,
    pub instructor: String,
    pub credits: f64,
    pub instruction_language: String,
    pub meetings: Vec<Meeting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Assignment {
    pub id: String,
    pub course_id: String,
    pub title: String,
    /// The instructions, as plain text.
    pub details: String,
    /// RFC 3339 with offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// The LMS state: unknown, open, submitted or graded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submission_status: Option<String>,
    /// The grade as the LMS displays it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
}

impl Assignment {
    pub fn from_row(row: &Value) -> Result<Self, AgentError> {
        let mut assignment: Self = pick(row)?;
        assignment.details = plain_text(&assignment.details);
        Ok(assignment)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub(super) struct FolderFile {
    pub name: String,
    pub url: String,
}

/// One LMS item of a course: a page, file, link, folder or video.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct LmsItem {
    pub id: String,
    pub session: i32,
    pub title: String,
    /// pdf, link or video.
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_number: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_files: Option<Vec<FolderFile>>,
    /// The page body, as plain text.
    #[serde(alias = "bodyHtml", skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

impl LmsItem {
    pub fn from_row(row: &Value) -> Result<Self, AgentError> {
        let mut item: Self = pick(row)?;
        item.body = item
            .body
            .as_deref()
            .map(plain_text)
            .filter(|body| !body.is_empty());
        item.folder_files = item.folder_files.filter(|files| !files.is_empty());
        Ok(item)
    }
}

/// A weekly personal event: a template, like a course meeting.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Event {
    pub id: String,
    pub term_id: String,
    pub title: String,
    /// 1 = Monday … 7 = Sunday.
    pub days: Vec<u8>,
    pub start_time: String,
    pub end_time: String,
    pub room: String,
}

/// A date's school-calendar state: a personal exception, or a holiday,
/// no-class, break or replacement day.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct CalendarDay {
    pub date: String,
    #[serde(rename = "type")]
    pub day_type: String,
    /// Plain text, or `{ja, en}`.
    pub label: Value,
    /// The weekday whose timetable runs, 1 = Monday.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_day: Option<u8>,
}

/// An entry of the school's important-dates list covering a date.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct ImportantDate {
    pub start_date: String,
    pub end_date: String,
    pub label: Text,
    pub category: String,
}

/// One date as Core computes it for the app's calendar, with every personal
/// change applied: marks, cancellations, moved or edited classes, published
/// exams, holidays and replacement days.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Day {
    pub date: String,
    /// The weekly class pattern stops on this date (holiday, break, outside
    /// teaching weeks) and no class or exam is held. Personal events may
    /// still fall on it.
    pub no_classes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_notice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarDay>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub important_dates: Vec<ImportantDate>,
    pub occurrences: Vec<Occurrence>,
}

impl Day {
    pub fn from_agenda(agenda: crate::schedule::DatedAgenda) -> Result<Self, AgentError> {
        let crate::schedule::DatedAgenda {
            date,
            occurrences,
            calendar_state,
            important_dates,
            no_classes,
            calendar_notice,
        } = agenda;
        Ok(Self {
            date,
            no_classes,
            calendar_notice,
            calendar: calendar_state.as_ref().map(pick).transpose()?,
            important_dates: pick_all(&important_dates)?,
            occurrences: occurrences.into_iter().map(Occurrence::from).collect(),
        })
    }
}

/// One class, exam or personal event on one date.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Occurrence {
    /// This one session: stable across reads, and the key personal marks use.
    pub key: String,
    /// The course, personal event or exam record it comes from.
    pub id: String,
    /// course, exam (a published exam, or a class the person marked as an
    /// exam or quiz) or event (a weekly personal event).
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    pub date: String,
    pub title: String,
    /// The course code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub room: String,
    /// HH:MM, Japan time.
    pub start_time: String,
    pub end_time: String,
    /// Set when it ends on a later date than `date`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_date: Option<String>,
    /// Cancelled (休講) by the school or the person; kept so it can be
    /// explained, but it is not held.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cancelled: bool,
    /// An exam block the person marked as a quiz (小テスト).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub quiz: bool,
    /// The person changed its date, time or room.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub overridden: bool,
    /// The published exam changed after the person corrected it, or left the
    /// exam timetable; the person's values stay until they choose.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub source_changed: bool,
    /// A course only planned for its quarter, with no registration there.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub planned: bool,
    /// It overlaps another class or exam.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub conflict: bool,
    /// The keys of what it overlaps.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conflict_with: Vec<String>,
    /// 物理学演習・実験, which meets in turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternation: Option<Alternation>,
}

impl From<crate::schedule::Occurrence> for Occurrence {
    fn from(occurrence: crate::schedule::Occurrence) -> Self {
        // Every field is named, so a field Core adds stops the build until
        // it is shown here or left out on purpose.
        let crate::schedule::Occurrence {
            key,
            id,
            kind,
            course_id,
            term_id,
            date,
            title,
            subtitle,
            room,
            start_time,
            end_time,
            end_date,
            color: _,
            conflict,
            dim: _,
            cancelled,
            time_label: _,
            conflict_with,
            overridden,
            alternation,
            planned,
            quiz,
            source_changed,
            grid: _,
        } = occurrence;
        Self {
            key,
            id,
            kind: wire_name(&kind),
            course_id,
            term_id,
            date,
            title,
            code: subtitle,
            room,
            start_time,
            end_time,
            end_date,
            cancelled: cancelled == Some(true),
            quiz: quiz == Some(true),
            overridden: overridden == Some(true),
            source_changed: source_changed == Some(true),
            planned: planned == Some(true),
            conflict: conflict == Some(true),
            conflict_with: conflict_with.unwrap_or_default(),
            alternation: alternation.map(Alternation::from),
        }
    }
}

/// The turns of a 物理学演習・実験 class on one date.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Alternation {
    /// Who meets on this date: red, white or all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    /// Isct-Data publishes this class's sessions.
    pub scheduled: bool,
    /// The person's group, red or white, once chosen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// orientation, exercise, lab, exam or no-class.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The published session number, such as "3Q-1".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// The 演習 and 実験 sharing this block, exercise first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pair: Vec<PairedCourse>,
}

impl From<crate::schedule::Alternation> for Alternation {
    fn from(alternation: crate::schedule::Alternation) -> Self {
        let crate::schedule::Alternation {
            turn,
            scheduled,
            group,
            session,
            number,
            pair,
        } = alternation;
        Self {
            turn: turn.as_ref().map(wire_name),
            scheduled,
            group: group.as_ref().map(wire_name),
            session,
            number,
            pair: pair
                .unwrap_or_default()
                .into_iter()
                .map(PairedCourse::from)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct PairedCourse {
    pub course_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub room: String,
    /// This course's own meeting, which may be shorter than the block.
    pub start_time: String,
    pub end_time: String,
}

impl From<crate::schedule::PairedCourse> for PairedCourse {
    fn from(course: crate::schedule::PairedCourse) -> Self {
        let crate::schedule::PairedCourse {
            course_id,
            title,
            code,
            room,
            color: _,
            start_time,
            end_time,
        } = course;
        Self {
            course_id,
            title,
            code,
            room,
            start_time,
            end_time,
        }
    }
}

/// A stored plan or override Core could not apply. The rest of the schedule
/// is computed without it, so an answer with problems is still complete for
/// everything else.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScheduleProblem {
    /// courseMeeting or scheduleOverride.
    pub kind: String,
    pub id: String,
    pub title: String,
    pub message: String,
}

impl From<crate::schedule::ScheduleProblem> for ScheduleProblem {
    fn from(problem: crate::schedule::ScheduleProblem) -> Self {
        let crate::schedule::ScheduleProblem {
            kind,
            id,
            title,
            message,
        } = problem;
        Self {
            kind: wire_name(&kind),
            id,
            title,
            message,
        }
    }
}

/// One exam or quiz of a quarter, from the unified exam list: the exam
/// timetable with the person's corrections, and the class sessions the
/// person marked as an exam or quiz. Entries without a course or a time are
/// kept; the calendar cannot place them, but they exist.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct Exam {
    pub id: String,
    /// The exam's key; a marked session's key is the session's.
    pub key: String,
    /// exam, quiz, or class (a class the exam timetable lists in the exam
    /// period).
    pub kind: String,
    /// published (the exam timetable) or session (a marked class).
    pub origin: String,
    pub term: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
    /// The course came from code matching, not from the person's link.
    pub auto_linked: bool,
    pub code: String,
    pub title: String,
    /// The date in effect, after the person's correction.
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub periods: Option<[u8; 2]>,
    pub room: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// scheduled or cancelled.
    pub status: String,
    /// The published facts, before the person's corrections.
    pub source: ExamSchedule,
    /// Date, time, room or title differs from the published record.
    pub edited: bool,
    /// The person's mark (exam, quiz, cancelled, class), which outranks the
    /// published kind and status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    /// The published record moved away from what the person corrected, or
    /// left the timetable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ExamConflict>,
}

impl From<crate::schedule::ExamView> for Exam {
    fn from(exam: crate::schedule::ExamView) -> Self {
        let crate::schedule::ExamView {
            id,
            key,
            kind,
            origin,
            term,
            course_id,
            auto_linked,
            code,
            title,
            date,
            start_time,
            end_time,
            periods,
            room,
            notes,
            color: _,
            status,
            source,
            edited,
            mark,
            conflict,
        } = exam;
        Self {
            id,
            key,
            kind,
            origin,
            term,
            course_id,
            auto_linked,
            code,
            title,
            date,
            start_time,
            end_time,
            periods,
            room,
            notes,
            status,
            source: source.into(),
            edited,
            mark,
            conflict: conflict.map(|conflict| {
                let crate::schedule::ExamConflict { base, removed } = conflict;
                ExamConflict {
                    base: base.into(),
                    removed,
                }
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExamSchedule {
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_time: Option<String>,
    pub room: String,
}

impl From<crate::schedule::ExamSchedule> for ExamSchedule {
    fn from(schedule: crate::schedule::ExamSchedule) -> Self {
        let crate::schedule::ExamSchedule {
            date,
            start_time,
            end_time,
            room,
        } = schedule;
        Self {
            date,
            start_time,
            end_time,
            room,
        }
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExamConflict {
    /// The published facts when the person last corrected the exam.
    pub base: ExamSchedule,
    /// The exam timetable no longer lists this exam.
    pub removed: bool,
}

/// The name a Core enum serializes as.
fn wire_name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        _ => String::new(),
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct GpaSummary {
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpa: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpt: Option<f64>,
    pub earned_credits: f64,
}

/// One row of the 教務 GPA-by-term table, cell by cell.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub(super) struct GpaTerm {
    pub cells: Value,
}

/// One cell of the 教務 graduation requirement tables.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub(super) struct Requirement {
    pub table: u32,
    pub category: String,
    pub label: String,
    pub value: Value,
}

/// One 教務 grade row; undergraduate and graduate pages fill different
/// fields.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Grade {
    pub category: Text,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_credits: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<String>,
}

/// One 教務 registration or declared-credit row.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Registration {
    pub offering_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub term_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quarters: Option<Vec<u8>>,
    /// Quarter, weekday and periods as the page lists them.
    #[serde(skip_serializing_if = "Value::is_null")]
    pub schedule: Value,
    /// Requirement category and mark (required, recommended, elective).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub requirement: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub unit: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_total: Option<f64>,
}

/// A course offering in the local catalog.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Offering {
    pub id: String,
    pub year: i32,
    pub quarters: Vec<String>,
    pub code: String,
    pub title: Text,
    pub instructors: Vec<Text>,
    pub instruction_language: String,
    pub credits: Credits,
    /// Weekly slots, intensive dates or registration-only rows.
    pub schedule: Value,
    pub intensive: bool,
    pub syllabus_url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct RecommendedCourse {
    pub admission_year: i32,
    pub department_id: String,
    pub category: Text,
    pub level: i32,
    pub code: String,
    pub group: String,
    /// required or recommended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    pub title: Text,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}
