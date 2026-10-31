use super::*;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

struct Fixture {
    root: PathBuf,
    store: DataStore,
    notes: crate::notes::NoteStore,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("isct-agent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize_dir(root.clone()).unwrap().0;
        store
            .put_workspace_setting(
                "devicePreferences",
                json!({"agentAccess":{"enabled":true,"lastWrites":[]}}),
            )
            .unwrap();
        let notes = crate::notes::NoteStore::default();
        notes.at(root.join("files"));
        Self { root, store, notes }
    }

    fn service(&self) -> AgentService {
        AgentService::new(self.store.clone(), self.workspaces(), Default::default())
    }

    fn workspaces(&self) -> AssignmentWorkspaces {
        AssignmentWorkspaces::new(
            self.store.clone(),
            self.notes.clone(),
            crate::downloads::Downloader::default(),
            crate::course_materials::DownloadSession::default(),
        )
    }

    fn seed_offering(&self) {
        self.store
            .catalog_upsert_offerings(&[json!({
                "id":"202612345", "year":2026, "quarters":["3Q"], "code":"ABC.101",
                "title":{"en":"Agent Systems"}, "instructors":[{"en":"Ada"}],
                "instructionLanguage":"en", "format":{"kinds":["lecture"]}, "media":{},
                "credits":{"lecture":2.0,"exercise":0.0,"lab":0.0},
                "schedule":[{"kind":"weekly","day":1,"periods":[1,2],"room":{"en":"W1"}}],
                "academicUnit":{"en":"Engineering"}, "intensive":false,
                "syllabusUrl":"https://example.invalid/202612345", "fetchedAt":{}
            })])
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn trace_keeps_validation_write_audit_and_read_shape_together() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let created = service
        .call(
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"Review","days":[1],"startTime":"09:00","endTime":"09:45"}))
        .unwrap();
    let id = created["id"].as_str().unwrap();
    assert!(created.get("rev").is_none());

    let updated = service
        .call(
            "weekly_event_update",
            json!({"id":id,"room":"W9","endTime":"10:00"}),
        )
        .unwrap();
    assert_eq!(updated, json!({"id":id}));
    let schedule = service
        .call("schedule", json!({"from":"2026-10-19","to":"2026-10-19"}))
        .unwrap();
    assert_eq!(schedule["events"][0]["room"], "W9");
    assert_eq!(schedule["events"][0]["endTime"], "10:00");
    assert!(schedule["events"][0].get("rev").is_none());

    let workspace = fixture.store.load_workspace().unwrap();
    let writes = workspace
        .pointer("/settings/agentAccess/lastWrites")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0]["tool"], "weekly_event_update");
    assert_eq!(writes[1]["tool"], "weekly_event_create");
}

#[test]
fn weekly_event_tools_meet_the_editor_rules_and_invalid_writes_do_not_commit() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let lab = service
        .call(
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"Lab","days":[2],"startTime":"13:30","endTime":"15:10"}))
        .unwrap();
    let before = fixture.store.user_rows("personal_events").unwrap();
    for (tool, args) in [
        (
            "weekly_event_create",
            json!({"term":"2026-5Q","title":"Q","days":[1],"startTime":"09:00","endTime":"10:00"}),
        ),
        (
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"Sunday+1","days":[8],"startTime":"09:00","endTime":"10:00"}),
        ),
        (
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"No days","startTime":"09:00","endTime":"10:00"}),
        ),
        (
            "weekly_event_update",
            json!({"id":lab["id"],"endTime":"12:00"}),
        ),
        (
            "weekly_event_update",
            json!({"id":"missing","title":"Ghost"}),
        ),
    ] {
        let error = service.call(tool, args.clone()).unwrap_err();
        assert!(matches!(error, AgentError::InvalidInput(_)), "{args}");
        assert_eq!(fixture.store.user_rows("personal_events").unwrap(), before);
    }

    // Writers that bypass the shared save meet the same contract.
    let error = fixture
        .store
        .agent_write("weekly_event_create", |write, _| {
            write.put(
                "personal_events",
                json!({"kind":"event","termId":"2026-3Q","title":"Raw","room":"","days":[],"startTime":"09:00","endTime":"10:00"}),
                None,
            )
        })
        .unwrap_err();
    assert!(matches!(
        error,
        AgentWriteError::Data(DataError::InvalidInput(_))
    ));

    service
        .call("weekly_event_delete", json!({"id":lab["id"]}))
        .unwrap();
    let error = service
        .call("weekly_event_delete", json!({"id":lab["id"]}))
        .unwrap_err();
    assert!(matches!(error, AgentError::InvalidInput(_)));
}

// Access off refuses reads and writes alike before anything is written;
// unknown tools and unknown arguments are refused by the contract itself.
#[test]
fn closed_access_and_unknown_calls_fail_before_mutation() {
    let fixture = Fixture::new();
    let service = fixture.service();
    assert!(matches!(
        service.call("submit_everything", json!({})),
        Err(AgentError::UnknownTool)
    ));
    let error = service
        .call(
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"Extra","days":[1],"startTime":"09:00","endTime":"09:30","colour":"red"}),
        )
        .unwrap_err();
    assert!(matches!(error, AgentError::InvalidInput(_)), "{error}");
    assert!(fixture
        .store
        .user_rows("personal_events")
        .unwrap()
        .is_empty());

    fixture
        .store
        .put_workspace_setting(
            "devicePreferences",
            json!({"agentAccess":{"enabled":false,"lastWrites":[]}}),
        )
        .unwrap();
    assert!(matches!(
        service.call("grades", json!({})),
        Err(AgentError::Disabled)
    ));
    assert!(matches!(
        service.call(
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"No","days":[1],"startTime":"09:00","endTime":"09:30"}),
        ),
        Err(AgentError::Disabled)
    ));
    assert!(fixture
        .store
        .user_rows("personal_events")
        .unwrap()
        .is_empty());
}

// What `tools/list` serves: every input and answer is a self-contained
// object schema (structured results must be objects), arguments outside the
// schema are refused, and the side effect decides the hints.
#[test]
fn every_tool_declares_object_schemas_and_its_effect() {
    for tool in tools() {
        for schema in [&tool.input_schema, &tool.output_schema] {
            assert_eq!(schema.get("type"), Some(&json!("object")), "{}", tool.name);
            assert!(!schema.contains_key("$defs"), "{}", tool.name);
        }
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&json!(false)),
            "{}",
            tool.name
        );
        if tool.effect == Effect::Read {
            assert!(tool.idempotent && !tool.destructive, "{}", tool.name);
        }
        // Whatever a client's approval mode, an LMS write asks the person.
        let asks = tool.meta.get("anthropic/requiresUserInteraction") == Some(&json!(true));
        assert_eq!(asks, tool.effect == Effect::LmsWrite, "{}", tool.name);
        if asks {
            assert!(
                tool.destructive && tool.open_world && !tool.read_only,
                "{}",
                tool.name
            );
        }
    }
    let lms_writes = tools()
        .iter()
        .filter(|tool| tool.effect == Effect::LmsWrite)
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(lms_writes, ["submission_save", "submission_remove"]);
    let names = tools().iter().map(|tool| tool.name).collect::<Vec<_>>();
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len());
}

// An LMS write tool needs its own switch on top of agent access, checked
// before its arguments are even read; the submission tools answer
// `unavailable` until the school transport is attached.
#[test]
fn lms_write_tools_need_their_own_switch() {
    use crate::error_code::{Coded, ErrorCode};
    let fixture = Fixture::new();
    let service = fixture.service();
    let refused = service
        .call("submission_save", json!({"preparationId": "p"}))
        .unwrap_err();
    assert_eq!(refused.code(), ErrorCode::LmsWritesDisabled);
    let detached = service
        .call("submission_status", json!({"operationId": "o"}))
        .unwrap_err();
    assert_eq!(detached.code(), ErrorCode::Unavailable);
}

#[test]
fn transaction_rechecks_revoked_access_before_running_the_write() {
    let fixture = Fixture::new();
    fixture
        .store
        .put_workspace_setting(
            "devicePreferences",
            json!({"agentAccess":{"enabled":false,"lastWrites":[]}}),
        )
        .unwrap();
    let ran = std::cell::Cell::new(false);
    let result = fixture.store.agent_write("weekly_event_create", |_, _| {
        ran.set(true);
        Ok(())
    });
    assert!(matches!(result, Err(AgentWriteError::Disabled)));
    assert!(!ran.get());
}

#[test]
fn rejected_audit_rolls_back_every_row_of_plan_add() {
    let fixture = Fixture::new();
    fixture.seed_offering();
    let connection = fixture.store.test_connection();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_mcp_audit BEFORE UPDATE ON settings
             WHEN NEW.updated_by='mcp' BEGIN SELECT RAISE(ABORT, 'audit rejected'); END;",
        )
        .unwrap();
    drop(connection);

    let error = fixture
        .service()
        .call(
            "plan_add",
            json!({"offeringId":"202612345","term":"2026-3Q"}),
        )
        .unwrap_err();
    assert!(matches!(error, AgentError::Storage(_)));
    assert!(fixture
        .store
        .user_rows("manual_courses")
        .unwrap()
        .is_empty());
    assert!(fixture.store.user_rows("plan_items").unwrap().is_empty());
}

#[test]
fn concurrent_plan_add_converges_on_one_course_and_plan_item() {
    let fixture = Fixture::new();
    fixture.seed_offering();
    let service = Arc::new(fixture.service());
    let barrier = Arc::new(Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let service = service.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            service.call(
                "plan_add",
                json!({"offeringId":"202612345","term":"2026-3Q"}),
            )
        }));
    }
    let results = threads
        .into_iter()
        .map(|thread| thread.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results[0]["planItemId"], results[1]["planItemId"]);
    assert_eq!(fixture.store.user_rows("manual_courses").unwrap().len(), 1);
    assert_eq!(fixture.store.user_rows("plan_items").unwrap().len(), 1);
}

#[test]
fn plan_add_uses_the_shared_first_year_gate_before_writing() {
    let fixture = Fixture::new();
    fixture
        .store
        .put_workspace_setting(
            "academicProfile",
            json!({
                "name":"Mine", "gradePath":[], "years":[],
                "studyEnrollments":[
                    {"stage":"undergraduate", "entryKind":"regular", "entryGrade":1,
                     "admittedTerm":"2026-1Q"},
                    {"stage":"master", "entryKind":"regular", "entryGrade":1,
                     "admittedTerm":"2026-4Q"}
                ]
            }),
        )
        .unwrap();
    fixture
        .store
        .catalog_upsert_offerings(&[json!({
            "id":"202612399", "year":2026, "quarters":["3Q","4Q"], "code":"LAW.W221-12",
            "title":{"en":"Upper Systems"}, "instructors":[],
            "instructionLanguage":"en", "format":{"kinds":["lecture"]}, "media":{},
            "credits":{"lecture":2.0,"exercise":0.0,"lab":0.0}, "schedule":[],
            "academicUnit":{"en":"Engineering"}, "intensive":false,
            "syllabusUrl":"https://example.invalid/202612399", "fetchedAt":{}
        })])
        .unwrap();

    let error = fixture
        .service()
        .call(
            "plan_add",
            json!({"offeringId":"202612399","term":"2026-4Q"}),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "LAW.W221-12 is not available to a regular first-year undergraduate student."
    );
    assert!(fixture
        .store
        .user_rows("manual_courses")
        .unwrap()
        .is_empty());
    assert!(fixture.store.user_rows("plan_items").unwrap().is_empty());
}

#[test]
fn schedule_and_courses_reject_reversed_ranges_and_unknown_quarters() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let error = service
        .call("schedule", json!({"from":"2026-10-20","to":"2026-10-19"}))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "date range must be ordered and no longer than 366 days"
    );
    let error = service
        .call("courses", json!({"year":2026,"quarter":"5Q"}))
        .unwrap_err();
    assert!(matches!(error, AgentError::InvalidInput(_)), "{error}");
}

#[test]
fn plan_add_places_a_multi_quarter_offering_in_each_of_its_quarters() {
    let fixture = Fixture::new();
    fixture
        .store
        .catalog_upsert_offerings(&[json!({
            "id":"202612346", "year":2026, "quarters":["3Q","4Q"], "code":"ABC.201",
            "title":{"en":"Long Systems"}, "instructors":[{"en":"Ada"}],
            "instructionLanguage":"en", "format":{"kinds":["lecture"]}, "media":{},
            "credits":{"lecture":2.0,"exercise":0.0,"lab":0.0},
            "schedule":[{"kind":"weekly","day":1,"periods":[1,2],"room":{"en":"W1"}}],
            "academicUnit":{"en":"Engineering"}, "intensive":false,
            "syllabusUrl":"https://example.invalid/202612346", "fetchedAt":{}
        })])
        .unwrap();
    let service = fixture.service();
    let added = service
        .call(
            "plan_add",
            json!({"offeringId":"202612346","term":"2026-4Q"}),
        )
        .unwrap();
    assert_eq!(added["terms"], json!(["2026-3Q", "2026-4Q"]));
    assert_eq!(added["planItemId"], "mcp-plan:2026-4Q:202612346");
    // One course, the same in each of its quarters.
    let ids: Vec<_> = ["3Q", "4Q"]
        .map(|quarter| {
            let courses = service
                .call("courses", json!({"year":2026,"quarter":quarter}))
                .unwrap();
            assert_eq!(
                courses["courses"][0]["offeringId"], "202612346",
                "{quarter}"
            );
            courses["courses"][0]["id"].clone()
        })
        .into();
    assert_eq!(ids[0], ids[1]);
    // Planned in another of its quarters is no reason to refuse.
    service
        .call(
            "plan_remove",
            json!({"offeringId":"202612346","term":"2026-3Q"}),
        )
        .unwrap();
    let again = service
        .call(
            "plan_add",
            json!({"offeringId":"202612346","term":"2026-3Q"}),
        )
        .unwrap();
    assert_eq!(again["courseId"], ids[0]);
    assert_eq!(fixture.store.user_rows("manual_courses").unwrap().len(), 1);
    let live = fixture
        .store
        .user_rows("plan_items")
        .unwrap()
        .into_iter()
        .filter(|row| row.get("deletedAt").is_none())
        .count();
    assert_eq!(live, 2);
}

#[test]
fn plan_add_does_not_revive_a_deleted_copy_of_a_live_course() {
    let fixture = Fixture::new();
    fixture.seed_offering();
    let service = fixture.service();
    service
        .call(
            "plan_add",
            json!({"offeringId":"202612345","term":"2026-3Q"}),
        )
        .unwrap();
    let course = fixture.store.user_rows("manual_courses").unwrap().remove(0);
    fixture
        .store
        .delete_user_record("manual_courses", course["id"].as_str().unwrap())
        .unwrap();
    // The school lists the same offering as a registered course.
    let mut school = course.clone();
    school["id"] = json!("2026-school-202612345");
    school["source"] = json!({"provider":"kyomu"});
    fixture
        .store
        .write(None, |transaction| {
            transaction
                .execute(
                    "INSERT INTO timetable_courses (id,value,hash,updated_at,updated_by,rev)
                     VALUES (?1,?2,'seed','2026-10-01T00:00:00Z','kyomu',1)",
                    rusqlite::params!["2026-school-202612345", school.to_string()],
                )
                .map_err(crate::data::DataError::from)
        })
        .unwrap();
    let added = service
        .call(
            "plan_add",
            json!({"offeringId":"202612345","term":"2026-3Q"}),
        )
        .unwrap();
    assert_eq!(added["courseId"], "2026-school-202612345");
    assert!(fixture.store.user_rows("manual_courses").unwrap()[0]
        .get("deletedAt")
        .is_some());
}

// A read tool loads only the fields its declared changes name. A field it
// uses but does not declare would read as missing and silently empty its
// answer, so each read tool must answer exactly as it does over every field.
#[test]
fn read_tools_answer_the_same_from_their_own_fields_as_from_every_field() {
    let fixture = Fixture::new();
    fixture.seed_offering();
    let service = fixture.service();
    service
        .call(
            "plan_add",
            json!({"offeringId":"202612345","term":"2026-3Q"}),
        )
        .unwrap();
    service
        .call(
            "weekly_event_create",
            json!({"term":"2026-3Q","title":"Review","days":[1],"startTime":"09:00","endTime":"09:45"}))
        .unwrap();
    let courses = service
        .call("courses", json!({"year":2026,"quarter":"3Q"}))
        .unwrap();
    let course = courses["courses"][0]["id"].as_str().unwrap().to_string();
    let every = || -> Result<View, AgentError> { Ok(fixture.store.load_reads()?) };
    for (name, args) in [
        ("today", json!({})),
        ("schedule", json!({"from":"2026-10-01","to":"2026-10-31"})),
        ("courses", json!({"year":2026,"quarter":"3Q"})),
        ("course", json!({"id": course})),
        ("recommended", json!({"departmentId":"math"})),
    ] {
        // `today` stamps the moment it answered; only that may differ.
        let tool = registry::find(name).unwrap();
        let answer = |view: &dyn Fn() -> Result<View, AgentError>| {
            let workspaces = fixture.workspaces();
            let cx = Context {
                store: &fixture.store,
                workspaces: &workspaces,
                submissions: None,
                view,
            };
            let mut value = tool.run(&cx, args.clone()).unwrap();
            if let Some(map) = value.as_object_mut() {
                map.remove("now");
            }
            value
        };
        assert_eq!(
            answer(&|| load_view(&fixture.store, tool.reads)),
            answer(&every),
            "{name}"
        );
    }
    assert!(!courses["courses"].as_array().unwrap().is_empty());
}

// An agent reads an assignment, has Core create its workspace once and sees
// what it wrote into `submit/`; Core's next operation follows a newly
// selected storage root.
#[test]
fn workspace_tools_create_once_report_submit_files_and_follow_the_root() {
    let fixture = Fixture::new();
    fixture.seed_offering();
    let service = fixture.service();
    let added = service
        .call(
            "plan_add",
            json!({"offeringId":"202612345","term":"2026-3Q"}),
        )
        .unwrap();
    let assignment = json!({
        "id":"lms-assign:1", "kind":"assignment", "courseId":added["courseId"],
        "title":"Report 1", "details":"<p>Write <b>two</b> pages.</p>",
        "dueAt":"2026-10-20T14:59:00Z", "source":{"provider":"lms"}
    });
    fixture
        .store
        .write(None, |transaction| {
            transaction
                .execute(
                    "INSERT INTO assignments (id,value,hash,updated_at,updated_by,rev)
                     VALUES (?1,?2,'seed','2026-10-01T00:00:00Z','lms',1)",
                    rusqlite::params!["lms-assign:1", assignment.to_string()],
                )
                .map_err(DataError::from)
        })
        .unwrap();
    // Downloads are async; the desktop runs tools on a blocking thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let call = |name: &'static str| {
        let service = service.clone();
        runtime
            .block_on(async move {
                tokio::task::spawn_blocking(move || {
                    service.call(name, json!({"assignmentId":"lms-assign:1"}))
                })
                .await
            })
            .unwrap()
            .unwrap()
    };
    let read = call("assignment");
    assert_eq!(read["notes"], "Write two pages.");
    assert!(read.get("workspace").is_none());

    let created = call("workspace_create");
    assert_eq!(created["missing"], json!([]));
    let path = PathBuf::from(created["path"].as_str().unwrap());
    assert!(path.starts_with(fixture.root.join("files").canonicalize().unwrap()));
    let brief = std::fs::read_to_string(path.join("ASSIGNMENT.md")).unwrap();
    assert!(brief.starts_with("# Report 1\n\n"), "{brief}");
    assert!(brief.contains("- Due: 2026-10-20 23:59 JST\n"), "{brief}");
    assert!(brief.contains("\n\nWrite two pages.\n"), "{brief}");
    assert_eq!(call("workspace_create")["path"], created["path"]);
    assert_eq!(call("assignment")["workspace"]["path"], created["path"]);

    std::fs::write(path.join("submit/report.pdf"), b"%PDF-1").unwrap();
    let status = call("workspace_status");
    assert_eq!(status["exists"], true);
    assert_eq!(status["files"][0]["name"], "report.pdf");
    assert_eq!(status["files"][0]["bytes"], 6);
    assert_eq!(
        status["files"][0]["sha256"],
        "21af8e71c8703196df7fe1ff901869a88fe64c07bbaa83d838efb45a52b4f303"
    );

    fixture
        .notes
        .select_root(fixture.root.join("elsewhere"), || Ok::<(), String>(()))
        .unwrap();
    assert_eq!(call("workspace_status")["exists"], false);
}
