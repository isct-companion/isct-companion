use super::{
    client::{https_url, MAX_DATA_BYTES},
    SubscriptionError,
};
use crate::locale::Localized;
use chrono::{Datelike, NaiveDate, NaiveTime};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// Isct-Data names every school text in Japanese.
fn blank(ja: Option<&str>) -> bool {
    ja.is_none_or(|ja| ja.trim().is_empty())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourceLink {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DateRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum CalendarDayType {
    Holiday,
    NoClass,
    Replacement,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CalendarDay {
    pub date: String,
    #[serde(rename = "type")]
    pub(crate) day_type: CalendarDayType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) replacement_day: Option<u8>,
    pub label: Localized,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AcademicCalendar {
    pub(crate) schema_version: u32,
    pub(crate) academic_year: i32,
    pub source: Vec<SourceLink>,
    pub(crate) quarters: BTreeMap<String, DateRange>,
    pub days: Vec<CalendarDay>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) teaching_ranges: BTreeMap<String, Vec<DateRange>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) exam_ranges: BTreeMap<String, Vec<DateRange>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Credits {
    pub(crate) lecture: f64,
    pub(crate) exercise: f64,
    pub(crate) lab: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RecommendationMark {
    Required,
    Recommended,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RecommendedCourse {
    pub(crate) category: Localized,
    pub level: u16,
    pub code: String,
    #[serde(default)]
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<RecommendationMark>,
    pub title: Localized,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) credits: Option<Credits>,
    #[serde(default)]
    pub(crate) competencies: Vec<u8>,
    #[serde(default)]
    pub content: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) source_pages: Vec<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Department {
    pub id: String,
    pub name: Localized,
    pub(crate) syllabus_scope: String,
    pub(crate) unit_code: String,
    pub(crate) guide_pdf: String,
    pub(crate) recommended: Vec<RecommendedCourse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Departments {
    pub(crate) schema_version: u32,
    pub(crate) admission_year: i32,
    pub source: Vec<SourceLink>,
    pub(crate) departments: Vec<Department>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FeedSource {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) landing_page_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) published_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) retrieved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportantDateEntry {
    pub id: String,
    pub(crate) start_date: String,
    pub(crate) end_date: String,
    pub label: Localized,
    pub(crate) category: String,
    pub(crate) source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label_english_source: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportantDates {
    pub(crate) schema_version: u32,
    pub(crate) academic_year: i32,
    pub(crate) timezone: String,
    pub source: Vec<FeedSource>,
    pub entries: Vec<ImportantDateEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExamEntry {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub quarter: String,
    pub(crate) course_code: String,
    pub date: String,
    pub label: Localized,
    pub(crate) source_ids: Vec<String>,
    pub room: Option<Localized>,
    #[serde(default)]
    pub periods: Option<Vec<u8>>,
    #[serde(default)]
    pub(crate) start_time: Option<String>,
    #[serde(default)]
    pub(crate) end_time: Option<String>,
    #[serde(default)]
    pub(crate) syllabus_course_id: Option<String>,
    #[serde(default)]
    pub(crate) class_label: Option<Localized>,
    #[serde(default)]
    pub notes: Option<Localized>,
    #[serde(default)]
    pub(crate) source_pages: Vec<u32>,
    #[serde(default)]
    pub(crate) rooms: Vec<Localized>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExamFeed {
    pub(crate) schema_version: u32,
    pub(crate) academic_year: i32,
    pub(crate) timezone: String,
    pub source: Vec<FeedSource>,
    pub entries: Vec<ExamEntry>,
}

/// The two datasets cut from the central exam timetable: its final-exam rows,
/// and its class rows (授業, including make-up classes) in the exam period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExamFeedKind {
    Exams,
    ExamWeekClasses,
}

impl ExamFeedKind {
    pub(super) fn directory(self) -> &'static str {
        match self {
            Self::Exams => "exam",
            Self::ExamWeekClasses => "exam-week-classes",
        }
    }
    /// The `kind` every entry of this dataset carries.
    pub(crate) fn entry_kind(self) -> &'static str {
        match self {
            Self::Exams => "exam",
            Self::ExamWeekClasses => "class",
        }
    }
    pub(super) fn row_prefix(self) -> &'static str {
        match self {
            Self::Exams => "exam",
            Self::ExamWeekClasses => "exam-week-class",
        }
    }
    pub(super) fn source_key_prefix(self) -> &'static str {
        match self {
            Self::Exams => "reference:exams",
            Self::ExamWeekClasses => "reference:exam-week-classes",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PublishedExamFeed {
    pub kind: ExamFeedKind,
    pub level: String,
    pub quarter: String,
    pub url: String,
    pub(crate) feed: ExamFeed,
}

/// One dated session of a course that does not meet every week.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClassSession {
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
pub(crate) struct ClassScheduleEntry {
    pub id: String,
    pub(crate) course_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) class_label: Option<Localized>,
    pub(crate) offering_ids: Vec<String>,
    pub(crate) quarters: Vec<String>,
    pub(crate) source_ids: Vec<String>,
    pub sessions: Vec<ClassSession>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClassSchedules {
    pub(crate) schema_version: u32,
    pub(crate) academic_year: i32,
    pub(crate) timezone: String,
    pub source: Vec<FeedSource>,
    pub entries: Vec<ClassScheduleEntry>,
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

pub(crate) fn validate_academic_calendar(raw: &str) -> Result<AcademicCalendar, SubscriptionError> {
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
        if blank(day.label.ja.as_deref()) {
            return Err("calendar day Japanese label is empty".into());
        }
    }
    Ok(value)
}

pub(crate) fn validate_departments(raw: &str) -> Result<Departments, SubscriptionError> {
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
        if blank(department.name.ja.as_deref())
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
            if blank(course.title.ja.as_deref()) || blank(course.category.ja.as_deref()) {
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

pub(super) fn object_mut<'a>(
    value: &'a mut Value,
    context: &str,
) -> Result<&'a mut serde_json::Map<String, Value>, SubscriptionError> {
    value.as_object_mut().ok_or_else(|| {
        SubscriptionError::Storage(format!("{context} did not serialize as an object"))
    })
}

pub(crate) fn validate_important_dates(
    raw: &str,
    year: i32,
) -> Result<ImportantDates, SubscriptionError> {
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
            || blank(entry.label.ja.as_deref())
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

pub(crate) fn validate_class_schedules(
    raw: &str,
    year: i32,
) -> Result<ClassSchedules, SubscriptionError> {
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
            || entry
                .class_label
                .as_ref()
                .is_some_and(|label| blank(label.ja.as_deref()))
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

pub(crate) fn validate_exam_feed(
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
            || blank(entry.label.ja.as_deref())
            || [&entry.room, &entry.class_label, &entry.notes]
                .into_iter()
                .flatten()
                .chain(&entry.rooms)
                .any(|text| blank(text.ja.as_deref()))
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
