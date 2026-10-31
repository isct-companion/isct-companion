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
pub(super) struct Meeting {
    /// 1 = Monday … 7 = Sunday.
    pub days: Vec<u8>,
    pub periods: Vec<u8>,
    /// HH:MM, Japan time.
    pub start_time: String,
    pub end_time: String,
    pub room: String,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub folder_files: Vec<FolderFile>,
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
        Ok(item)
    }
}

/// A weekly personal event.
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub(super) struct IntensiveDay {
    pub date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub periods: Option<(u8, u8)>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct IntensiveSchedule {
    pub course_id: String,
    pub from: String,
    pub to: String,
    pub periods: (u8, u8),
    #[serde(skip_serializing_if = "Option::is_none")]
    pub days: Option<Vec<IntensiveDay>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<String>,
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

/// A class actually held on a date.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ClassToday {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub course_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    pub room: String,
}

/// A weekly personal event falling on a date.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct EventToday {
    pub id: String,
    pub title: String,
    pub start_time: String,
    pub end_time: String,
    pub room: String,
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
