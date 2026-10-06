use crate::data::versioned::content_hash;
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::{Client, Url};
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

pub const ACADEMIC_CALENDAR_SCHEMA: &str =
    include_str!("../subscriptions/schema/academic-calendar.schema.json");
pub const DEPARTMENTS_SCHEMA: &str =
    include_str!("../subscriptions/schema/departments.schema.json");
pub const BUNDLED_INDEX: &str = include_str!("../subscriptions/index.json");
pub const BUNDLED_ACADEMIC_CALENDAR_2026: &str =
    include_str!("../subscriptions/data/2026/academic-calendar.json");
pub const BUNDLED_DEPARTMENTS_2025: &str =
    include_str!("../subscriptions/data/2025/departments.json");
pub const BUNDLED_DEPARTMENTS_2026: &str =
    include_str!("../subscriptions/data/2026/departments.json");

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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcademicYearEntry {
    academic_calendar: String,
    #[serde(default)]
    exams: Vec<String>,
    #[serde(default)]
    important_dates: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionYearEntry {
    departments: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubscriptionIndex {
    #[serde(default)]
    academic_years: BTreeMap<String, AcademicYearEntry>,
    #[serde(default)]
    admission_years: BTreeMap<String, AdmissionYearEntry>,
}

fn https_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| format!("invalid HTTPS URL: {raw}"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return Err(format!("invalid HTTPS URL: {raw}"));
    }
    Ok(url)
}

fn date(raw: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| format!("invalid date: {raw}"))
}

fn validate_sources(sources: &[SourceLink]) -> Result<(), String> {
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

pub fn validate_academic_calendar(raw: &str) -> Result<AcademicCalendar, String> {
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
            return Err(format!("invalid {quarter} range"));
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
            return Err(format!("duplicate calendar day: {}", day.date));
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

pub fn validate_departments(raw: &str) -> Result<Departments, String> {
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

pub struct SubscriptionClient {
    client: Client,
}

impl SubscriptionClient {
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { client })
    }
    async fn get(&self, url: Url, limit: usize) -> Result<String, String> {
        let mut response = self
            .client
            .get(url.clone())
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("subscription GET failed: {}", response.status()));
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
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err("subscription response exceeds the size limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| "subscription response is not UTF-8".into())
    }
    pub async fn fetch_selection(
        &self,
        index_url: &str,
        academic_year: i32,
        admission_year: i32,
    ) -> Result<(AcademicCalendar, Departments), String> {
        let base = https_url(index_url)?;
        let raw = self.get(base.clone(), MAX_INDEX_BYTES).await?;
        let index: SubscriptionIndex =
            serde_json::from_str(&raw).map_err(|e| format!("invalid subscription index: {e}"))?;
        let calendar_entry = index
            .academic_years
            .get(&academic_year.to_string())
            .ok_or_else(|| {
                format!("subscription has no calendar for academic year {academic_year}")
            })?;
        let guide_entry = index
            .admission_years
            .get(&admission_year.to_string())
            .ok_or_else(|| {
                format!("subscription has no guide for admission year {admission_year}")
            })?;
        let calendar_url = https_url(
            base.join(&calendar_entry.academic_calendar)
                .map_err(|_| "invalid academic calendar path")?
                .as_str(),
        )?;
        let departments_url = https_url(
            base.join(&guide_entry.departments)
                .map_err(|_| "invalid departments path")?
                .as_str(),
        )?;
        let calendar_raw = self.get(calendar_url, MAX_DATA_BYTES).await?;
        let departments_raw = self.get(departments_url, MAX_DATA_BYTES).await?;
        let calendar = validate_academic_calendar(&calendar_raw)?;
        let departments = validate_departments(&departments_raw)?;
        if calendar.academic_year != academic_year || departments.admission_year != admission_year {
            return Err("subscription file selection does not match index".into());
        }
        Ok((calendar, departments))
    }

    pub async fn fetch_calendar(
        &self,
        index_url: &str,
        year: i32,
    ) -> Result<(AcademicCalendar, String), String> {
        let base = https_url(index_url)?;
        let index: SubscriptionIndex =
            serde_json::from_str(&self.get(base.clone(), MAX_INDEX_BYTES).await?)
                .map_err(|e| e.to_string())?;
        let entry = index
            .academic_years
            .get(&year.to_string())
            .ok_or("calendar year is not published in this index")?;
        let url = https_url(
            base.join(&entry.academic_calendar)
                .map_err(|e| e.to_string())?
                .as_str(),
        )?;
        let calendar = validate_academic_calendar(&self.get(url.clone(), MAX_DATA_BYTES).await?)?;
        if calendar.academic_year != year {
            return Err("calendar year does not match the selection".into());
        }
        Ok((calendar, url.to_string()))
    }
    pub async fn fetch_departments(
        &self,
        index_url: &str,
        year: i32,
    ) -> Result<(Departments, String), String> {
        let base = https_url(index_url)?;
        let index: SubscriptionIndex =
            serde_json::from_str(&self.get(base.clone(), MAX_INDEX_BYTES).await?)
                .map_err(|e| e.to_string())?;
        let entry = index
            .admission_years
            .get(&year.to_string())
            .ok_or("admission year is not published in this index")?;
        let url = https_url(
            base.join(&entry.departments)
                .map_err(|e| e.to_string())?
                .as_str(),
        )?;
        let departments = validate_departments(&self.get(url.clone(), MAX_DATA_BYTES).await?)?;
        if departments.admission_year != year {
            return Err("guide year does not match the selection".into());
        }
        Ok((departments, url.to_string()))
    }
    /// Convenience for installations where the current academic year and the
    /// student's admission cohort happen to be the same. Callers with a saved
    /// profile must use `fetch_selection` instead.
    pub async fn fetch_year(
        &self,
        index_url: &str,
        year: i32,
    ) -> Result<(AcademicCalendar, Departments), String> {
        self.fetch_selection(index_url, year, year).await
    }
}

fn apply_row(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    source_hash: &str,
    extra: &[(&str, rusqlite::types::Value)],
) -> Result<bool, String> {
    let hash = content_hash(value)?;
    let old: Option<(String, u64, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash,rev,missing_since FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let timestamp = Utc::now().to_rfc3339();
    if old.as_ref().is_some_and(|(h, _, _)| h == &hash) {
        let changed = old.as_ref().is_some_and(|(_, _, missing)| missing.is_some());
        tx.execute(&format!("UPDATE {table} SET last_seen_at=?2,missing_since=NULL,deleted_at=NULL,
            value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"), rusqlite::params![id,timestamp])
            .map_err(|e|e.to_string())?;
        return Ok(changed);
    }
    let mut names = vec![
        "id",
        "value",
        "hash",
        "updated_at",
        "updated_by",
        "rev",
        "source",
        "deleted_at",
        "last_seen_at",
        "missing_since",
    ];
    names.extend(extra.iter().map(|(n, _)| *n));
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
        object.insert("sourceKey".into(), format!("reference:{table}:{source_hash}:{id}").into());
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let mut values = vec![
        id.to_string().into(),
        serde_json::to_string(&stored)
            .map_err(|e| e.to_string())?
            .into(),
        hash.into(),
        timestamp.clone().into(),
        String::from("subscription").into(),
        (rev as i64).into(),
        source_hash.to_string().into(),
        rusqlite::types::Value::Null,
        timestamp.clone().into(),
        rusqlite::types::Value::Null,
    ];
    values.extend(extra.iter().map(|(_, v)| v.clone()));
    let marks = (1..=names.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let updates = names
        .iter()
        .skip(1)
        .map(|n| format!("{n}=excluded.{n}"))
        .collect::<Vec<_>>()
        .join(",");
    tx.execute(
        &format!(
            "INSERT INTO {table} ({}) VALUES ({marks}) ON CONFLICT(id) DO UPDATE SET {updates}",
            names.join(",")
        ),
        rusqlite::params_from_iter(values),
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

pub fn apply_subscription(
    connection: &mut Connection,
    calendar: &AcademicCalendar,
    departments: &Departments,
) -> Result<usize, String> {
    apply_reference(connection, Some(calendar), Some(departments))
}

pub fn apply_reference(
    connection: &mut Connection,
    selected_calendar: Option<&AcademicCalendar>,
    selected_departments: Option<&Departments>,
) -> Result<usize, String> {
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
    let calendar_value = serde_json::to_value(calendar).map_err(|e| e.to_string())?;
    let departments_value = serde_json::to_value(departments).map_err(|e| e.to_string())?;
    let calendar_hash = content_hash(&calendar_value)?;
    let guide_hash = content_hash(&departments_value)?;
    let tx = connection.transaction().map_err(|e| e.to_string())?;
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
                &tx,
                "academic_calendar",
                value["id"].as_str().unwrap(),
                &value,
                &calendar_hash,
                &[
                    ("date", date.clone().into()),
                    ("type", kind.to_string().into()),
                    ("label", serde_json::to_string(&label).unwrap().into()),
                    ("source_hash", calendar_hash.clone().into()),
                ],
            )?);
        }
    }
    for day in &calendar.days {
        let id = format!("calendar:{}:day:{}", calendar.academic_year, day.date);
        calendar_ids.insert(id.clone());
        let mut value = serde_json::to_value(day).map_err(|e| e.to_string())?;
        value
            .as_object_mut()
            .expect("calendar day object")
            .insert("id".into(), id.clone().into());
        value
            .as_object_mut()
            .expect("calendar day object")
            .insert("academicYear".into(), calendar.academic_year.into());
        value
            .as_object_mut()
            .expect("calendar day object")
            .insert("provenance".into(), json!({"source":calendar.source}));
        changed += usize::from(apply_row(
            &tx,
            "academic_calendar",
            &id,
            &value,
            &calendar_hash,
            &[
                ("date", day.date.clone().into()),
                (
                    "type",
                    match day.day_type {
                        CalendarDayType::Holiday => "holiday",
                        CalendarDayType::NoClass => "no-class",
                        CalendarDayType::Replacement => "replacement",
                    }
                    .to_string()
                    .into(),
                ),
                (
                    "replacement_day",
                    day.replacement_day
                        .map(|v| (v as i64).into())
                        .unwrap_or(rusqlite::types::Value::Null),
                ),
                ("label", serde_json::to_string(&day.label).unwrap().into()),
                ("source_hash", calendar_hash.clone().into()),
            ],
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
        let mut value = serde_json::to_value(department).map_err(|e| e.to_string())?;
        let object = value.as_object_mut().expect("department object");
        object.insert("id".into(), id.clone().into());
        object.insert("admissionYear".into(), departments.admission_year.into());
        object.insert("cohortYear".into(), departments.admission_year.into());
        object.insert("provenance".into(), json!({"source":departments.source}));
        changed += usize::from(apply_row(
            &tx,
            "departments",
            &id,
            &value,
            &guide_hash,
            &[],
        )?);
        for course in &department.recommended {
            let id = format!(
                "cohort:{}:recommended:{}:{}:{}",
                departments.admission_year, department.id, course.code, course.group
            );
            recommended_ids.insert(id.clone());
            let mut value = serde_json::to_value(course).map_err(|e| e.to_string())?;
            let object = value.as_object_mut().expect("recommended course object");
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
                &tx,
                "recommended_courses",
                &id,
                &value,
                &guide_hash,
                &[
                    (
                        "department_id",
                        format!(
                            "cohort:{}:department:{}",
                            departments.admission_year, department.id
                        )
                        .into(),
                    ),
                    ("code", course.code.clone().into()),
                    ("course_group", course.group.clone().into()),
                ],
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
            .map_err(|e| e.to_string())?;
        let old = statement
            .query_map([prefix], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed+=tx.execute(&format!("UPDATE {table} SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1"),rusqlite::params![id,missing]).map_err(|e|e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

pub fn bundled_2026() -> Result<(AcademicCalendar, Departments), String> {
    Ok((
        validate_academic_calendar(BUNDLED_ACADEMIC_CALENDAR_2026)?,
        validate_departments(BUNDLED_DEPARTMENTS_2026)?,
    ))
}

/// Selects the two independent bundled datasets. A calendar year must never
/// be used as a fallback for a student's admission cohort.
pub fn bundled_selection(
    academic_year: i32,
    admission_year: i32,
) -> Result<(AcademicCalendar, Departments), String> {
    if academic_year != 2026 {
        return Err(format!("no bundled academic calendar for {academic_year}"));
    }
    let calendar = validate_academic_calendar(BUNDLED_ACADEMIC_CALENDAR_2026)?;
    let departments = match admission_year {
        2025 => validate_departments(BUNDLED_DEPARTMENTS_2025)?,
        2026 => validate_departments(BUNDLED_DEPARTMENTS_2026)?,
        _ => {
            return Err(format!(
                "no bundled guide for admission year {admission_year}"
            ))
        }
    };
    Ok((calendar, departments))
}
