use super::{apply_lms_payload, IngestError, LMS_EXTRACT_SCRIPT};
use crate::data::migrations;
use crate::school_parser::lms::{LmsCourse, LmsPayload, LmsStructure};
use rusqlite::Connection;
use serde_json::json;
use std::collections::HashMap;

fn invalid_item_payload() -> LmsPayload {
    LmsPayload {
        status: "ok".into(),
        source_url: "https://lms.s.isct.ac.jp/2025/my/".into(),
        language: "en".into(),
        courses_complete: false,
        full_scan: false,
        courses: vec![LmsCourse {
            id: 42,
            fullname: "Test".into(),
            shortname: "TEST[2026]".into(),
            idnumber: String::new(),
            coursecategory: 1,
            startdate: 0,
            enddate: 0,
            viewurl: "https://lms.s.isct.ac.jp/2025/course/view.php?id=42".into(),
            hidden: false,
        }],
        structures: vec![LmsStructure {
            course_id: 42,
            complete: true,
            state: Some(
                json!({"section":[],"cm":[{"id":7,"sectionid":1,"module":"resource","name":"bad","url":"https://example.invalid/file"}]}),
            ),
            error: None,
        }],
        action_events: Vec::new(),
        dated_events: Vec::new(),
        assign_status: HashMap::new(),
        timetable: None,
        timetables: Vec::new(),
        timetable_errors: Vec::new(),
        skipped: 0,
    }
}

#[test]
fn extractor_routes_work_by_loaded_page_and_explicit_scope() {
    assert!(LMS_EXTRACT_SCRIPT
        .contains("const fullScan = globalThis.__ISCT_COMPANION_LMS_FULL__ === true"));
    assert!(LMS_EXTRACT_SCRIPT
        .contains("const requestedCourseId = explicitCourseId || (!fullScan ? pageCourseId : 0)"));
    assert!(LMS_EXTRACT_SCRIPT.contains("'core_calendar_get_action_events_by_course'"));
    assert!(LMS_EXTRACT_SCRIPT
        .contains("if (requestedCourseId) return Number(course.id) === requestedCourseId;"));
    assert!(LMS_EXTRACT_SCRIPT
        .contains("return fullScan || (currentScan && currentIds.has(Number(course.id)));"));
}

#[test]
fn ingest_error_distinguishes_payload_rejection_from_storage_failure() {
    let payload = invalid_item_payload();
    let mut missing_schema = Connection::open_in_memory().unwrap();
    assert!(matches!(
        apply_lms_payload(&mut missing_schema, &payload),
        Err(IngestError::Storage(_))
    ));

    let mut connection = Connection::open_in_memory().unwrap();
    migrations::migrate(&mut connection).unwrap();
    assert!(matches!(
        apply_lms_payload(&mut connection, &payload),
        Err(IngestError::InvalidPayload(_))
    ));
}
#[test]
fn page_state_survives_missing_page_then_accepts_reopened_state() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrations::migrate(&mut connection).unwrap();
    let mut raw = json!({
        "status":"ok", "sourceUrl":"https://lms.s.isct.ac.jp/2025/my/",
        "courses":[{"id":42,"fullname":"Test","shortname":"TEST[2026]","coursecategory":1,"viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=42"}],
        "structures":[{"courseId":42,"complete":true,"state":{
            "section":[], "cm":[{"id":7,"sectionid":1,"module":"assign","name":"Essay",
            "url":"https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=7"}]
        }}],
        "assignStatus":[{"cmid":7,"submission":"graded","grade":" 85.00 / 100.00 ","acceptsSubmission":false}]
    });
    for stage in 0..3 {
        if stage == 1 {
            raw["assignStatus"] = json!([]);
        }
        if stage == 2 {
            raw["assignStatus"] = json!([{"cmid":7,"submission":"open","acceptsSubmission":true}]);
        }
        let (_, payload) = crate::school_parser::lms::parse_payload(&raw.to_string()).unwrap();
        apply_lms_payload(&mut connection, &payload.unwrap()).unwrap();
        let value: String = connection
            .query_row(
                "SELECT value FROM assignments WHERE lms_cmid=7",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&value).unwrap();
        if stage < 2 {
            assert_eq!(value["submission"], "graded");
            assert_eq!(value["grade"], "85.00 / 100.00");
            assert_eq!(value["acceptsSubmission"], false);
        } else {
            assert_eq!(value["submission"], "open");
            assert!(value.get("grade").is_none());
            assert_eq!(value["acceptsSubmission"], true);
        }
    }
}

#[test]
fn page_captures_do_not_make_the_full_refresh_look_fresh() {
    let path = std::env::temp_dir().join(format!("isct-lms-scope-{}.db", uuid::Uuid::new_v4()));
    let store = crate::data::DataStore::initialize(path).unwrap().0;
    let mut connection = Connection::open(store.path()).unwrap();
    let raw = json!({"status":"ok","sourceUrl":"https://lms.s.isct.ac.jp/2025/my/","coursesComplete":true,"courses":[]});
    let (_, payload) = crate::school_parser::lms::parse_payload(&raw.to_string()).unwrap();
    let mut payload = payload.unwrap();
    apply_lms_payload(&mut connection, &payload).unwrap();
    let runs = store.last_import_runs().unwrap();
    assert!(runs["lms:summary"].is_string());
    assert!(runs["lms:all"].is_null());
    payload.full_scan = true;
    apply_lms_payload(&mut connection, &payload).unwrap();
    let full = store.last_import_runs().unwrap()["lms:all"].clone();
    assert!(full.is_string());
    payload.full_scan = false;
    apply_lms_payload(&mut connection, &payload).unwrap();
    assert_eq!(store.last_import_runs().unwrap()["lms:all"], full);
    payload.full_scan = true;
    payload.courses_complete = false;
    apply_lms_payload(&mut connection, &payload).unwrap();
    assert_eq!(store.last_import_runs().unwrap()["lms:all"], full);
}
