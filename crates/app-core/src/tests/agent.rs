use super::*;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

struct Fixture {
    root: PathBuf,
    store: DataStore,
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
        Self { root, store }
    }

    fn service(&self) -> AgentService {
        AgentService::new(self.store.clone())
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

const WRITES: AgentPolicy = AgentPolicy { allow_writes: true };

#[test]
fn trace_keeps_validation_write_audit_and_read_shape_together() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let created = service
        .call(
            "weekly_event_create",
            &json!({"term":"2026-3Q","title":"Review","days":[1],"startTime":"09:00","endTime":"09:45"}),
            WRITES,
        )
        .unwrap();
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["ok"], true);
    assert!(created.get("rev").is_none());

    let updated = service
        .call(
            "weekly_event_update",
            &json!({"id":id,"room":"W9","endTime":"10:00"}),
            WRITES,
        )
        .unwrap();
    assert_eq!(updated, json!({"ok":true,"id":id}));
    let schedule = service
        .call(
            "schedule",
            &json!({"from":"2026-10-19","to":"2026-10-19"}),
            WRITES,
        )
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
            &json!({"term":"2026-3Q","title":"Lab","days":[2],"startTime":"13:30","endTime":"15:10"}),
            WRITES,
        )
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
        let error = service.call(tool, &args, WRITES).unwrap_err();
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
        .call("weekly_event_delete", &json!({"id":lab["id"]}), WRITES)
        .unwrap();
    let error = service
        .call("weekly_event_delete", &json!({"id":lab["id"]}), WRITES)
        .unwrap_err();
    assert!(matches!(error, AgentError::InvalidInput(_)));
}

#[test]
fn authorization_and_write_policy_fail_before_mutation() {
    let fixture = Fixture::new();
    let service = fixture.service();
    let denied = service
        .call(
            "weekly_event_create",
            &json!({"term":"2026-3Q","title":"No","days":[1],"startTime":"09:00","endTime":"09:30"}),
            AgentPolicy {
                allow_writes: false,
            },
        )
        .unwrap_err();
    assert!(matches!(denied, AgentError::WritesDisabled));
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
        service.call("grades", &json!({}), WRITES),
        Err(AgentError::Disabled)
    ));
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
            &json!({"offeringId":"202612345","term":"2026-3Q"}),
            WRITES,
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
                &json!({"offeringId":"202612345","term":"2026-3Q"}),
                WRITES,
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
            &json!({"offeringId":"202612399","term":"2026-4Q"}),
            WRITES,
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
        .call(
            "schedule",
            &json!({"from":"2026-10-20","to":"2026-10-19"}),
            WRITES,
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "date range must be ordered and no longer than 366 days"
    );
    assert_eq!(
        service
            .call("courses", &json!({"year":2026,"quarter":"5Q"}), WRITES)
            .unwrap_err()
            .to_string(),
        "term must look like 2026-3Q, got 2026-5Q"
    );
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
            &json!({"offeringId":"202612346","term":"2026-4Q"}),
            WRITES,
        )
        .unwrap();
    assert_eq!(added["terms"], json!(["2026-3Q", "2026-4Q"]));
    assert_eq!(added["planItemId"], "mcp-plan:2026-4Q:202612346");
    // One course, the same in each of its quarters.
    let ids: Vec<_> = ["3Q", "4Q"]
        .map(|quarter| {
            let courses = service
                .call("courses", &json!({"year":2026,"quarter":quarter}), WRITES)
                .unwrap();
            assert_eq!(courses[0]["offeringId"], "202612346", "{quarter}");
            courses[0]["id"].clone()
        })
        .into();
    assert_eq!(ids[0], ids[1]);
    // Planned in another of its quarters is no reason to refuse.
    service
        .call(
            "plan_remove",
            &json!({"offeringId":"202612346","term":"2026-3Q"}),
            WRITES,
        )
        .unwrap();
    let again = service
        .call(
            "plan_add",
            &json!({"offeringId":"202612346","term":"2026-3Q"}),
            WRITES,
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
            &json!({"offeringId":"202612345","term":"2026-3Q"}),
            WRITES,
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
            &json!({"offeringId":"202612345","term":"2026-3Q"}),
            WRITES,
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
            &json!({"offeringId":"202612345","term":"2026-3Q"}),
            WRITES,
        )
        .unwrap();
    service
        .call(
            "weekly_event_create",
            &json!({"term":"2026-3Q","title":"Review","days":[1],"startTime":"09:00","endTime":"09:45"}),
            WRITES,
        )
        .unwrap();
    let courses = service
        .call("courses", &json!({"year":2026,"quarter":"3Q"}), WRITES)
        .unwrap();
    let course = courses[0]["id"].as_str().unwrap().to_string();
    let every = || -> Result<View, AgentError> { Ok(fixture.store.load_reads()?) };
    for (name, args) in [
        ("today", json!({})),
        ("schedule", json!({"from":"2026-10-01","to":"2026-10-31"})),
        ("courses", json!({"year":2026,"quarter":"3Q"})),
        ("course", json!({"id": course})),
        ("recommended", json!({"departmentId":"math"})),
    ] {
        // `today` stamps the moment it answered; only that may differ.
        let answer = |view: &dyn Fn() -> Result<View, AgentError>| {
            let mut value = call_inner(&fixture.store, name, &args, view).unwrap();
            if let Some(map) = value.as_object_mut() {
                map.remove("now");
            }
            value
        };
        assert_eq!(
            answer(&|| tool_view(&fixture.store, name)),
            answer(&every),
            "{name}"
        );
    }
    assert!(!courses.as_array().unwrap().is_empty());
}
