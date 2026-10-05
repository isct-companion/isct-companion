use science_tokyo_workstation_lib::data::DataStore;
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
            "Classes, intensive dates and personal plans in a date range.",
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
                "year": {"type": "integer"}, "term": {"type": "string"}
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
            "Create a personal one-off or repeating plan.",
            json!({
                "title": {"type": "string"}, "start": {"type": "string"},
                "duration": {"type": "integer", "minimum": 1}, "rrule": {"type": "string"},
                "room": {"type": "string"}
            }),
            &["title", "start", "duration"],
        ),
        tool(
            "plan_event_update",
            "Update a personal plan.",
            json!({
                "id": {"type": "string"}, "title": {"type": "string"},
                "start": {"type": "string"}, "duration": {"type": "integer"},
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
        .filter(|row| row.get("deletedAt").is_none())
        .cloned()
        .collect()
}

fn stamp_mcp(row: &mut Value) {
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Some(object) = row.as_object_mut() {
        let rev = object.get("rev").and_then(Value::as_u64).unwrap_or(0) + 1;
        object.insert("rev".into(), Value::from(rev));
        object.insert("updatedAt".into(), Value::String(timestamp));
        object.insert("updatedBy".into(), Value::String("mcp".into()));
    }
}

fn write_result(store: &DataStore, mut workspace: Value, tool_name: &str) -> Result<Value, String> {
    if let Some(writes) = workspace
        .pointer_mut("/settings/agentAccess/lastWrites")
        .and_then(Value::as_array_mut)
    {
        writes.insert(
            0,
            json!({"at": chrono::Utc::now().to_rfc3339(), "tool": tool_name}),
        );
        writes.truncate(20);
    }
    store.save_workspace(&workspace)?;
    Ok(json!({"ok": true}))
}

fn call(store: &DataStore, name: &str, args: &Value) -> Result<Value, String> {
    let mut workspace = store.load_workspace()?;
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
            let from = required(args, "from")?;
            let to = required(args, "to")?;
            Ok(json!({
                "from": from, "to": to,
                "courses": visible_rows(&workspace, "courses"),
                "intensiveSchedules": visible_rows(&workspace, "intensiveSchedules"),
                "plans": visible_rows(&workspace, "plans")
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
                    .filter(|row| row.get("courseId").and_then(Value::as_str) == Some(id)).collect::<Vec<_>>(),
                "assignments": visible_rows(&workspace, "assignments").into_iter()
                    .filter(|row| row.get("courseId").and_then(Value::as_str) == Some(id)).collect::<Vec<_>>(),
                "syllabus": store.table_rows("offering_content")?
            }))
        }
        "search_catalog" => Ok(Value::Array(
            store.catalog_search(required(args, "query")?)?,
        )),
        "registration" => Ok(Value::Array(store.table_rows("registrations")?)),
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
            if !workspace["planItems"].as_array().is_some_and(|rows| {
                rows.iter().any(|row| {
                    row.get("offeringId").and_then(Value::as_str) == Some(offering_id)
                        && row.get("termId").and_then(Value::as_str) == Some(term)
                        && row.get("deletedAt").is_none()
                })
            }) {
                workspace["planItems"].as_array_mut().unwrap().push(json!({
                    "id": uuid::Uuid::new_v4().to_string(), "kind": "plan-item",
                    "termId": term, "offeringId": offering_id, "state": "planned",
                    "rev": 1, "updatedAt": chrono::Utc::now().to_rfc3339(),
                    "updatedBy": "mcp"
                }));
            }
            write_result(store, workspace, name)
        }
        "plan_remove" => {
            let offering_id = required(args, "offeringId")?;
            let term = required(args, "term")?;
            for row in workspace["planItems"].as_array_mut().unwrap() {
                if row.get("offeringId").and_then(Value::as_str) == Some(offering_id)
                    && row.get("termId").and_then(Value::as_str) == Some(term)
                    && row.get("deletedAt").is_none()
                {
                    stamp_mcp(row);
                    row["deletedAt"] = Value::String(chrono::Utc::now().to_rfc3339());
                }
            }
            write_result(store, workspace, name)
        }
        "plan_event_create" => {
            let rule = args.get("rrule").and_then(Value::as_str);
            if let Some(rule) = rule {
                science_tokyo_workstation_lib::plans::validate_rrule(rule)?;
            }
            workspace["plans"].as_array_mut().unwrap().push(json!({
                "id": uuid::Uuid::new_v4().to_string(),
                "title": required(args, "title")?,
                "start": required(args, "start")?,
                "duration": args.get("duration").and_then(Value::as_u64).ok_or("duration is required")?,
                "rrule": rule, "room": args.get("room"), "exdates": [],
                "rev": 1, "updatedAt": chrono::Utc::now().to_rfc3339(), "updatedBy": "mcp"
            }));
            write_result(store, workspace, name)
        }
        "plan_event_update" => {
            let id = required(args, "id")?;
            let row = workspace["plans"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| "plan was not found".to_string())?;
            if let Some(rule) = args.get("rrule").and_then(Value::as_str) {
                science_tokyo_workstation_lib::plans::validate_rrule(rule)?;
            }
            for key in ["title", "start", "duration", "rrule", "room"] {
                if let Some(value) = args.get(key) {
                    row[key] = value.clone();
                }
            }
            stamp_mcp(row);
            write_result(store, workspace, name)
        }
        "plan_event_delete" => {
            let id = required(args, "id")?;
            let row = workspace["plans"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| "plan was not found".to_string())?;
            stamp_mcp(row);
            row["deletedAt"] = Value::String(chrono::Utc::now().to_rfc3339());
            write_result(store, workspace, name)
        }
        "hide_assignment" | "restore_assignment" => {
            let id = required(args, "id")?;
            let row = workspace["assignments"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| "assignment was not found".to_string())?;
            row["hidden"] = Value::Bool(name == "hide_assignment");
            stamp_mcp(row);
            write_result(store, workspace, name)
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
        eprintln!("campus-mcp requires --db /path/to/school.db");
        std::process::exit(2);
    };
    let store = match DataStore::initialize(database) {
        Ok((store, _)) => store,
        Err(error) => {
            eprintln!("campus-mcp could not open the database: {error}");
            std::process::exit(2);
        }
    };
    let enabled = store
        .load_workspace()
        .ok()
        .and_then(|workspace| {
            workspace
                .pointer("/settings/agentAccess/enabled")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    if !enabled {
        eprintln!("Campus Life agent access is disabled in Settings.");
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
                    "serverInfo": {"name": "campus-life", "version": env!("CARGO_PKG_VERSION")}
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
