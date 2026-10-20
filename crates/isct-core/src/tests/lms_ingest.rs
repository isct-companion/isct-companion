use super::{apply_lms_payload, IngestError};
use crate::data::{commit_on, migrations};
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
fn ingest_error_distinguishes_payload_rejection_from_storage_failure() {
    let payload = invalid_item_payload();
    let mut missing_schema = Connection::open_in_memory().unwrap();
    assert!(matches!(
        commit_on(&mut missing_schema, |tx| apply_lms_payload(tx, &payload))
            .map(|(changed, _)| changed),
        Err(IngestError::Storage(_))
    ));

    let mut connection = Connection::open_in_memory().unwrap();
    migrations::migrate(&mut connection).unwrap();
    assert!(matches!(
        commit_on(&mut connection, |tx| apply_lms_payload(tx, &payload))
            .map(|(changed, _)| changed),
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
        commit_on(&mut connection, |tx| {
            apply_lms_payload(tx, &payload.unwrap())
        })
        .map(|(changed, _)| changed)
        .unwrap();
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
