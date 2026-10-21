use crate::data::{canonical_course_id, versioned::content_hash, DataError};
use chrono::{Datelike, NaiveDate, NaiveTime, Utc};
use reqwest::{Client, Url};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum SubscriptionError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Storage(String),
}

impl From<String> for SubscriptionError {
    fn from(message: String) -> Self {
        Self::Validation(message)
    }
}

impl From<&str> for SubscriptionError {
    fn from(message: &str) -> Self {
        Self::Validation(message.to_string())
    }
}

impl From<DataError> for SubscriptionError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<rusqlite::Error> for SubscriptionError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for SubscriptionError {
    fn from(error: serde_json::Error) -> Self {
        Self::Validation(error.to_string())
    }
}

impl From<reqwest::Error> for SubscriptionError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.to_string())
    }
}

const MAX_INDEX_BYTES: usize = 128 * 1024;
const MAX_DATA_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLink {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Localized {
    pub ja: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub en: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum CalendarDayType {
    Holiday,
    NoClass,
    Replacement,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CalendarDay {
    pub date: String,
    #[serde(rename = "type")]
    pub day_type: CalendarDayType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_day: Option<u8>,
    pub label: Localized,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcademicCalendar {
    pub schema_version: u32,
    pub academic_year: i32,
    pub source: Vec<SourceLink>,
    pub quarters: BTreeMap<String, DateRange>,
    pub days: Vec<CalendarDay>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub teaching_ranges: BTreeMap<String, Vec<DateRange>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exam_ranges: BTreeMap<String, Vec<DateRange>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Credits {
    pub lecture: f64,
    pub exercise: f64,
    pub lab: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RecommendationMark {
    Required,
    Recommended,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecommendedCourse {
    pub category: Localized,
    pub level: u16,
    pub code: String,
    #[serde(default)]
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<RecommendationMark>,
    pub title: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<Credits>,
    #[serde(default)]
    pub competencies: Vec<u8>,
    #[serde(default)]
    pub content: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_pages: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Department {
    pub id: String,
    pub name: Localized,
    pub syllabus_scope: String,
    pub unit_code: String,
    pub guide_pdf: String,
    pub recommended: Vec<RecommendedCourse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Departments {
    pub schema_version: u32,
    pub admission_year: i32,
    pub source: Vec<SourceLink>,
    pub departments: Vec<Department>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedSource {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing_page_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportantDateEntry {
    pub id: String,
    pub start_date: String,
    pub end_date: String,
    pub label: Localized,
    pub category: String,
    pub source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_english_source: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportantDates {
    pub schema_version: u32,
    pub academic_year: i32,
    pub timezone: String,
    pub source: Vec<FeedSource>,
    pub entries: Vec<ImportantDateEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExamEntry {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub quarter: String,
    pub course_code: String,
    pub date: String,
    pub label: Localized,
    pub source_ids: Vec<String>,
    pub room: Option<Localized>,
    #[serde(default)]
    pub periods: Option<Vec<u8>>,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub syllabus_course_id: Option<String>,
    #[serde(default)]
    pub class_label: Option<Localized>,
    #[serde(default)]
    pub notes: Option<Localized>,
    #[serde(default)]
    pub source_pages: Vec<u32>,
    #[serde(default)]
    pub rooms: Vec<Localized>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExamFeed {
    pub schema_version: u32,
    pub academic_year: i32,
    pub timezone: String,
    pub source: Vec<FeedSource>,
    pub entries: Vec<ExamEntry>,
}

/// The two datasets cut from the central exam timetable: its final-exam rows,
/// and its class rows (授業, including make-up classes) in the exam period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExamFeedKind {
    Exams,
    ExamWeekClasses,
}

impl ExamFeedKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Exams => "exam",
            Self::ExamWeekClasses => "exam-week-classes",
        }
    }
    /// The `kind` every entry of this dataset carries.
    pub fn entry_kind(self) -> &'static str {
        match self {
            Self::Exams => "exam",
            Self::ExamWeekClasses => "class",
        }
    }
    fn row_prefix(self) -> &'static str {
        match self {
            Self::Exams => "exam",
            Self::ExamWeekClasses => "exam-week-class",
        }
    }
    fn source_key_prefix(self) -> &'static str {
        match self {
            Self::Exams => "reference:exams",
            Self::ExamWeekClasses => "reference:exam-week-classes",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PublishedExamFeed {
    pub kind: ExamFeedKind,
    pub level: String,
    pub quarter: String,
    pub url: String,
    pub feed: ExamFeed,
}

/// One dated session of a course that does not meet every week.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassSession {
    pub date: String,
    /// `orientation` and `exam` are for every group, `no-class` cancels the
    /// slot, `exercise` and `lab` are for `group` only.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassScheduleEntry {
    pub id: String,
    pub course_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_label: Option<Localized>,
    pub offering_ids: Vec<String>,
    pub quarters: Vec<String>,
    pub source_ids: Vec<String>,
    pub sessions: Vec<ClassSession>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClassSchedules {
    pub schema_version: u32,
    pub academic_year: i32,
    pub timezone: String,
    pub source: Vec<FeedSource>,
    pub entries: Vec<ClassScheduleEntry>,
}

// Index entries ignore keys they do not know, so publishing a new dataset
// kind does not break apps that predate it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcademicYearEntry {
    academic_calendar: IndexedSource,
    #[serde(default)]
    exams: Vec<IndexedSource>,
    #[serde(default)]
    exam_week_classes: Vec<IndexedSource>,
    #[serde(default)]
    important_dates: Option<IndexedSource>,
    #[serde(default)]
    class_schedules: Option<IndexedSource>,
}

#[derive(Clone, Debug, Deserialize)]
struct AdmissionYearEntry {
    departments: IndexedSource,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexedSource {
    source: String,
    hash: String,
    ts: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionIndex {
    #[serde(default)]
    academic_years: BTreeMap<String, AcademicYearEntry>,
    #[serde(default)]
    admission_years: BTreeMap<String, AdmissionYearEntry>,
}

#[derive(Clone, Debug)]
pub struct ReferenceFetch<T> {
    pub value: Option<T>,
    pub data_url: String,
    pub remote_hash: String,
    pub source_ts: String,
}

fn validate_indexed_source(source: &IndexedSource) -> Result<(), SubscriptionError> {
    if source.hash.len() != 64 || !source.hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("subscription index hash must be a SHA-256 hex digest".into());
    }
    chrono::DateTime::parse_from_rfc3339(&source.ts)
        .map_err(|_| "subscription index ts must be RFC 3339".to_string())?;
    Ok(())
}

fn source_url(base: &Url, source: &IndexedSource) -> Result<Url, SubscriptionError> {
    validate_indexed_source(source)?;
    https_url(
        base.join(&source.source)
            .map_err(|_| "invalid subscription source path")?
            .as_str(),
    )
}

fn exam_manifest_hash(sources: &[IndexedSource]) -> Result<String, SubscriptionError> {
    content_hash(&Value::Array(
        sources
            .iter()
            .map(|source| json!({ "source": source.source, "hash": source.hash }))
            .collect(),
    ))
    .map_err(SubscriptionError::from)
}

fn verify_source_hash(raw: &str, expected: &str) -> Result<(), SubscriptionError> {
    let value: Value =
        serde_json::from_str(raw).map_err(|error| format!("invalid subscription JSON: {error}"))?;
    let actual = content_hash(&value)?;
    if actual != expected.to_ascii_lowercase() {
        return Err("subscription data does not match the hash declared by index.json".into());
    }
    Ok(())
}

fn https_url(raw: &str) -> Result<Url, SubscriptionError> {
    let url = Url::parse(raw).map_err(|_| format!("invalid HTTPS URL: {raw}"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(format!("invalid HTTPS URL: {raw}").into());
    }
    Ok(url)
}

fn date(raw: &str) -> Result<NaiveDate, SubscriptionError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| SubscriptionError::from(format!("invalid date: {raw}")))
}

fn validate_sources(sources: &[SourceLink]) -> Result<(), SubscriptionError> {
    if sources.is_empty() {
        return Err("source must contain at least one citation".into());
    }
    for source in sources {
        if source.title.trim().is_empty() {
            return Err("source title is empty".into());
        }
        https_url(&source.url)?;
    }
    Ok(())
}

pub fn validate_academic_calendar(raw: &str) -> Result<AcademicCalendar, SubscriptionError> {
    if raw.len() > MAX_DATA_BYTES {
        return Err("academic calendar exceeds the size limit".into());
    }
    let value: AcademicCalendar =
        serde_json::from_str(raw).map_err(|e| format!("invalid academic calendar: {e}"))?;
    if !matches!(value.schema_version, 1 | 2) {
        return Err("academic calendar schemaVersion must be 1 or 2".into());
    }
    if !(2000..=2200).contains(&value.academic_year) {
        return Err("academicYear is out of range".into());
    }
    validate_sources(&value.source)?;
    let expected = ["1Q", "2Q", "3Q", "4Q"];
    if value.quarters.len() != 4 || expected.iter().any(|q| !value.quarters.contains_key(*q)) {
        return Err("quarters must contain exactly 1Q through 4Q".into());
    }
    let mut prior = None;
    for quarter in expected {
        let range = &value.quarters[quarter];
        let start = date(&range.start)?;
        let end = date(&range.end)?;
        if start.year() != value.academic_year || end < start {
            return Err(format!("invalid {quarter} range").into());
        }
        if prior.is_some_and(|end| start <= end) {
            return Err("quarter ranges overlap or are unordered".into());
        }
        prior = Some(end);
    }
    if value.schema_version == 1
        && (!value.teaching_ranges.is_empty() || !value.exam_ranges.is_empty())
    {
        return Err("teachingRanges and examRanges require schemaVersion 2".into());
    }
    if value.schema_version == 2
        && (value.teaching_ranges.len() != 4
            || expected
                .iter()
                .any(|q| !value.teaching_ranges.contains_key(*q)))
    {
        return Err("calendar v2 needs teachingRanges for all four quarters".into());
    }
    for ranges in [&value.teaching_ranges, &value.exam_ranges] {
        for (quarter, items) in ranges {
            let bounds = value
                .quarters
                .get(quarter)
                .ok_or("range has an unknown quarter")?;
            let mut prior: Option<NaiveDate> = None;
            for item in items {
                let start = date(&item.start)?;
                let end = date(&item.end)?;
                if start > end
                    || start < date(&bounds.start)?
                    || end > date(&bounds.end)?
                    || prior.is_some_and(|p| start <= p)
                {
                    return Err("invalid, overlapping, or unordered teaching/exam ranges".into());
                }
                prior = Some(end);
            }
        }
    }
    let mut dates = HashSet::new();
    for day in &value.days {
        let parsed = date(&day.date)?;
        if parsed.year() != value.academic_year && parsed.year() != value.academic_year + 1 {
            return Err("calendar day is outside the academic year".into());
        }
        if !dates.insert(&day.date) {
            return Err(format!("duplicate calendar day: {}", day.date).into());
        }
        match day.day_type {
            CalendarDayType::Replacement if !matches!(day.replacement_day, Some(1..=7)) => {
                return Err("replacement day requires replacementDay 1 through 7".into())
            }
            CalendarDayType::Replacement => {}
            _ if day.replacement_day.is_some() => {
                return Err("replacementDay is only valid for replacement days".into())
            }
            _ => {}
        }
        if day.label.ja.trim().is_empty() {
            return Err("calendar day Japanese label is empty".into());
        }
    }
    Ok(value)
}

pub fn validate_departments(raw: &str) -> Result<Departments, SubscriptionError> {
    if raw.len() > MAX_DATA_BYTES {
        return Err("departments data exceeds the size limit".into());
    }
    let value: Departments =
        serde_json::from_str(raw).map_err(|e| format!("invalid departments: {e}"))?;
    if value.schema_version != 1 {
        return Err("departments schemaVersion must be 1".into());
    }
    if !(2000..=2200).contains(&value.admission_year) {
        return Err("admissionYear is out of range".into());
    }
    validate_sources(&value.source)?;
    let guide_prefix = format!("/guide/guide_{}/gakubu1/", value.admission_year);
    if !value.source.iter().any(|source| {
        Url::parse(&source.url).is_ok_and(|url| {
            url.host_str() == Some("www.titech.ac.jp") && url.path().starts_with(&guide_prefix)
        })
    }) {
        return Err("department source does not match admissionYear".into());
    }
    if value.departments.is_empty() {
        return Err("departments must not be empty".into());
    }
    let mut ids = HashSet::new();
    let mut units = HashSet::new();
    for department in &value.departments {
        if !ids.insert(&department.id)
            || department.id.is_empty()
            || !department
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'-')
        {
            return Err("department id is invalid or duplicated".into());
        }
        if department.name.ja.trim().is_empty()
            || department.name.en.as_deref().is_none_or(str::is_empty)
        {
            return Err("department names require ja and en".into());
        }
        if department.syllabus_scope.split('/').count() != 2
            || !department.unit_code.bytes().all(|b| b.is_ascii_digit())
            || department.unit_code.len() != 6
            || !units.insert(&department.unit_code)
        {
            return Err("department scope or unitCode is invalid".into());
        }
        https_url(&department.guide_pdf)?;
        if !Url::parse(&department.guide_pdf).is_ok_and(|url| {
            url.host_str() == Some("www.titech.ac.jp") && url.path().starts_with(&guide_prefix)
        }) {
            return Err("department guidePdf does not match admissionYear".into());
        }
        let mut courses = HashSet::new();
        for course in &department.recommended {
            if course.title.ja.trim().is_empty() || course.category.ja.trim().is_empty() {
                return Err("recommended course title and category require Japanese text".into());
            }
            if !(100..=900).contains(&course.level) || course.code.trim().is_empty() {
                return Err("recommended course identity is invalid".into());
            }
            if !courses.insert((course.code.as_str(), course.group.as_str())) {
                return Err("recommended course is duplicated".into());
            }
            if let Some(credits) = &course.credits {
                if !credits.lecture.is_finite()
                    || !credits.exercise.is_finite()
                    || !credits.lab.is_finite()
                    || credits.lecture < 0.0
                    || credits.exercise < 0.0
                    || credits.lab < 0.0
                {
                    return Err("recommended course credits are invalid".into());
                }
            }
            if course.source_pages.contains(&0)
                || course.source_kind.as_ref().is_some_and(|kind| {
                    !matches!(
                        kind.as_str(),
                        "curriculum-map" | "standard-plan" | "course-table"
                    )
                })
            {
                return Err("recommended course source is invalid".into());
            }
        }
    }
    Ok(value)
}

fn validate_feed_sources(sources: &[FeedSource]) -> Result<HashSet<&str>, SubscriptionError> {
    if sources.is_empty() {
        return Err("source must contain at least one citation".into());
    }
    let mut ids = HashSet::new();
    for source in sources {
        if source.id.trim().is_empty()
            || source.title.trim().is_empty()
            || !ids.insert(source.id.as_str())
        {
            return Err("feed source identity is empty or duplicated".into());
        }
        https_url(&source.url)?;
        if let Some(url) = &source.landing_page_url {
            https_url(url)?;
        }
        if source.sha256.as_ref().is_some_and(|hash| {
            hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) {
            return Err("feed source sha256 is invalid".into());
        }
    }
    Ok(ids)
}

fn date_in_academic_year(value: NaiveDate, year: i32) -> bool {
    NaiveDate::from_ymd_opt(year, 4, 1)
        .zip(NaiveDate::from_ymd_opt(year + 1, 3, 31))
        .is_some_and(|(start, end)| value >= start && value <= end)
}

fn object_mut<'a>(
    value: &'a mut Value,
    context: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, SubscriptionError> {
    value.as_object_mut().ok_or_else(|| {
        SubscriptionError::Storage(format!("{context} did not serialize as an object"))
    })
}

pub fn validate_important_dates(raw: &str, year: i32) -> Result<ImportantDates, SubscriptionError> {
    if raw.len() > MAX_DATA_BYTES {
        return Err("important dates exceed the size limit".into());
    }
    let value: ImportantDates =
        serde_json::from_str(raw).map_err(|e| format!("invalid important dates: {e}"))?;
    if value.schema_version != 1 || value.academic_year != year || value.timezone != "Asia/Tokyo" {
        return Err("important dates metadata does not match the selection".into());
    }
    let sources = validate_feed_sources(&value.source)?;
    let mut ids = HashSet::new();
    for entry in &value.entries {
        let start = date(&entry.start_date)?;
        let end = date(&entry.end_date)?;
        if !ids.insert(entry.id.as_str())
            || entry.id.trim().is_empty()
            || start > end
            || !date_in_academic_year(start, year)
            || !date_in_academic_year(end, year)
            || entry.label.ja.trim().is_empty()
            || !matches!(
                entry.category.as_str(),
                "campus-festival" | "ceremony" | "academic" | "other"
            )
            || entry.source_ids.is_empty()
            || entry
                .source_ids
                .iter()
                .any(|id| !sources.contains(id.as_str()))
            || entry
                .label_english_source
                .as_deref()
                .is_some_and(|v| !matches!(v, "official" | "translation"))
        {
            return Err(format!("important date {} is invalid", entry.id).into());
        }
    }
    Ok(value)
}

pub fn validate_class_schedules(raw: &str, year: i32) -> Result<ClassSchedules, SubscriptionError> {
    if raw.len() > MAX_DATA_BYTES {
        return Err("class schedules exceed the size limit".into());
    }
    let value: ClassSchedules =
        serde_json::from_str(raw).map_err(|e| format!("invalid class schedules: {e}"))?;
    if value.schema_version != 1 || value.academic_year != year || value.timezone != "Asia/Tokyo" {
        return Err("class schedules metadata does not match the selection".into());
    }
    let sources = validate_feed_sources(&value.source)?;
    let mut ids = HashSet::new();
    for entry in &value.entries {
        let invalid = || SubscriptionError::from(format!("class schedule {} is invalid", entry.id));
        if !ids.insert(entry.id.as_str())
            || entry.id.trim().is_empty()
            || entry.course_code.trim().is_empty()
            || entry.offering_ids.is_empty()
            || entry
                .offering_ids
                .iter()
                .any(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()))
            || entry.quarters.is_empty()
            || entry
                .quarters
                .iter()
                .any(|quarter| !matches!(quarter.as_str(), "1Q" | "2Q" | "3Q" | "4Q"))
            || entry.source_ids.is_empty()
            || entry
                .source_ids
                .iter()
                .any(|id| !sources.contains(id.as_str()))
            || entry.sessions.is_empty()
        {
            return Err(invalid());
        }
        let mut previous = None;
        for session in &entry.sessions {
            let day = date(&session.date)?;
            let group_ok = match session.kind.as_str() {
                "exercise" | "lab" => matches!(session.group.as_deref(), Some("red" | "white")),
                "orientation" | "exam" | "no-class" => session.group.is_none(),
                _ => false,
            };
            // Sessions are listed once each, in date order.
            if !group_ok || !date_in_academic_year(day, year) || previous.is_some_and(|p| p >= day)
            {
                return Err(invalid());
            }
            previous = Some(day);
        }
    }
    Ok(value)
}

pub fn validate_exam_feed(
    raw: &str,
    year: i32,
    quarter: &str,
    kind: ExamFeedKind,
) -> Result<ExamFeed, SubscriptionError> {
    if raw.len() > MAX_DATA_BYTES {
        return Err("exam data exceeds the size limit".into());
    }
    let value: ExamFeed = serde_json::from_str(raw).map_err(|e| format!("invalid exams: {e}"))?;
    if value.schema_version != 1 || value.academic_year != year || value.timezone != "Asia/Tokyo" {
        return Err("exam metadata does not match the selection".into());
    }
    let sources = validate_feed_sources(&value.source)?;
    let mut ids = HashSet::new();
    for entry in &value.entries {
        let exam_date = date(&entry.date)?;
        let exact = match (&entry.start_time, &entry.end_time) {
            (Some(start), Some(end)) => NaiveTime::parse_from_str(start, "%H:%M")
                .ok()
                .zip(NaiveTime::parse_from_str(end, "%H:%M").ok())
                .is_some_and(|(a, b)| a < b),
            (None, None) => false,
            _ => return Err(format!("exam {} has an incomplete exact time", entry.id).into()),
        };
        let periods = entry.periods.as_ref().is_some_and(|p| {
            p.len() == 2 && crate::schedule::validate_period_range([p[0], p[1]]).is_ok()
        });
        if !ids.insert(entry.id.as_str())
            || entry.id.trim().is_empty()
            || entry.kind != kind.entry_kind()
            || (kind == ExamFeedKind::ExamWeekClasses && !periods)
            || entry.quarter.to_ascii_lowercase() != quarter
            || entry.course_code.trim().is_empty()
            || entry.label.ja.trim().is_empty()
            || !date_in_academic_year(exam_date, year)
            || !matches!(entry.status.as_str(), "scheduled" | "cancelled")
            || (!periods && !exact)
            || entry.source_ids.is_empty()
            || entry
                .source_ids
                .iter()
                .any(|id| !sources.contains(id.as_str()))
            || entry.source_pages.contains(&0)
        {
            return Err(format!("exam {} is invalid", entry.id).into());
        }
    }
    Ok(value)
}

fn exam_scope(path: &str, year: i32, kind: ExamFeedKind) -> Option<(String, String)> {
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 3
        || parts[0] != kind.directory()
        || !matches!(parts[1], "undergraduate" | "graduate")
    {
        return None;
    }
    let stem = parts[2].strip_suffix(".json")?;
    let (raw_year, quarter) = stem.split_once('-')?;
    if raw_year.parse::<i32>().ok()? != year || !matches!(quarter, "1q" | "2q" | "3q" | "4q") {
        return None;
    }
    Some((parts[1].to_string(), quarter.to_string()))
}

pub struct SubscriptionClient {
    client: Client,
}

impl SubscriptionClient {
    pub fn new() -> Result<Self, SubscriptionError> {
        let client = crate::net::client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(SubscriptionError::from)?;
        Ok(Self { client })
    }
    /// Reads and validates the single repository entry point. Its raw hash is
    /// used to skip dataset requests when no metadata changed.
    pub async fn probe_index_source(&self, index_url: &str) -> Result<String, SubscriptionError> {
        let (_, _, hash) = self.fetch_index(index_url, None).await?;
        Ok(hash)
    }
    async fn get(&self, url: Url, limit: usize) -> Result<String, SubscriptionError> {
        let mut response = self
            .client
            .get(url.clone())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(SubscriptionError::from)?;
        if !response.status().is_success() {
            return Err(SubscriptionError::Network(format!(
                "subscription GET failed: {}",
                response.status()
            )));
        }
        if response.url().scheme() != "https" {
            return Err("subscription redirect left HTTPS".into());
        }
        if let Some(length) = response.content_length() {
            if length as usize > limit {
                return Err("subscription response exceeds the size limit".into());
            }
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(SubscriptionError::from)? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err("subscription response exceeds the size limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| "subscription response is not UTF-8".into())
    }

    async fn fetch_index(
        &self,
        index_url: &str,
        expected_hash: Option<&str>,
    ) -> Result<(Url, SubscriptionIndex, String), SubscriptionError> {
        let base = https_url(index_url)?;
        let raw = self.get(base.clone(), MAX_INDEX_BYTES).await?;
        let actual = format!("{:x}", Sha256::digest(raw.as_bytes()));
        if let Some(expected) = expected_hash {
            if actual != expected.to_ascii_lowercase() {
                return Err("index.json changed while the refresh was running".into());
            }
        }
        let index = serde_json::from_str(&raw)
            .map_err(|error| format!("invalid subscription index: {error}"))?;
        Ok((base, index, actual))
    }

    pub async fn fetch_calendar(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<AcademicCalendar>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = &index
            .academic_years
            .get(&year.to_string())
            .ok_or("calendar year is not published in this index")?
            .academic_calendar;
        let url = source_url(&base, source)?;
        if known_hash == Some(source.hash.as_str()) {
            return Ok(ReferenceFetch {
                value: None,
                data_url: url.to_string(),
                remote_hash: source.hash.clone(),
                source_ts: source.ts.clone(),
            });
        }
        let raw = self.get(url.clone(), MAX_DATA_BYTES).await?;
        verify_source_hash(&raw, &source.hash)?;
        let calendar = validate_academic_calendar(&raw)?;
        if calendar.academic_year != year {
            return Err("calendar year does not match the selection".into());
        }
        Ok(ReferenceFetch {
            value: Some(calendar),
            data_url: url.to_string(),
            remote_hash: source.hash.clone(),
            source_ts: source.ts.clone(),
        })
    }
    pub async fn fetch_departments(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<Departments>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = &index
            .admission_years
            .get(&year.to_string())
            .ok_or("admission year is not published in this index")?
            .departments;
        let url = source_url(&base, source)?;
        if known_hash == Some(source.hash.as_str()) {
            return Ok(ReferenceFetch {
                value: None,
                data_url: url.to_string(),
                remote_hash: source.hash.clone(),
                source_ts: source.ts.clone(),
            });
        }
        let raw = self.get(url.clone(), MAX_DATA_BYTES).await?;
        verify_source_hash(&raw, &source.hash)?;
        let departments = validate_departments(&raw)?;
        if departments.admission_year != year {
            return Err("guide year does not match the selection".into());
        }
        Ok(ReferenceFetch {
            value: Some(departments),
            data_url: url.to_string(),
            remote_hash: source.hash.clone(),
            source_ts: source.ts.clone(),
        })
    }
    pub async fn fetch_important_dates(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<ImportantDates>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let source = index
            .academic_years
            .get(&year.to_string())
            .and_then(|entry| entry.important_dates.as_ref())
            .ok_or("important dates are not published for this year")?;
        let url = source_url(&base, source)?;
        if known_hash == Some(source.hash.as_str()) {
            return Ok(ReferenceFetch {
                value: None,
                data_url: url.to_string(),
                remote_hash: source.hash.clone(),
                source_ts: source.ts.clone(),
            });
        }
        let raw = self.get(url.clone(), MAX_DATA_BYTES).await?;
        verify_source_hash(&raw, &source.hash)?;
        let value = validate_important_dates(&raw, year)?;
        Ok(ReferenceFetch {
            value: Some(value),
            data_url: url.to_string(),
            remote_hash: source.hash.clone(),
            source_ts: source.ts.clone(),
        })
    }

    /// A year without published class schedules yields an empty feed, so
    /// rows of a withdrawn file are marked missing instead of lingering.
    pub async fn fetch_class_schedules(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<ClassSchedules>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let Some(source) = index
            .academic_years
            .get(&year.to_string())
            .ok_or("academic year is not published in this index")?
            .class_schedules
            .as_ref()
        else {
            let remote_hash = content_hash(&json!({"classSchedules": null, "year": year}))?;
            return Ok(ReferenceFetch {
                value: (known_hash != Some(remote_hash.as_str())).then(|| ClassSchedules {
                    schema_version: 1,
                    academic_year: year,
                    timezone: "Asia/Tokyo".into(),
                    source: Vec::new(),
                    entries: Vec::new(),
                }),
                data_url: index_url.to_string(),
                remote_hash,
                source_ts: String::new(),
            });
        };
        let url = source_url(&base, source)?;
        if known_hash == Some(source.hash.as_str()) {
            return Ok(ReferenceFetch {
                value: None,
                data_url: url.to_string(),
                remote_hash: source.hash.clone(),
                source_ts: source.ts.clone(),
            });
        }
        let raw = self.get(url.clone(), MAX_DATA_BYTES).await?;
        verify_source_hash(&raw, &source.hash)?;
        let value = validate_class_schedules(&raw, year)?;
        Ok(ReferenceFetch {
            value: Some(value),
            data_url: url.to_string(),
            remote_hash: source.hash.clone(),
            source_ts: source.ts.clone(),
        })
    }

    pub async fn fetch_exams(
        &self,
        index_url: &str,
        year: i32,
        expected_index_hash: Option<&str>,
        known_hash: Option<&str>,
    ) -> Result<ReferenceFetch<Vec<PublishedExamFeed>>, SubscriptionError> {
        let (base, index, _) = self.fetch_index(index_url, expected_index_hash).await?;
        let entry = index
            .academic_years
            .get(&year.to_string())
            .ok_or("academic year is not published in this index")?;
        // Class rows share the exam subscription; with none published the
        // combined hash equals the exams-only hash older apps stored.
        let selected: Vec<_> = entry
            .exams
            .iter()
            .map(|source| (ExamFeedKind::Exams, source))
            .chain(
                entry
                    .exam_week_classes
                    .iter()
                    .map(|source| (ExamFeedKind::ExamWeekClasses, source)),
            )
            .collect();
        let sources: Vec<_> = selected
            .iter()
            .map(|(_, source)| (*source).clone())
            .collect();
        for source in &sources {
            validate_indexed_source(source)?;
        }
        let remote_hash = exam_manifest_hash(&sources)?;
        let source_ts = sources
            .iter()
            .map(|source| source.ts.as_str())
            .max()
            .unwrap_or_default()
            .to_string();
        if known_hash == Some(remote_hash.as_str()) {
            return Ok(ReferenceFetch {
                value: None,
                data_url: index_url.to_string(),
                remote_hash,
                source_ts,
            });
        }
        let mut feeds = Vec::new();
        for (kind, source) in selected {
            let (level, quarter) = exam_scope(&source.source, year, kind).ok_or_else(|| {
                format!("invalid exam path in subscription index: {}", source.source)
            })?;
            let url = source_url(&base, source)?;
            let raw = self.get(url.clone(), MAX_DATA_BYTES).await?;
            verify_source_hash(&raw, &source.hash)?;
            let feed = validate_exam_feed(&raw, year, &quarter, kind)?;
            feeds.push(PublishedExamFeed {
                kind,
                level,
                quarter,
                url: url.to_string(),
                feed,
            });
        }
        Ok(ReferenceFetch {
            value: Some(feeds),
            data_url: index_url.to_string(),
            remote_hash,
            source_ts,
        })
    }
}

fn apply_row(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    source_hash: &str,
) -> Result<bool, SubscriptionError> {
    let hash = content_hash(value)?;
    let old: Option<(String, u64, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash,rev,missing_since FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(SubscriptionError::from)?;
    let timestamp = Utc::now().to_rfc3339();
    if old.as_ref().is_some_and(|(h, _, _)| h == &hash) {
        let changed = old
            .as_ref()
            .is_some_and(|(_, _, missing)| missing.is_some());
        tx.execute(&format!("UPDATE {table} SET last_seen_at=?2,missing_since=NULL,deleted_at=NULL,
            value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"), rusqlite::params![id,timestamp])
            .map_err(SubscriptionError::from)?;
        return Ok(changed);
    }
    let rev = old.map_or(0, |x| x.1) + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "subscription".into());
        object.insert(
            "source".into(),
            json!({"provider":"subscription","hash":source_hash}),
        );
        object
            .entry("sourceKey")
            .or_insert_with(|| format!("reference:{table}:{id}").into());
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let json = serde_json::to_string(&stored).map_err(SubscriptionError::from)?;
    tx.execute(
        &format!(
            "INSERT INTO {table}
             (id,value,hash,updated_at,updated_by,rev,source,deleted_at,last_seen_at,missing_since)
             VALUES (?1,?2,?3,?4,'subscription',?5,?6,NULL,?4,NULL)
             ON CONFLICT(id) DO UPDATE SET value=excluded.value,hash=excluded.hash,
               updated_at=excluded.updated_at,updated_by=excluded.updated_by,rev=excluded.rev,
               source=excluded.source,deleted_at=NULL,last_seen_at=excluded.last_seen_at,
               missing_since=NULL"
        ),
        rusqlite::params![id, json, hash, timestamp, rev as i64, source_hash],
    )
    .map_err(SubscriptionError::from)?;
    Ok(true)
}

pub fn apply_reference(
    tx: &Transaction<'_>,
    selected_calendar: Option<&AcademicCalendar>,
    selected_departments: Option<&Departments>,
) -> Result<usize, SubscriptionError> {
    let empty_calendar = AcademicCalendar {
        schema_version: 1,
        academic_year: 0,
        source: vec![],
        quarters: BTreeMap::new(),
        days: vec![],
        teaching_ranges: BTreeMap::new(),
        exam_ranges: BTreeMap::new(),
    };
    let empty_departments = Departments {
        schema_version: 1,
        admission_year: 0,
        source: vec![],
        departments: vec![],
    };
    let calendar = selected_calendar.unwrap_or(&empty_calendar);
    let departments = selected_departments.unwrap_or(&empty_departments);
    let calendar_value = serde_json::to_value(calendar).map_err(SubscriptionError::from)?;
    let departments_value = serde_json::to_value(departments).map_err(SubscriptionError::from)?;
    let calendar_hash = content_hash(&calendar_value)?;
    let guide_hash = content_hash(&departments_value)?;
    let mut changed = 0;
    let mut calendar_ids = HashSet::new();
    for (quarter, range) in &calendar.quarters {
        for (kind, date) in [("quarter-start", &range.start), ("quarter-end", &range.end)] {
            let id = format!("calendar:{}:{kind}:{quarter}", calendar.academic_year);
            calendar_ids.insert(id.clone());
            let label = json!({"ja":format!("{quarter} {}",if kind=="quarter-start"{"開始"}else{"終了"}),"en":format!("{quarter} {}",if kind=="quarter-start"{"starts"}else{"ends"})});
            let mut value = json!({"id":id,"academicYear":calendar.academic_year,"date":date,"type":kind,"quarter":quarter,"label":label,"provenance":{"source":calendar.source}});
            if kind == "quarter-start" {
                if let Some(ranges) = calendar.teaching_ranges.get(quarter) {
                    value["teachingRanges"] = json!(ranges);
                }
                if let Some(ranges) = calendar.exam_ranges.get(quarter) {
                    value["examRanges"] = json!(ranges);
                }
            }
            changed += usize::from(apply_row(
                tx,
                "academic_calendar",
                &id,
                &value,
                &calendar_hash,
            )?);
        }
    }
    for day in &calendar.days {
        let id = format!("calendar:{}:day:{}", calendar.academic_year, day.date);
        calendar_ids.insert(id.clone());
        let mut value = serde_json::to_value(day).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "calendar day")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), calendar.academic_year.into());
        object.insert("provenance".into(), json!({"source":calendar.source}));
        changed += usize::from(apply_row(
            tx,
            "academic_calendar",
            &id,
            &value,
            &calendar_hash,
        )?);
    }
    let mut department_ids = HashSet::new();
    let mut recommended_ids = HashSet::new();
    for department in &departments.departments {
        let id = format!(
            "cohort:{}:department:{}",
            departments.admission_year, department.id
        );
        department_ids.insert(id.clone());
        let mut value = serde_json::to_value(department).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "department")?;
        object.insert("id".into(), id.clone().into());
        object.insert("admissionYear".into(), departments.admission_year.into());
        object.insert("cohortYear".into(), departments.admission_year.into());
        object.insert("provenance".into(), json!({"source":departments.source}));
        changed += usize::from(apply_row(tx, "departments", &id, &value, &guide_hash)?);
        for course in &department.recommended {
            let id = format!(
                "cohort:{}:recommended:{}:{}:{}",
                departments.admission_year, department.id, course.code, course.group
            );
            recommended_ids.insert(id.clone());
            let mut value = serde_json::to_value(course).map_err(SubscriptionError::from)?;
            let object = object_mut(&mut value, "recommended course")?;
            object.insert("id".into(), id.clone().into());
            object.insert("admissionYear".into(), departments.admission_year.into());
            object.insert("cohortYear".into(), departments.admission_year.into());
            object.insert("departmentId".into(), department.id.clone().into());
            object.insert(
                "departmentRowId".into(),
                format!(
                    "cohort:{}:department:{}",
                    departments.admission_year, department.id
                )
                .into(),
            );
            object.insert(
                "provenance".into(),
                json!({"source":departments.source,"guidePdf":department.guide_pdf}),
            );
            changed += usize::from(apply_row(
                tx,
                "recommended_courses",
                &id,
                &value,
                &guide_hash,
            )?);
        }
    }
    for (table, prefix, live) in [
        (
            "academic_calendar",
            format!("calendar:{}:%", calendar.academic_year),
            calendar_ids,
        ),
        (
            "departments",
            format!("cohort:{}:department:%", departments.admission_year),
            department_ids,
        ),
        (
            "recommended_courses",
            format!("cohort:{}:recommended:%", departments.admission_year),
            recommended_ids,
        ),
    ] {
        if (table == "academic_calendar" && selected_calendar.is_none())
            || (table != "academic_calendar" && selected_departments.is_none())
        {
            continue;
        }
        let mut statement = tx
            .prepare(&format!(
                "SELECT id FROM {table} WHERE id LIKE ?1 AND missing_since IS NULL"
            ))
            .map_err(SubscriptionError::from)?;
        let old = statement
            .query_map([prefix], |r| r.get::<_, String>(0))
            .map_err(SubscriptionError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(SubscriptionError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed+=tx.execute(&format!("UPDATE {table} SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1"),rusqlite::params![id,missing]).map_err(SubscriptionError::from)?;
        }
    }
    Ok(changed)
}

fn mark_missing(
    tx: &Transaction<'_>,
    table: &str,
    prefix: &str,
    live: &HashSet<String>,
) -> Result<usize, SubscriptionError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT id FROM {table} WHERE id LIKE ?1 AND missing_since IS NULL"
        ))
        .map_err(SubscriptionError::from)?;
    let old = statement
        .query_map([prefix], |row| row.get::<_, String>(0))
        .map_err(SubscriptionError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(SubscriptionError::from)?;
    drop(statement);
    let mut changed = 0;
    for id in old.into_iter().filter(|id| !live.contains(id)) {
        let timestamp = Utc::now().to_rfc3339();
        changed += tx.execute(&format!("UPDATE {table} SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1"), rusqlite::params![id,timestamp]).map_err(SubscriptionError::from)?;
    }
    Ok(changed)
}

pub fn apply_important_dates(
    tx: &Transaction<'_>,
    feed: &ImportantDates,
) -> Result<usize, SubscriptionError> {
    let feed_value = serde_json::to_value(feed).map_err(SubscriptionError::from)?;
    let source_hash = content_hash(&feed_value)?;
    let mut changed = 0;
    let mut live = HashSet::new();
    for entry in &feed.entries {
        let id = format!("important-date:{}:{}", feed.academic_year, entry.id);
        live.insert(id.clone());
        let mut value = serde_json::to_value(entry).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "important date")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), feed.academic_year.into());
        object.insert(
            "sourceKey".into(),
            format!(
                "reference:important-dates:{}:{}",
                feed.academic_year, entry.id
            )
            .into(),
        );
        object.insert("provenance".into(), json!({"source":feed.source}));
        changed += usize::from(apply_row(tx, "important_dates", &id, &value, &source_hash)?);
    }
    changed += mark_missing(
        tx,
        "important_dates",
        &format!("important-date:{}:%", feed.academic_year),
        &live,
    )?;
    Ok(changed)
}

pub fn apply_class_schedules(
    tx: &Transaction<'_>,
    feed: &ClassSchedules,
) -> Result<usize, SubscriptionError> {
    let feed_value = serde_json::to_value(feed).map_err(SubscriptionError::from)?;
    let source_hash = content_hash(&feed_value)?;
    let mut changed = 0;
    let mut live = HashSet::new();
    for entry in &feed.entries {
        let id = format!("class-schedule:{}:{}", feed.academic_year, entry.id);
        live.insert(id.clone());
        let mut value = serde_json::to_value(entry).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "class schedule")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), feed.academic_year.into());
        object.insert(
            "sourceKey".into(),
            format!(
                "reference:class-schedules:{}:{}",
                feed.academic_year, entry.id
            )
            .into(),
        );
        object.insert("provenance".into(), json!({"source":feed.source}));
        changed += usize::from(apply_row(tx, "class_schedules", &id, &value, &source_hash)?);
    }
    changed += mark_missing(
        tx,
        "class_schedules",
        &format!("class-schedule:{}:%", feed.academic_year),
        &live,
    )?;
    Ok(changed)
}

pub fn apply_exam_feed(
    tx: &Transaction<'_>,
    published: &PublishedExamFeed,
) -> Result<usize, SubscriptionError> {
    let feed_value = serde_json::to_value(&published.feed).map_err(SubscriptionError::from)?;
    let source_hash = content_hash(&feed_value)?;
    let mut changed = 0;
    let mut live = HashSet::new();
    for entry in &published.feed.entries {
        let id = format!(
            "{}:{}:{}:{}:{}",
            published.kind.row_prefix(),
            published.feed.academic_year,
            published.level,
            published.quarter,
            entry.id
        );
        live.insert(id.clone());
        let mut value = serde_json::to_value(entry).map_err(SubscriptionError::from)?;
        let object = object_mut(&mut value, "exam")?;
        object.insert("id".into(), id.clone().into());
        object.insert("academicYear".into(), published.feed.academic_year.into());
        object.insert("level".into(), published.level.clone().into());
        if let Some(offering_id) = entry.syllabus_course_id.as_deref() {
            let course_id = canonical_course_id(published.feed.academic_year, offering_id)?;
            object.insert("courseId".into(), course_id.into());
        }
        object.insert(
            "sourceKey".into(),
            format!(
                "{}:{}:{}:{}:{}",
                published.kind.source_key_prefix(),
                published.feed.academic_year,
                published.level,
                published.quarter,
                entry.id
            )
            .into(),
        );
        object.insert(
            "provenance".into(),
            json!({"source":published.feed.source,"dataUrl":published.url}),
        );
        changed += usize::from(apply_row(tx, "exams", &id, &value, &source_hash)?);
    }
    let prefix = format!(
        "{}:{}:{}:{}:%",
        published.kind.row_prefix(),
        published.feed.academic_year,
        published.level,
        published.quarter
    );
    changed += mark_missing(tx, "exams", &prefix, &live)?;
    Ok(changed)
}

#[cfg(test)]
#[path = "tests/subscriptions.rs"]
mod tests;
