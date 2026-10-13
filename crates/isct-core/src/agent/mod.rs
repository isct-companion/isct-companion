//! Agent read models and mutations shared by application adapters.

use crate::data::{AgentWriteError, DataError, DataStore};
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentPolicy {
    pub allow_writes: bool,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("IsctCompanion agent access is disabled in Settings.")]
    Disabled,
    #[error("Agent writes are disabled.")]
    WritesDisabled,
    #[error("unknown tool")]
    UnknownTool,
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Compatibility(String),
    #[error("{0}")]
    Storage(String),
}

impl From<&str> for AgentError {
    fn from(message: &str) -> Self {
        Self::InvalidInput(message.to_string())
    }
}
impl From<String> for AgentError {
    fn from(message: String) -> Self {
        Self::InvalidInput(message)
    }
}
impl From<DataError> for AgentError {
    fn from(error: DataError) -> Self {
        match error {
            DataError::InvalidInput(message) => Self::InvalidInput(message),
            DataError::Conflict(message) => Self::Conflict(message),
            DataError::Compatibility(message) => Self::Compatibility(message),
            DataError::Storage(message) => Self::Storage(message),
        }
    }
}

#[derive(Clone)]
pub struct AgentService {
    store: DataStore,
}

impl AgentService {
    pub fn new(store: DataStore) -> Self {
        Self { store }
    }

    pub fn call(&self, name: &str, args: &Value, policy: AgentPolicy) -> Result<Value, AgentError> {
        if !self.store.agent_access_enabled()? {
            return Err(AgentError::Disabled);
        }
        if is_write_tool(name) && !policy.allow_writes {
            return Err(AgentError::WritesDisabled);
        }
        let mut value = call_inner(&self.store, name, args)?;
        slim(&mut value);
        Ok(value)
    }
}

pub fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "plan_add"
            | "plan_remove"
            | "plan_event_create"
            | "plan_event_update"
            | "plan_event_delete"
            | "hide_assignment"
            | "restore_assignment"
    )
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, AgentError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AgentError::InvalidInput(format!("{key} is required")))
}

fn visible_rows(workspace: &Value, key: &str) -> Vec<Value> {
    workspace
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| row.get("deletedAt").is_none() && row.get("missingSince").is_none())
        .cloned()
        .collect()
}

fn date_range(args: &Value) -> Result<(chrono::NaiveDate, chrono::NaiveDate), AgentError> {
    let from = chrono::NaiveDate::parse_from_str(required(args, "from")?, "%Y-%m-%d")
        .map_err(|_| "from must be an ISO date".to_string())?;
    let to = chrono::NaiveDate::parse_from_str(required(args, "to")?, "%Y-%m-%d")
        .map_err(|_| "to must be an ISO date".to_string())?;
    let days = (to - from).num_days();
    if !(0..=365).contains(&days) {
        return Err("date range must be ordered and no longer than 366 days".into());
    }
    Ok((from, to))
}

/// `YYYY-nQ` → (year, quarter).
fn parse_term(term: &str) -> Result<(i64, &str), AgentError> {
    let invalid = || format!("term must look like 2026-3Q, got {term}");
    let (year, quarter) = term.split_once('-').ok_or_else(invalid)?;
    let year = year
        .parse::<i64>()
        .ok()
        .filter(|year| (1000..=9999).contains(year))
        .ok_or_else(invalid)?;
    if !matches!(quarter, "1Q" | "2Q" | "3Q" | "4Q") {
        return Err(invalid().into());
    }
    Ok((year, quarter))
}

fn now_jst() -> Result<chrono::DateTime<chrono::FixedOffset>, AgentError> {
    let offset = chrono::FixedOffset::east_opt(9 * 3600)
        .ok_or_else(|| AgentError::Compatibility("Japan time offset is unavailable".into()))?;
    Ok(chrono::Utc::now().with_timezone(&offset))
}

fn text<'a>(row: &'a Value, key: &str) -> Option<&'a str> {
    row.get(key).and_then(Value::as_str)
}

/// Courses on the timetable of `term`: registered at school or planned.
/// Mirrors `scheduledCourses` in src/core/schedule.ts.
fn scheduled_courses(workspace: &Value, term: &str) -> Vec<Value> {
    let ids = ["memberships", "planItems"]
        .into_iter()
        .flat_map(|key| visible_rows(workspace, key))
        .filter(|row| text(row, "termId") == Some(term))
        .filter_map(|row| text(&row, "courseId").map(str::to_string))
        .collect::<std::collections::HashSet<_>>();
    visible_rows(workspace, "courses")
        .into_iter()
        .filter(|row| {
            text(row, "termId") == Some(term) && text(row, "id").is_some_and(|id| ids.contains(id))
        })
        .collect()
}

/// The school-calendar row for a date: a personal exception first, then a
/// holiday, no-class, break or replacement day. Mirrors `calendarState`.
fn calendar_state(workspace: &Value, date: &str) -> Option<Value> {
    visible_rows(workspace, "exceptions")
        .into_iter()
        .find(|row| text(row, "date") == Some(date))
        .or_else(|| {
            visible_rows(workspace, "academicCalendar")
                .into_iter()
                .find(|row| {
                    text(row, "date") == Some(date)
                        && matches!(
                            text(row, "type"),
                            Some("holiday" | "no-class" | "break" | "replacement")
                        )
                })
        })
}

/// Classes actually held on `date`, after holidays, replacement days and the
/// quarter's teaching ranges. Mirrors `occurrencesOnDate` (courses only); the
/// TypeScript version runs in the webview, which this binary cannot reach.
fn classes_on(workspace: &Value, term: &str, date: chrono::NaiveDate) -> Vec<Value> {
    use chrono::Datelike;
    let iso = date.to_string();
    let state = calendar_state(workspace, &iso);
    let kind = state.as_ref().and_then(|row| text(row, "type"));
    let civil = u64::from(date.weekday().number_from_monday());
    let teaching_day = match kind {
        Some("replacement") => state
            .as_ref()
            .and_then(|row| row.get("replacementDay"))
            .and_then(Value::as_u64)
            .unwrap_or(civil),
        Some("holiday" | "no-class" | "break") => 0,
        _ => civil,
    };
    let outside_teaching = parse_term(term).ok().is_some_and(|(year, quarter)| {
        let start = format!("calendar:{year}:quarter-start:{quarter}");
        kind != Some("replacement")
            && visible_rows(workspace, "academicCalendar")
                .into_iter()
                .find(|row| text(row, "id") == Some(start.as_str()))
                .and_then(|row| row.get("teachingRanges").and_then(Value::as_array).cloned())
                .is_some_and(|ranges| {
                    !ranges.iter().any(|range| {
                        text(range, "start").is_some_and(|start| start <= iso.as_str())
                            && text(range, "end").is_some_and(|end| iso.as_str() <= end)
                    })
                })
    });
    let intensive = visible_rows(workspace, "intensiveSchedules");
    let mut classes = Vec::new();
    for course in scheduled_courses(workspace, term) {
        let title = course
            .pointer("/overrides/title")
            .and_then(Value::as_str)
            .filter(|title| !title.is_empty())
            .or_else(|| text(&course, "title"))
            .unwrap_or_default();
        let class = |start: &str, end: &str, room: Option<&Value>| {
            json!({
                "courseId": course.get("id"), "code": course.get("code"), "title": title,
                "startTime": start, "endTime": end, "room": room
            })
        };
        if teaching_day != 0 && !outside_teaching {
            for meeting in course
                .get("meetings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let held = meeting
                    .get("days")
                    .and_then(Value::as_array)
                    .is_some_and(|days| days.iter().any(|day| day.as_u64() == Some(teaching_day)))
                    && text(meeting, "validFrom").is_none_or(|from| from <= iso.as_str())
                    && text(meeting, "validTo").is_none_or(|to| iso.as_str() <= to);
                if let (true, Some(start), Some(end)) =
                    (held, text(meeting, "startTime"), text(meeting, "endTime"))
                {
                    classes.push(class(start, end, meeting.get("room")));
                }
            }
        }
        for row in intensive.iter().filter(|row| {
            row.get("courseId") == course.get("id")
                && text(row, "from").is_some_and(|from| from <= iso.as_str())
                && text(row, "to").is_some_and(|to| iso.as_str() <= to)
        }) {
            let periods = row
                .get("days")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|day| text(day, "date") == Some(iso.as_str()))
                .and_then(|day| day.get("periods").and_then(Value::as_array).cloned());
            let clocks = periods.as_ref().and_then(|periods| {
                Some((
                    clock(periods.first()?.as_u64()?, false)?,
                    clock(periods.get(1)?.as_u64()?, true)?,
                ))
            });
            if let Some((start, end)) = clocks {
                classes.push(class(start, end, row.get("room")));
            }
        }
    }
    classes.sort_by(|a, b| text(a, "startTime").cmp(&text(b, "startTime")));
    classes
}

/// Personal plans with at least one occurrence in the inclusive range.
fn plans_between(workspace: &Value, from: chrono::NaiveDate, to: chrono::NaiveDate) -> Vec<Value> {
    visible_rows(workspace, "plans")
        .into_iter()
        .filter_map(|mut row| {
            let start = text(&row, "start")?.to_string();
            if let Some(rule) = text(&row, "rrule") {
                let exdates = row
                    .get("exdates")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                let occurrences =
                    crate::plans::occurrences_between(&start, rule, &exdates, from, to).ok()?;
                if occurrences.is_empty() {
                    return None;
                }
                row["occurrences"] = json!(occurrences);
                Some(row)
            } else {
                chrono::DateTime::parse_from_rfc3339(&start)
                    .ok()
                    .filter(|value| value.date_naive() >= from && value.date_naive() <= to)
                    .map(|_| row)
            }
        })
        .collect()
}

/// Sync bookkeeping an agent never needs; dropping it keeps answers small.
fn slim(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for key in [
                "rev",
                "updatedAt",
                "updatedBy",
                "lastSeenAt",
                "sourceKey",
                "provenance",
                "hash",
                "fetchedAt",
                "missingSince",
            ] {
                object.remove(key);
            }
            object.values_mut().for_each(slim);
        }
        Value::Array(rows) => rows.iter_mut().for_each(slim),
        _ => {}
    }
}

fn fallback_term(date: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let year = if date.month() < 4 {
        date.year() - 1
    } else {
        date.year()
    };
    let quarter = match date.month() {
        4 | 5 => "1Q",
        6..=9 => "2Q",
        10 | 11 => "3Q",
        _ => "4Q",
    };
    format!("{year}-{quarter}")
}

fn localized(value: Option<&Value>) -> String {
    value
        .and_then(|value| value.get("en").or_else(|| value.get("ja")))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn clock(period: u64, end: bool) -> Option<&'static str> {
    let pair = match period {
        1 | 2 => ("08:50", "10:30"),
        3 | 4 => ("10:45", "12:25"),
        5 | 6 => ("13:30", "15:10"),
        7 | 8 => ("15:25", "17:05"),
        9 | 10 => ("17:15", "18:55"),
        _ => return None,
    };
    Some(if end { pair.1 } else { pair.0 })
}

fn course_from_offering(offering: &Value, term: &str) -> Result<Value, AgentError> {
    let offering_id = offering
        .get("id")
        .and_then(Value::as_str)
        .ok_or("offering is invalid")?;
    let title = localized(offering.get("title"));
    let instructors = offering
        .get("instructors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| localized(Some(value)))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    let meetings = offering
        .get("schedule")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            if entry.get("kind").and_then(Value::as_str) != Some("weekly") {
                return None;
            }
            let periods = entry.get("periods")?.as_array()?;
            let from = periods.first()?.as_u64()?;
            let to = periods.get(1)?.as_u64()?;
            Some(json!({
                "days":[entry.get("day")?.as_u64()?],
                "periods":(from..=to).collect::<Vec<_>>(),
                "startTime":clock(from, false)?, "endTime":clock(to, true)?,
                "room":localized(entry.get("room")), "roomNames":entry.get("room")
            }))
        })
        .collect::<Vec<_>>();
    let credits = offering
        .get("credits")
        .cloned()
        .unwrap_or_else(|| json!({"lecture":0,"exercise":0,"lab":0}));
    let total = ["lecture", "exercise", "lab"]
        .into_iter()
        .filter_map(|key| credits.get(key).and_then(Value::as_f64))
        .sum::<f64>();
    Ok(json!({
        "kind":"course", "termId":term, "offeringId":offering_id,
        "names":{"title":offering.get("title"),"instructor":{"en":instructors}},
        "code":offering.get("code").and_then(Value::as_str).unwrap_or_default(),
        "title":title, "instructor":instructors, "credits":total, "creditSplit":credits,
        "instructionLanguage":offering.get("instructionLanguage").and_then(Value::as_str).unwrap_or("other"),
        "color":"#556b8d", "meetings":meetings,
        "url":offering.get("syllabusUrl"),
        "source":{"provider":"syllabus","externalId":offering_id}
    }))
}

fn write_result(details: Value) -> Value {
    let mut result = json!({"ok": true});
    if let (Some(result), Value::Object(details)) = (result.as_object_mut(), details) {
        result.extend(details);
    }
    result
}

fn agent_write_result<T>(result: Result<T, AgentWriteError>) -> Result<T, AgentError> {
    match result {
        Ok(value) => Ok(value),
        Err(AgentWriteError::Disabled) => Err(AgentError::Disabled),
        Err(AgentWriteError::Data(error)) => Err(error.into()),
    }
}

/// A personal plan that has not been deleted.
fn live_plan(store: &DataStore, id: &str) -> Result<Value, AgentError> {
    store
        .user_rows("plans")?
        .into_iter()
        .find(|row| text(row, "id") == Some(id) && row.get("deletedAt").is_none())
        .ok_or_else(|| AgentError::InvalidInput("plan was not found".into()))
}

fn valid_plan_time(value: &str) -> Result<(), AgentError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| AgentError::InvalidInput("start must be an RFC 3339 timestamp".into()))
}

fn duration(args: &Value, required: bool) -> Result<Option<u64>, AgentError> {
    let value = args.get("duration").and_then(Value::as_u64);
    if required && value.is_none() {
        return Err("duration is required".into());
    }
    if value.is_some_and(|minutes| !(1..=10_080).contains(&minutes)) {
        return Err("duration must be between 1 and 10080 minutes".into());
    }
    Ok(value)
}

fn call_inner(store: &DataStore, name: &str, args: &Value) -> Result<Value, AgentError> {
    let workspace = store.load_workspace()?;
    match name {
        "today" => {
            let now = now_jst()?;
            let date = now.date_naive();
            let iso = date.to_string();
            let term = text(&workspace, "currentTerm")
                .map(str::to_string)
                .unwrap_or_else(|| fallback_term(date));
            let due = |row: &Value| {
                text(row, "dueAt").and_then(|due| chrono::DateTime::parse_from_rfc3339(due).ok())
            };
            let mut open = visible_rows(&workspace, "assignments")
                .into_iter()
                .filter(|row| {
                    row.get("hidden") != Some(&Value::Bool(true))
                        && text(row, "submission") != Some("submitted")
                })
                .collect::<Vec<_>>();
            open.sort_by_key(|row| due(row));
            let (upcoming, overdue): (Vec<_>, Vec<_>) = open
                .into_iter()
                .partition(|row| due(row).is_none_or(|due| due >= now));
            let recent = now - chrono::Duration::days(14);
            Ok(json!({
                "now": now.to_rfc3339(), "date": iso, "term": term,
                "calendar": calendar_state(&workspace, &iso),
                "classes": classes_on(&workspace, &term, date),
                "plans": plans_between(&workspace, date, date),
                "assignments": upcoming,
                "overdue": overdue.into_iter()
                    .filter(|row| due(row).is_some_and(|due| due >= recent)).collect::<Vec<_>>(),
                "academicSummary": visible_rows(&workspace, "gpaSummary")
            }))
        }
        "schedule" => {
            let (from, to) = date_range(args)?;
            let mut terms = std::collections::BTreeSet::new();
            let mut cursor = from;
            while cursor <= to {
                terms.insert(fallback_term(cursor));
                cursor += chrono::Duration::days(1);
            }
            Ok(json!({
                "from": from.to_string(), "to": to.to_string(),
                "courses": terms.iter()
                    .flat_map(|term| scheduled_courses(&workspace, term)).collect::<Vec<_>>(),
                "intensiveSchedules": visible_rows(&workspace, "intensiveSchedules").into_iter()
                    .filter(|row| {
                        let start = row.get("from").and_then(Value::as_str).and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
                        let end = row.get("to").and_then(Value::as_str).and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
                        matches!((start,end),(Some(start),Some(end)) if start <= to && end >= from)
                    }).collect::<Vec<_>>(),
                "plans": plans_between(&workspace, from, to)
            }))
        }
        "courses" => {
            let year = args
                .get("year")
                .and_then(Value::as_i64)
                .ok_or("year is required")?;
            let term = format!("{year}-{}", required(args, "quarter")?);
            parse_term(&term)?;
            Ok(Value::Array(scheduled_courses(&workspace, &term)))
        }
        "course" => {
            let id = required(args, "id")?;
            let course = visible_rows(&workspace, "courses")
                .into_iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| "course was not found".to_string())?;
            Ok(json!({
                "course": course,
                "lmsItems": store.table_rows("lms_items")?.into_iter()
                    .filter(|row| row.get("courseId").and_then(Value::as_str) == Some(id)
                        && row.get("visible") != Some(&Value::Bool(false))).collect::<Vec<_>>(),
                "assignments": visible_rows(&workspace, "assignments").into_iter()
                    .filter(|row| row.get("courseId").and_then(Value::as_str) == Some(id)
                        && row.get("hidden") != Some(&Value::Bool(true))).collect::<Vec<_>>(),
                "syllabus": store.table_rows("offering_content")?.into_iter()
                    .filter(|row| row.get("offeringId").and_then(Value::as_str)
                        == course.get("offeringId").and_then(Value::as_str)).collect::<Vec<_>>()
            }))
        }
        "search_catalog" => {
            // Search-only rows lack the schedule and credits plan_add needs;
            // the app reads them from the syllabus site, which this server never contacts.
            let mut rows = store.catalog_search(required(args, "query")?)?;
            for row in &mut rows {
                let plannable = match text(row, "id") {
                    Some(id) => store.catalog_offering(id)?.is_some(),
                    None => false,
                };
                row["plannable"] = Value::Bool(plannable);
            }
            Ok(Value::Array(rows))
        }
        "registration" => {
            let year = args.get("year").and_then(Value::as_i64);
            let term = args.get("term").and_then(Value::as_str);
            Ok(Value::Array(
                store
                    .table_rows("registrations")?
                    .into_iter()
                    .filter(|row| {
                        let offering = row
                            .get("offeringId")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let matches_year = year.is_none_or(|year| {
                            offering.get(..4).and_then(|v| v.parse::<i64>().ok()) == Some(year)
                        });
                        let matches_term = term.is_none_or(|term| {
                            let number = term.trim_end_matches('Q').parse::<u64>().ok();
                            row.get("termId")
                                .and_then(Value::as_str)
                                .is_some_and(|value| value.ends_with(term))
                                || row.get("quarters").and_then(Value::as_array).is_some_and(
                                    |quarters| {
                                        quarters.iter().any(|quarter| {
                                            quarter.as_u64() == number
                                                || quarter.as_str() == Some(term)
                                        })
                                    },
                                )
                        });
                        matches_year && matches_term
                    })
                    .collect(),
            ))
        }
        "grades" => Ok(Value::Array(store.table_rows("grades")?)),
        "gpa" => Ok(json!({
            "summary": store.table_rows("gpa_summary")?,
            "byTerm": store.table_rows("gpa_by_term")?
        })),
        "requirements" => Ok(Value::Array(store.table_rows("requirement_progress")?)),
        "recommended" => {
            let department = args
                .get("departmentId")
                .and_then(Value::as_str)
                .or_else(|| {
                    workspace
                        .pointer("/settings/departmentId")
                        .and_then(Value::as_str)
                })
                .ok_or("departmentId is required: no department is chosen in Settings")?;
            let admission = workspace
                .pointer("/academicProfile/admissionYear")
                .and_then(Value::as_i64);
            Ok(Value::Array(
                visible_rows(&workspace, "recommendedCourses")
                    .into_iter()
                    .filter(|row| {
                        // Settings may hold a scoped id such as "school:ee".
                        text(row, "departmentId").is_some_and(|id| {
                            id == department || department.ends_with(&format!(":{id}"))
                        }) && admission.is_none_or(|year| {
                            row.get("admissionYear").and_then(Value::as_i64) == Some(year)
                        })
                    })
                    .collect(),
            ))
        }
        "plan_add" => {
            let offering_id = required(args, "offeringId")?;
            let term = required(args, "term")?;
            let (year, quarter) = parse_term(term)?;
            let Some(offering) = store.catalog_offering(offering_id)? else {
                return Err(if store.catalog_search_item(offering_id)?.is_some() {
                    "this course's schedule is not synced yet (plannable: false). Open it once in IsctCompanion, then try again."
                } else {
                    "offering was not found in the local catalog"
                }
                .into());
            };
            if offering.get("year").and_then(Value::as_i64) != Some(year) {
                return Err("offering does not belong to the requested academic year".into());
            }
            let quarters = offering
                .get("quarters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !quarters.is_empty() && !quarters.contains(&quarter) {
                return Err(format!(
                    "offering is taught in {}, not {quarter}",
                    quarters.join(", ")
                )
                .into());
            }
            // One course per offering and year, whatever quarter it sits in
            // (like addSyllabusDraft): writing a new one would replace it.
            let canonical = format!("{year}-{offering_id}");
            let existing = visible_rows(&workspace, "courses").into_iter().find(|row| {
                text(row, "id") == Some(canonical.as_str())
                    || text(row, "offeringId") == Some(offering_id)
            });
            // The timetable lists a course only under its own quarter, so a
            // plan for another quarter would never show up.
            if let Some(other) = existing
                .as_ref()
                .and_then(|row| text(row, "termId"))
                .filter(|other| *other != term)
            {
                return Err(format!(
                    "this offering is already on the {other} timetable; plan it for {other}"
                )
                .into());
            }
            let course_value = course_from_offering(&offering, term)?;
            let details = agent_write_result(store.agent_write(name, |write| {
                let stored_course = write
                    .rows("manual_courses")?
                    .into_iter()
                    .find(|row| text(row, "offeringId") == Some(offering_id));
                let course = match stored_course {
                    Some(course) if course.get("deletedAt").is_none() => course,
                    Some(course) => write.put(
                        "manual_courses",
                        course_value,
                        course.get("rev").and_then(Value::as_u64),
                    )?,
                    None => match existing.clone() {
                        Some(course) => course,
                        None => write.put("manual_courses", course_value, Some(0))?,
                    },
                };
                if let Some(other) = text(&course, "termId").filter(|other| *other != term) {
                    return Err(DataError::InvalidInput(format!(
                        "this offering is already on the {other} timetable; plan it for {other}"
                    )));
                }
                let course_id = text(&course, "id")
                    .ok_or_else(|| {
                        DataError::InvalidInput("course identity could not be created".into())
                    })?
                    .to_string();
                let planned = write.rows("plan_items")?.into_iter().find(|row| {
                    text(row, "courseId") == Some(course_id.as_str())
                        && text(row, "termId") == Some(term)
                        && row.get("deletedAt").is_none()
                });
                let plan_item = match planned {
                    Some(row) => row,
                    None => {
                        let plan_id = format!("mcp-plan:{term}:{offering_id}");
                        let value = json!({"id":plan_id.clone(),"kind":"plan-item","termId":term,
                            "offeringId":offering_id,"courseId":course_id,"state":"planned",
                            "source":{"provider":"mcp"}});
                        let existing = write
                            .rows("plan_items")?
                            .into_iter()
                            .find(|row| text(row, "id") == Some(plan_id.as_str()));
                        let expected = existing
                            .as_ref()
                            .and_then(|row| row.get("rev"))
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        write.put("plan_items", value, Some(expected))?
                    }
                };
                Ok(json!({"courseId":course_id,"planItemId":plan_item.get("id")}))
            }))?;
            Ok(write_result(details))
        }
        "plan_remove" => {
            let offering_id = required(args, "offeringId")?;
            let term = required(args, "term")?;
            parse_term(term)?;
            let removed = agent_write_result(store.agent_write(name, |write| {
                let mut removed = 0;
                for row in write.rows("plan_items")? {
                    if text(&row, "offeringId") == Some(offering_id)
                        && text(&row, "termId") == Some(term)
                        && row.get("deletedAt").is_none()
                    {
                        if let Some(id) = text(&row, "id") {
                            write.delete(
                                "plan_items",
                                id,
                                row.get("rev").and_then(Value::as_u64),
                            )?;
                            removed += 1;
                        }
                    }
                }
                if removed == 0 {
                    return Err(DataError::InvalidInput(format!(
                        "offering {offering_id} is not planned for {term}"
                    )));
                }
                Ok(removed)
            }))?;
            Ok(write_result(json!({"removed": removed})))
        }
        "plan_event_create" => {
            let start = required(args, "start")?;
            valid_plan_time(start)?;
            let duration = duration(args, true)?
                .ok_or_else(|| AgentError::InvalidInput("duration is required".into()))?;
            let rule = args.get("rrule").and_then(Value::as_str);
            if let Some(rule) = rule {
                crate::plans::validate_rrule(rule)
                    .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
            }
            let value = json!({
            "title": required(args, "title")?,
            "start": start, "duration": duration,
            "rrule": rule, "room": args.get("room").cloned().unwrap_or(Value::Null),
            "exdates": [], "source": {"provider":"mcp"}
            });
            let plan = agent_write_result(
                store.agent_write(name, |write| write.put("plans", value, None)),
            )?;
            Ok(write_result(json!({"id": plan.get("id")})))
        }
        "plan_event_update" => {
            let id = required(args, "id")?;
            let mut row = live_plan(store, id)?;
            if let Some(start) = args.get("start").and_then(Value::as_str) {
                valid_plan_time(start)?;
            }
            duration(args, false)?;
            if let Some(rule) = args.get("rrule").and_then(Value::as_str) {
                crate::plans::validate_rrule(rule)
                    .map_err(|error| AgentError::InvalidInput(error.to_string()))?;
            }
            for key in ["title", "start", "duration", "rrule", "room"] {
                if let Some(value) = args.get(key) {
                    row[key] = value.clone();
                }
            }
            let expected = row.get("rev").and_then(Value::as_u64);
            agent_write_result(
                store.agent_write(name, |write| write.put("plans", row, expected).map(|_| ())),
            )?;
            Ok(write_result(json!({"id": id})))
        }
        "plan_event_delete" => {
            let id = required(args, "id")?;
            let expected = live_plan(store, id)?.get("rev").and_then(Value::as_u64);
            if !agent_write_result(
                store.agent_write(name, |write| write.delete("plans", id, expected)),
            )? {
                return Err("plan was not found".into());
            }
            Ok(write_result(json!({"id": id})))
        }
        "hide_assignment" | "restore_assignment" => {
            let id = required(args, "id")?;
            let assignment = workspace["assignments"]
                .as_array()
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                })
                .ok_or_else(|| "assignment was not found".to_string())?;
            let source_key = assignment
                .get("sourceKey")
                .and_then(Value::as_str)
                .unwrap_or(id);
            let mut mark = store
                .user_rows("assignment_marks")?
                .into_iter()
                .find(|row| row.get("sourceKey").and_then(Value::as_str) == Some(source_key))
                .unwrap_or_else(|| json!({"sourceKey":source_key,"assignmentId":id}));
            let expected = mark.get("rev").and_then(Value::as_u64).unwrap_or(0);
            mark["hidden"] = Value::Bool(name == "hide_assignment");
            agent_write_result(store.agent_write(name, |write| {
                write
                    .put("assignment_marks", mark, Some(expected))
                    .map(|_| ())
            }))?;
            Ok(write_result(json!({"id": id})))
        }
        _ => Err(AgentError::UnknownTool),
    }
}

#[cfg(test)]
mod tests;
