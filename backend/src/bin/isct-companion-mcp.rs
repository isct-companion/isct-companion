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
            "Teaching state, classes, assignments and academic summary for today.",
            json!({}),
            &[],
        ),
        tool(
            "schedule",
            "Classes, intensive dates and personal plans intersecting an inclusive ISO date range (maximum 366 days).",
            json!({
                "from": {"type": "string", "format": "date"},
                "to": {"type": "string", "format": "date"}
            }),
            &["from", "to"],
        ),
        tool(
            "courses",
            "Timetable courses for one academic quarter.",
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
            "Search the local bilingual course catalog.",
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
            "Recommended courses for a department.",
            json!({
                "departmentId": {"type": "string"}
            }),
            &[],
        ),
        tool(
            "plan_add",
            "Add a catalog offering to the person's local plan.",
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
            "Create a personal one-off or repeating plan. start is RFC 3339; duration is minutes.",
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

fn write_result(store: &DataStore, tool_name: &str) -> Result<Value, String> {
    store.record_agent_write(tool_name)?;
    Ok(json!({"ok": true}))
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
        "today" => Ok(json!({
            "courses": visible_rows(&workspace, "courses"),
            "assignments": visible_rows(&workspace, "assignments")
                .into_iter().filter(|row| row.get("hidden") != Some(&Value::Bool(true))).collect::<Vec<_>>(),
            "plans": visible_rows(&workspace, "plans"),
            "academicCalendar": visible_rows(&workspace, "academicCalendar"),
            "academicSummary": visible_rows(&workspace, "gpaSummary")
        })),
        "schedule" => {
            let (from, to) = date_range(args)?;
            let mut terms = std::collections::HashSet::new();
            let mut cursor = from;
            while cursor <= to {
                terms.insert(fallback_term(cursor));
                cursor += chrono::Duration::days(1);
            }
            let plans = visible_rows(&workspace, "plans")
                .into_iter()
                .filter_map(|mut row| {
                    let start = row.get("start").and_then(Value::as_str)?.to_string();
                    if let Some(rule) = row.get("rrule").and_then(Value::as_str) {
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
                .collect::<Vec<_>>();
            Ok(json!({
                "from": from.to_string(), "to": to.to_string(),
                "courses": visible_rows(&workspace, "courses").into_iter()
                    .filter(|row| row.get("termId").and_then(Value::as_str).is_some_and(|term| terms.contains(term)))
                    .collect::<Vec<_>>(),
                "intensiveSchedules": visible_rows(&workspace, "intensiveSchedules").into_iter()
                    .filter(|row| {
                        let start = row.get("from").and_then(Value::as_str).and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
                        let end = row.get("to").and_then(Value::as_str).and_then(|v| chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
                        matches!((start,end),(Some(start),Some(end)) if start <= to && end >= from)
                    }).collect::<Vec<_>>(),
                "plans": plans
            }))
        }
        "courses" => {
            let term = format!(
                "{}-{}",
                args.get("year")
                    .and_then(Value::as_i64)
                    .ok_or("year is required")?,
                required(args, "quarter")?
            );
            Ok(Value::Array(
                visible_rows(&workspace, "courses")
                    .into_iter()
                    .filter(|row| row.get("termId").and_then(Value::as_str) == Some(&term))
                    .collect(),
            ))
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
        "search_catalog" => Ok(Value::Array(
            store.catalog_search(required(args, "query")?)?,
        )),
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
            let department = args.get("departmentId").and_then(Value::as_str);
            Ok(Value::Array(
                visible_rows(&workspace, "recommendedCourses")
                    .into_iter()
                    .filter(|row| {
                        department.is_none()
                            || row.get("departmentId").and_then(Value::as_str) == department
                    })
                    .collect(),
            ))
        }
        "plan_add" => {
            let offering_id = required(args, "offeringId")?;
            let term = required(args, "term")?;
            let offering = store
                .catalog_offering(offering_id)?
                .ok_or_else(|| "offering was not found in the local catalog".to_string())?;
            let expected_year = term.get(..4).and_then(|value| value.parse::<i64>().ok());
            if offering.get("year").and_then(Value::as_i64) != expected_year {
                return Err("offering does not belong to the requested academic year".into());
            }
            let mut course_id = visible_rows(&workspace, "courses")
                .into_iter()
                .find(|row| {
                    row.get("offeringId").and_then(Value::as_str) == Some(offering_id)
                        && row.get("termId").and_then(Value::as_str) == Some(term)
                })
                .and_then(|row| row.get("id").and_then(Value::as_str).map(str::to_string));
            if course_id.is_none() {
                let course = store.put_user_record(
                    "manual_courses",
                    course_from_offering(&offering, term)?,
                    "mcp",
                )?;
                course_id = course.get("id").and_then(Value::as_str).map(str::to_string);
            }
            let course_id = course_id.ok_or("course identity could not be created")?;
            if !store.user_rows("plan_items")?.iter().any(|row| {
                row.get("courseId").and_then(Value::as_str) == Some(course_id.as_str())
                    && row.get("termId").and_then(Value::as_str) == Some(term)
                    && row.get("deletedAt").is_none()
            }) {
                store.put_user_record(
                    "plan_items",
                    json!({
                        "kind": "plan-item",
                        "termId": term, "offeringId": offering_id, "courseId": course_id, "state": "planned",
                        "source": {"provider":"mcp"}
                    }),
                    "mcp",
                )?;
            }
            write_result(store, name)
        }
        "plan_remove" => {
            let offering_id = required(args, "offeringId")?;
            let term = required(args, "term")?;
            for row in store.user_rows("plan_items")? {
                if row.get("offeringId").and_then(Value::as_str) == Some(offering_id)
                    && row.get("termId").and_then(Value::as_str) == Some(term)
                    && row.get("deletedAt").is_none()
                {
                    if let Some(id) = row.get("id").and_then(Value::as_str) {
                        store.delete_user_record_checked(
                            "plan_items",
                            id,
                            "mcp",
                            row.get("rev").and_then(Value::as_u64),
                        )?;
                    }
                }
            }
            write_result(store, name)
        }
        "plan_event_create" => {
            let start = required(args, "start")?;
            valid_plan_time(start)?;
            let duration = duration(args, true)?.unwrap();
            let rule = args.get("rrule").and_then(Value::as_str);
            if let Some(rule) = rule {
                isct_companion_lib::plans::validate_rrule(rule)?;
            }
            store.put_user_record(
                "plans",
                json!({
                    "title": required(args, "title")?,
                    "start": start, "duration": duration,
                    "rrule": rule, "room": args.get("room").cloned().unwrap_or(Value::Null),
                    "exdates": [], "source": {"provider":"mcp"}
                }),
                "mcp",
            )?;
            write_result(store, name)
        }
        "plan_event_update" => {
            let id = required(args, "id")?;
            let mut row = store
                .user_rows("plans")?
                .into_iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| "plan was not found".to_string())?;
            if let Some(start) = args.get("start").and_then(Value::as_str) {
                valid_plan_time(start)?;
            }
            duration(args, false)?;
            if let Some(rule) = args.get("rrule").and_then(Value::as_str) {
                isct_companion_lib::plans::validate_rrule(rule)?;
            }
            for key in ["title", "start", "duration", "rrule", "room"] {
                if let Some(value) = args.get(key) {
                    row[key] = value.clone();
                }
            }
            let expected = row.get("rev").and_then(Value::as_u64);
            store.put_user_record_checked("plans", row, "mcp", expected)?;
            write_result(store, name)
        }
        "plan_event_delete" => {
            let id = required(args, "id")?;
            let expected = store
                .user_rows("plans")?
                .into_iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .and_then(|row| row.get("rev").and_then(Value::as_u64));
            if !store.delete_user_record_checked("plans", id, "mcp", expected)? {
                return Err("plan was not found".into());
            }
            write_result(store, name)
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
            write_result(store, name)
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
                match call(&store, name, &args) {
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
