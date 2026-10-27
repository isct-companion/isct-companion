use super::*;
use crate::data::{commit_on, migrations};
use adpt_isct::lms::{validate_assign, validate_course, validate_deadlines, validate_enrolment};
use chrono::TimeZone;
use serde_json::json;

const ACCOUNT: &str = "science:student";
const ASSIGN_URL: &str = "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=7";

fn database() -> Connection {
    migrations::open_in_memory().unwrap()
}

fn enrol(connection: &mut Connection) {
    let enrolment = validate_enrolment(json!({"language": "en", "courses": [{
        "id": 42, "fullname": "Test", "shortname": "TEST[2026]", "coursecategory": "3",
        "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=42"}], "timetables": []}))
    .unwrap();
    commit_on(connection, |tx| {
        apply_lms_enrolment(tx, &enrolment, ACCOUNT, Utc::now())
    })
    .unwrap();
}

fn outline(cms: Value, sections: Value) -> LmsCourseContent {
    validate_course(42, json!({"section": sections, "cm": cms})).unwrap()
}

fn row(connection: &Connection, table: &str, id: &str) -> Option<Value> {
    connection
        .query_row(
            &format!("SELECT value FROM {table} WHERE id=?1"),
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .unwrap()
        .map(|raw| serde_json::from_str(&raw).unwrap())
}

fn missing(connection: &Connection, table: &str, id: &str) -> bool {
    connection
        .query_row(
            &format!("SELECT missing_since IS NOT NULL FROM {table} WHERE id=?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn a_course_commits_whole_and_marks_what_its_outline_dropped_missing() {
    let mut connection = database();
    enrol(&mut connection);
    let first = outline(
        json!([
            {"id": "100", "sectionid": "9", "module": "resource", "name": "File",
             "url": "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=100",
             "visible": true, "uservisible": true, "bodyHtml": "<p>File description</p>",
             "modified": 1_700_000_000},
            {"id": "7", "sectionid": "9", "module": "assign", "name": "Essay", "url": ASSIGN_URL,
             "visible": true, "uservisible": true}
        ]),
        json!([{"id": "9", "number": 9, "title": "Week", "visible": true, "bodyHtml": "<p>Text only</p>"},
               {"id": "8", "number": 8, "title": "Week", "visible": true}]),
    );
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &first, ACCOUNT, Utc::now())
    })
    .unwrap();
    // The next read fetches details only for what changed.
    let stored = stored_modules(&connection, 42).unwrap();
    assert_eq!(
        stored[&100],
        StoredModule {
            modified: Some(1_700_000_000),
            has_body: true
        }
    );
    assert!(!stored[&7].has_body);

    // An outline-only refresh: no bodies, the assignment removed, a page added.
    let second = outline(
        json!([
            {"id": "100", "sectionid": "9", "module": "resource", "name": "File",
             "url": "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=100",
             "visible": true, "uservisible": true, "modified": 1_700_000_000},
            {"id": "102", "sectionid": "8", "module": "page", "name": "New",
             "url": "https://lms.s.isct.ac.jp/2025/mod/page/view.php?id=102",
             "visible": true, "uservisible": true}
        ]),
        json!([{"id": "9", "number": 9, "title": "Week", "visible": true},
               {"id": "8", "number": 8, "title": "Week", "visible": true}]),
    );
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &second, ACCOUNT, Utc::now())
    })
    .unwrap();
    let course = row(&connection, "lms_courses", "lms-course:42").unwrap();
    assert_eq!(course["sections"].as_array().unwrap().len(), 2);
    assert_eq!(course["sections"][0]["id"], 9);
    assert_eq!(course["sections"][0]["bodyHtml"], "<p>Text only</p>");
    assert_eq!(course["title"], json!({"en": "Test"}));
    let item = row(&connection, "lms_items", "lms-item:100").unwrap();
    assert_eq!(item["bodyHtml"], "<p>File description</p>");
    assert_eq!(item["sectionNumber"], 9);
    let page = row(&connection, "lms_items", "lms-item:102").unwrap();
    assert_eq!(page["sectionNumber"], 8);
    assert_eq!(page["courseId"], course_link(&connection));
    assert!(missing(&connection, "lms_items", "lms-item:7"));
    assert!(missing(&connection, "assignments", "lms-assignment:7"));
    assert!(!missing(&connection, "lms_items", "lms-item:100"));
    let state = sync_state::get(&connection, &Key::new(Dataset::LmsCourse, "42"))
        .unwrap()
        .unwrap();
    assert_eq!(state.account, ACCOUNT);
    assert!(state.fetched_at.is_some());

    // The same outline again finds nothing new: no record row or course
    // identity is written, only the key's freshness, so no view hears of a
    // change and cloud sync captures nothing.
    let (_, tables) = commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &second, ACCOUNT, Utc::now())
    })
    .unwrap();
    assert_eq!(
        crate::workspace_changes::changes_for(&tables),
        [crate::workspace_changes::WorkspaceChange::Freshness].into()
    );
    assert!(!crate::workspace_changes::touches_user_data(&tables));
    assert_eq!(row(&connection, "lms_items", "lms-item:100"), Some(item));
    assert_eq!(
        row(&connection, "lms_courses", "lms-course:42"),
        Some(course)
    );
    // A deleted row the read lists again with the same content comes back.
    connection
        .execute(
            "UPDATE lms_items SET deleted_at='2026-10-01T00:00:00Z' WHERE id='lms-item:102'",
            [],
        )
        .unwrap();
    let (_, tables) = commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &second, ACCOUNT, Utc::now())
    })
    .unwrap();
    assert!(tables.contains("lms_items"));
    let deleted: bool = connection
        .query_row(
            "SELECT deleted_at IS NOT NULL FROM lms_items WHERE id='lms-item:102'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!deleted);

    // The enrolment owns the course list: a course it no longer lists is
    // missing; its stored sections are left to `lms.course`.
    let empty = validate_enrolment(json!({"courses": []})).unwrap();
    commit_on(&mut connection, |tx| {
        apply_lms_enrolment(tx, &empty, ACCOUNT, Utc::now())
    })
    .unwrap();
    assert!(missing(&connection, "lms_courses", "lms-course:42"));
}

fn course_link(connection: &Connection) -> Value {
    connection
        .query_row(
            "SELECT course_id FROM course_keys WHERE key='lms:42'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .into()
}

#[test]
fn a_failed_course_key_writes_nothing_and_keeps_its_fetched_time() {
    let content = outline(
        json!([{"id": 7, "sectionid": 1, "module": "assign", "name": "Essay", "url": ASSIGN_URL}]),
        json!([]),
    );
    let mut missing_schema = Connection::open_in_memory().unwrap();
    assert!(matches!(
        commit_on(&mut missing_schema, |tx| apply_lms_course(
            tx,
            &content,
            ACCOUNT,
            Utc::now()
        )),
        Err(IngestError::Storage(_))
    ));

    let mut connection = database();
    enrol(&mut connection);
    let key = Key::new(Dataset::LmsCourse, "42");
    let fetched_at = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &content, ACCOUNT, fetched_at)
    })
    .unwrap();
    // A module whose link leaves the LMS rejects the key after another
    // module was already written: the transaction drops both.
    let bad = LmsCourseContent {
        course_id: 42,
        sections: Vec::new(),
        modules: vec![
            json!({"id": 8, "sectionid": 1, "module": "page", "name": "New",
                   "url": "https://lms.s.isct.ac.jp/2025/mod/page/view.php?id=8"}),
            json!({"id": 9, "sectionid": 1, "module": "resource", "name": "bad",
                   "url": "https://example.invalid/file"}),
        ],
    };
    assert!(matches!(
        commit_on(&mut connection, |tx| apply_lms_course(
            tx,
            &bad,
            ACCOUNT,
            Utc::now()
        )),
        Err(IngestError::InvalidPayload(_))
    ));
    commit_on(&mut connection, |tx| {
        sync_state::fail(tx, &key, ACCOUNT, "parse", Utc::now())
    })
    .unwrap();
    assert!(row(&connection, "lms_items", "lms-item:8").is_none());
    assert!(!missing(&connection, "lms_items", "lms-item:7"));
    assert!(!missing(&connection, "assignments", "lms-assignment:7"));
    let state = sync_state::get(&connection, &key).unwrap().unwrap();
    assert_eq!(state.fetched_at, Some(fetched_at));
    assert_eq!(state.failures, 1);

    // A course the enrolment does not list is not written either; it waits
    // for the list rather than failing as a bad payload.
    let other = LmsCourseContent {
        course_id: 43,
        ..content.clone()
    };
    assert!(matches!(
        commit_on(&mut connection, |tx| apply_lms_course(
            tx,
            &other,
            ACCOUNT,
            Utc::now()
        )),
        Err(IngestError::NotEnrolled(43))
    ));
}

#[test]
fn each_key_keeps_the_assignment_fields_the_others_own() {
    let mut connection = database();
    enrol(&mut connection);
    let due = Utc::now().timestamp() + 86_400;
    let deadlines = |listed: bool| {
        let events = if listed {
            vec![
                json!({"instance": 7, "modulename": "assign", "activityname": "Essay",
                "timesort": due, "url": ASSIGN_URL, "course": {"id": 42}}),
            ]
        } else {
            Vec::new()
        };
        validate_deadlines(LmsDeadlines {
            action_events: events,
            dated_events: Vec::new(),
        })
        .unwrap()
    };
    let assignment =
        |connection: &Connection| row(connection, "assignments", "lms-assignment:7").unwrap();

    // Open work of a course whose outline was not read yet gets its row.
    commit_on(&mut connection, |tx| {
        apply_lms_deadlines(tx, &deadlines(true), ACCOUNT, Utc::now())
    })
    .unwrap();
    let first = assignment(&connection);
    assert_eq!(first["submission"], "open");
    assert_eq!(first["title"], "Essay");
    let due_at = first["dueAt"].clone();
    assert!(due_at.is_string());

    // The outline owns title and link; due date and state stay.
    let course = outline(
        json!([{"id": 7, "sectionid": 1, "module": "assign", "name": "Essay (final)", "url": ASSIGN_URL}]),
        json!([]),
    );
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &course, ACCOUNT, Utc::now())
    })
    .unwrap();
    let renamed = assignment(&connection);
    assert_eq!(renamed["title"], "Essay (final)");
    assert_eq!(renamed["submission"], "open");
    assert_eq!(renamed["dueAt"], due_at);

    // The assignment page owns submission, grade and acceptsSubmission; once
    // it is stored, neither the open-work list nor the outline undoes them.
    let graded = validate_assign(
        42,
        vec![json!({"cmid": 7, "submission": "graded", "grade": " 85.00 / 100.00 ", "acceptsSubmission": false})],
    )
    .unwrap();
    commit_on(&mut connection, |tx| {
        apply_lms_assign(tx, &graded, ACCOUNT, Utc::now())
    })
    .unwrap();
    commit_on(&mut connection, |tx| {
        apply_lms_deadlines(tx, &deadlines(true), ACCOUNT, Utc::now())
    })
    .unwrap();
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &course, ACCOUNT, Utc::now())
    })
    .unwrap();
    let kept = assignment(&connection);
    assert_eq!(kept["submission"], "graded");
    assert_eq!(kept["grade"], "85.00 / 100.00");
    assert_eq!(kept["acceptsSubmission"], false);
    assert_eq!(kept["title"], "Essay (final)");
    assert_eq!(kept["dueAt"], due_at);

    // A new page state replaces all three.
    let reopened = validate_assign(
        42,
        vec![json!({"cmid": 7, "submission": "open", "acceptsSubmission": true})],
    )
    .unwrap();
    commit_on(&mut connection, |tx| {
        apply_lms_assign(tx, &reopened, ACCOUNT, Utc::now())
    })
    .unwrap();
    let open = assignment(&connection);
    assert_eq!(open["submission"], "open");
    assert!(open.get("grade").is_none());
    assert_eq!(open["acceptsSubmission"], true);

    // Open work that left the list before its deadline was handed in.
    commit_on(&mut connection, |tx| {
        apply_lms_deadlines(tx, &deadlines(false), ACCOUNT, Utc::now())
    })
    .unwrap();
    assert_eq!(assignment(&connection)["submission"], "submitted");
    assert_eq!(assignment(&connection)["dueAt"], due_at);

    // The stored states cover the known rows, so a repeat outline keeps them
    // current; a new assignment has none yet and makes them missing, to be
    // read next. (States read before the row existed would have been lost.)
    let states = Key::new(Dataset::LmsAssign, "42");
    let fetched = |connection: &Connection| {
        sync_state::get(connection, &states)
            .unwrap()
            .and_then(|state| state.fetched_at)
    };
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &course, ACCOUNT, Utc::now())
    })
    .unwrap();
    assert!(fetched(&connection).is_some());
    let grown = outline(
        json!([
            {"id": 7, "sectionid": 1, "module": "assign", "name": "Essay (final)", "url": ASSIGN_URL},
            {"id": 8, "sectionid": 1, "module": "quiz", "name": "Quiz 1",
             "url": "https://lms.s.isct.ac.jp/2025/mod/quiz/view.php?id=8"}
        ]),
        json!([]),
    );
    commit_on(&mut connection, |tx| {
        apply_lms_course(tx, &grown, ACCOUNT, Utc::now())
    })
    .unwrap();
    assert!(fetched(&connection).is_none());
}

#[test]
fn course_terms_follow_the_grid_and_the_month_in_japan() {
    let now = Utc.with_ymd_and_hms(2026, 10, 20, 3, 0, 0).unwrap();
    let course = |id: u64, year: i32, quarter: u8, shown: &[&str]| StoredLmsCourse {
        id,
        title: String::new(),
        year: Some(year),
        quarter: Some(quarter),
        shown_in: shown.iter().map(|term| term.to_string()).collect(),
    };
    // 3Q now; course 2 started in 2Q but the current grid lists it.
    let courses = [
        course(1, 2026, 3, &[]),
        course(2, 2026, 2, &["2026-3Q"]),
        course(3, 2026, 1, &[]),
        course(4, 2025, 3, &[]),
    ];
    let current = current_lms_term(now);
    assert_eq!(current, (2026, 3));
    let terms: Vec<Term> = courses
        .iter()
        .map(|course| lms_course_term(course, current))
        .collect();
    assert_eq!(
        terms,
        [Term::Current, Term::Current, Term::Past, Term::Frozen]
    );
}
