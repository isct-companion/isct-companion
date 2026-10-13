use isct_companion_lib::data::DataStore;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "today",
            "Today in Japan time: the calendar state (holiday, replacement day), classes actually held, personal plans, open assignments not yet due, open assignments overdue in the last 14 days, and the GPA summary.",
            json!({}),
            &[],
        ),
        tool(
            "schedule",
            "Registered or planned courses of the quarters in an inclusive ISO date range (maximum 366 days), intensive dates and personal plans with their occurrences.",
            json!({
                "from": {"type": "string", "format": "date"},
                "to": {"type": "string", "format": "date"}
            }),
            &["from", "to"],
        ),
        tool(
            "courses",
            "Registered or planned courses for one academic quarter.",
            json!({
                "year": {"type": "integer"}, "quarter": {"enum": ["1Q", "2Q", "3Q", "4Q"]}
            }),
            &["year", "quarter"],
        ),
        tool(
            "course",
            "One course with syllabus, LMS items and assignments.",
            json!({
                "id": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "search_catalog",
            "Search the local bilingual course catalog. Only rows with plannable: true can be passed to plan_add; the others need their syllabus opened once in IsctCompanion first.",
            json!({
                "query": {"type": "string"}
            }),
            &["query"],
        ),
        tool(
            "registration",
            "Imported read-only registration records.",
            json!({
                "year": {"type": "integer"}, "term": {"enum": ["1Q", "2Q", "3Q", "4Q"]}
            }),
            &[],
        ),
        tool("grades", "Imported official grade records.", json!({}), &[]),
        tool(
            "gpa",
            "Imported GPA, GPT and credit summaries.",
            json!({}),
            &[],
        ),
        tool(
            "requirements",
            "Imported graduation requirement progress.",
            json!({}),
            &[],
        ),
        tool(
            "recommended",
            "Recommended courses for the person's admission year. departmentId defaults to the department chosen in Settings.",
            json!({
                "departmentId": {"type": "string"}
            }),
            &[],
        ),
        tool(
            "plan_add",
            "Add a catalog offering (plannable: true in search_catalog) to the person's local plan. term is YYYY-nQ and must be one of the offering's quarters.",
            json!({
                "offeringId": {"type": "string"}, "term": {"type": "string"}
            }),
            &["offeringId", "term"],
        ),
        tool(
            "plan_remove",
            "Remove an offering from the person's local plan.",
            json!({
                "offeringId": {"type": "string"}, "term": {"type": "string"}
            }),
            &["offeringId", "term"],
        ),
        tool(
            "plan_event_create",
            "Create a personal one-off or repeating plan and return its id. start is RFC 3339; duration is minutes.",
            json!({
                "title": {"type": "string"}, "start": {"type": "string", "format": "date-time"},
                "duration": {"type": "integer", "minimum": 1, "maximum": 10080, "description": "Duration in minutes."}, "rrule": {"type": "string"},
                "room": {"type": "string"}
            }),
            &["title", "start", "duration"],
        ),
        tool(
            "plan_event_update",
            "Update a personal plan. start is RFC 3339; duration is minutes.",
            json!({
                "id": {"type": "string"}, "title": {"type": "string"},
                "start": {"type": "string", "format": "date-time"}, "duration": {"type": "integer", "minimum": 1, "maximum": 10080, "description": "Duration in minutes."},
                "rrule": {"type": "string"}, "room": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "plan_event_delete",
            "Delete a personal plan.",
            json!({
                "id": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "hide_assignment",
            "Hide an assignment locally without changing LMS.",
            json!({
                "id": {"type": "string"}
            }),
            &["id"],
        ),
        tool(
            "restore_assignment",
            "Restore a locally hidden assignment.",
            json!({
                "id": {"type": "string"}
            }),
            &["id"],
        ),
    ]
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required"))
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

fn date_range(args: &Value) -> Result<(chrono::NaiveDate, chrono::NaiveDate), String> {
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
fn parse_term(term: &str) -> Result<(i64, &str), String> {
    let invalid = || format!("term must look like 2026-3Q, got {term}");
    let (year, quarter) = term.split_once('-').ok_or_else(invalid)?;
    let year = year
        .parse::<i64>()
        .ok()
        .filter(|year| (1000..=9999).contains(year))
        .ok_or_else(invalid)?;
    if !matches!(quarter, "1Q" | "2Q" | "3Q" | "4Q") {
        return Err(invalid());
    }
    Ok((year, quarter))
}

fn now_jst() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(9 * 3600).unwrap())
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
                let occurrences = isct_companion_lib::plans::occurrences_between(
                    &start, rule, &exdates, from, to,
                )
                .ok()?;
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

fn course_from_offering(offering: &Value, term: &str) -> Result<Value, String> {
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

fn write_result(store: &DataStore, tool_name: &str, details: Value) -> Result<Value, String> {
    store.record_agent_write(tool_name)?;
    let mut result = json!({"ok": true});
    if let (Some(result), Value::Object(details)) = (result.as_object_mut(), details) {
        result.extend(details);
    }
    Ok(result)
}

/// A personal plan that has not been deleted.
fn live_plan(store: &DataStore, id: &str) -> Result<Value, String> {
    store
        .user_rows("plans")?
        .into_iter()
        .find(|row| text(row, "id") == Some(id) && row.get("deletedAt").is_none())
        .ok_or_else(|| "plan was not found".to_string())
}

fn valid_plan_time(value: &str) -> Result<(), String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| "start must be an RFC 3339 timestamp".to_string())
}

fn duration(args: &Value, required: bool) -> Result<Option<u64>, String> {
    let value = args.get("duration").and_then(Value::as_u64);
    if required && value.is_none() {
        return Err("duration is required".into());
    }
    if value.is_some_and(|minutes| !(1..=10_080).contains(&minutes)) {
        return Err("duration must be between 1 and 10080 minutes".into());
    }
    Ok(value)
}

fn call(store: &DataStore, name: &str, args: &Value) -> Result<Value, String> {
    if !store.agent_access_enabled()? {
        return Err("IsctCompanion agent access is disabled in Settings.".into());
    }
    let workspace = store.load_workspace()?;
    match name {
        "today" => {
            let now = now_jst();
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
                ));
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
                ));
            }
            let course_id = match existing.as_ref().and_then(|row| text(row, "id")) {
                Some(id) => id.to_string(),
                None => {
                    let course = store.put_user_record(
                        "manual_courses",
                        course_from_offering(&offering, term)?,
                        "mcp",
                    )?;
                    text(&course, "id")
                        .ok_or("course identity could not be created")?
                        .to_string()
                }
            };
            let planned = store.user_rows("plan_items")?.into_iter().find(|row| {
                text(row, "courseId") == Some(course_id.as_str())
                    && text(row, "termId") == Some(term)
                    && row.get("deletedAt").is_none()
            });
            let plan_item = match planned {
                Some(row) => row,
                None => store.put_user_record(
                    "plan_items",
                    json!({
                        "kind": "plan-item",
                        "termId": term, "offeringId": offering_id, "courseId": course_id, "state": "planned",
                        "source": {"provider":"mcp"}
                    }),
                    "mcp",
                )?,
            };
            write_result(
                store,
                name,
                json!({"courseId": course_id, "planItemId": plan_item.get("id")}),
            )
        }
        "plan_remove" => {
            let offering_id = required(args, "offeringId")?;
            let term = required(args, "term")?;
            parse_term(term)?;
            let mut removed = 0;
            for row in store.user_rows("plan_items")? {
                if text(&row, "offeringId") == Some(offering_id)
                    && text(&row, "termId") == Some(term)
                    && row.get("deletedAt").is_none()
                {
                    if let Some(id) = text(&row, "id") {
                        store.delete_user_record_checked(
                            "plan_items",
                            id,
                            "mcp",
                            row.get("rev").and_then(Value::as_u64),
                        )?;
                        removed += 1;
                    }
                }
            }
            if removed == 0 {
                return Err(format!("offering {offering_id} is not planned for {term}"));
            }
            write_result(store, name, json!({"removed": removed}))
        }
        "plan_event_create" => {
            let start = required(args, "start")?;
            valid_plan_time(start)?;
            let duration = duration(args, true)?.unwrap();
            let rule = args.get("rrule").and_then(Value::as_str);
            if let Some(rule) = rule {
                isct_companion_lib::plans::validate_rrule(rule).map_err(|error| error.to_string())?;
            }
            let plan = store.put_user_record(
                "plans",
                json!({
                    "title": required(args, "title")?,
                    "start": start, "duration": duration,
                    "rrule": rule, "room": args.get("room").cloned().unwrap_or(Value::Null),
                    "exdates": [], "source": {"provider":"mcp"}
                }),
                "mcp",
            )?;
            write_result(store, name, json!({"id": plan.get("id")}))
        }
        "plan_event_update" => {
            let id = required(args, "id")?;
            let mut row = live_plan(store, id)?;
            if let Some(start) = args.get("start").and_then(Value::as_str) {
                valid_plan_time(start)?;
            }
            duration(args, false)?;
            if let Some(rule) = args.get("rrule").and_then(Value::as_str) {
                isct_companion_lib::plans::validate_rrule(rule).map_err(|error| error.to_string())?;
            }
            for key in ["title", "start", "duration", "rrule", "room"] {
                if let Some(value) = args.get(key) {
                    row[key] = value.clone();
                }
            }
            let expected = row.get("rev").and_then(Value::as_u64);
            store.put_user_record_checked("plans", row, "mcp", expected)?;
            write_result(store, name, json!({"id": id}))
        }
        "plan_event_delete" => {
            let id = required(args, "id")?;
            let expected = live_plan(store, id)?.get("rev").and_then(Value::as_u64);
            if !store.delete_user_record_checked("plans", id, "mcp", expected)? {
                return Err("plan was not found".into());
            }
            write_result(store, name, json!({"id": id}))
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
            store.put_user_record_checked("assignment_marks", mark, "mcp", Some(expected))?;
            write_result(store, name, json!({"id": id}))
        }
        _ => Err("unknown tool".into()),
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let mut database = None;
    while let Some(argument) = arguments.next() {
        if argument == "--db" {
            database = arguments.next().map(PathBuf::from);
        }
    }
    let Some(database) = database else {
        eprintln!("isct-companion-mcp requires --db /path/to/school.db");
        std::process::exit(2);
    };
    let store = match DataStore::initialize(database) {
        Ok((store, _)) => store,
        Err(error) => {
            eprintln!("isct-companion-mcp could not open the database: {error}");
            std::process::exit(2);
        }
    };
    let enabled = store.agent_access_enabled().unwrap_or(false);
    if !enabled {
        eprintln!("IsctCompanion agent access is disabled in Settings.");
        std::process::exit(3);
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let id = request.get("id").cloned();
        let Some(id) = id else { continue };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let answer = match method {
            "initialize" => response(
                id,
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "IsctCompanion", "version": env!("CARGO_PKG_VERSION")}
                }),
            ),
            "ping" => response(id, json!({})),
            "tools/list" => response(id, json!({"tools": tools()})),
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match call(&store, name, &args).map(|mut value| {
                    slim(&mut value);
                    value
                }) {
                    Ok(value) => response(
                        id,
                        json!({
                            "content": [{"type": "text", "text": serde_json::to_string(&value).unwrap()}],
                            "structuredContent": value
                        }),
                    ),
                    Err(error) => response(
                        id,
                        json!({
                            "content": [{"type": "text", "text": error}],
                            "isError": true
                        }),
                    ),
                }
            }
            _ => failure(id, -32601, "Method not found"),
        };
        let _ = writeln!(stdout, "{}", answer);
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> Value {
        let monday = json!([{"days": [1], "periods": [1, 2], "startTime": "08:50", "endTime": "10:30", "room": "M-102"}]);
        json!({
            "courses": [
                {"id": "c1", "termId": "2026-3Q", "code": "A", "title": "Registered", "meetings": monday},
                {"id": "c2", "termId": "2026-3Q", "code": "B", "title": "Not on the timetable", "meetings": monday},
                {"id": "c3", "termId": "2026-3Q", "code": "C", "title": "Intensive", "meetings": []}
            ],
            "memberships": [
                {"courseId": "c1", "termId": "2026-3Q"},
                {"courseId": "c3", "termId": "2026-3Q"}
            ],
            "planItems": [],
            "exceptions": [{"date": "2026-10-19", "type": "no-class"}],
            "academicCalendar": [
                {"id": "calendar:2026:quarter-start:3Q", "type": "quarter-start",
                 "teachingRanges": [{"start": "2026-10-01", "end": "2026-11-30"}]},
                {"date": "2026-10-12", "type": "holiday"},
                {"date": "2026-10-15", "type": "replacement", "replacementDay": 1}
            ],
            "intensiveSchedules": [{"courseId": "c3", "from": "2026-10-20", "to": "2026-10-22",
                "days": [{"date": "2026-10-21", "periods": [1, 2]}]}]
        })
    }

    fn titles(date: &str) -> Vec<String> {
        let date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        classes_on(&workspace(), "2026-3Q", date)
            .iter()
            .map(|class| class["title"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn classes_follow_the_school_calendar() {
        assert_eq!(titles("2026-10-05"), ["Registered"]); // ordinary Monday
        assert!(titles("2026-10-06").is_empty()); // Tuesday
        assert!(titles("2026-10-12").is_empty()); // holiday
        assert_eq!(titles("2026-10-15"), ["Registered"]); // Thursday teaching Monday classes
        assert!(titles("2026-10-19").is_empty()); // personal no-class exception
        assert_eq!(titles("2026-10-21"), ["Intensive"]); // chosen intensive day only
        assert!(titles("2026-10-20").is_empty());
        assert!(titles("2026-12-07").is_empty()); // Monday outside the teaching range
    }

    #[test]
    fn terms_are_validated() {
        assert_eq!(parse_term("2026-3Q"), Ok((2026, "3Q")));
        for bad in ["2026-banana", "2026-5Q", "spring", "26-1Q"] {
            assert!(parse_term(bad).is_err(), "{bad}");
        }
    }
}
