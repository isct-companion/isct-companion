use adpt_isct::{
    kyomu::{parse_payload as parse_kyomu_payload, KyomuStage},
    lms::{
        aggregate_slots, decode_entities, validate_course, validate_deadlines, validate_enrolment,
        LmsDeadlines,
    },
};
use app_core::data;
use app_core::lms_ingest::{apply_lms_course, apply_lms_deadlines, apply_lms_enrolment};

use serde_json::{json, Value};

const ACCOUNT: &str = "science:24B00001";

#[test]
fn validated_lms_keys_project_to_workspace_course_material_and_assignment() {
    // Synthetic, non-personal input exercises only the deterministic projection
    // boundary. It is not presented as an authenticated LMS end-to-end capture.
    let enrolment = validate_enrolment(json!({
        "language": "en",
        "courses": [{
            "id": 42,
            "fullname": "ローカル演習 / Local Exercise",
            "shortname": "SYN.T001[2026]",
            "idnumber": "202602440",
            "coursecategory": "3",
            "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=42",
            "hidden": false
        }]
    }))
    .expect("validated enrolment");
    // Shapes as Moodle sends them: ids in the outline are numeric strings.
    let outline = validate_course(
        42,
        json!({
            "section": [{ "id": "7", "title": "Week 1", "number": 1 }],
            "cm": [
                {
                    "id": "1001",
                    "sectionid": "7",
                    "module": "assign",
                    "name": "Local assignment",
                    "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=1001",
                    "visible": true,
                    "uservisible": true
                },
                {
                    "id": "1002",
                    "sectionid": "7",
                    "module": "resource",
                    "name": "Local handout",
                    "url": "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=1002",
                    "visible": true,
                    "uservisible": true
                }
            ]
        }),
    )
    .expect("validated outline");
    let deadlines = validate_deadlines(LmsDeadlines {
        action_events: vec![json!({
            "instance": 1001, "modulename": "assign", "course": { "id": 42 }, "timesort": 1800000000,
            "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=1001"
        })],
        dated_events: vec![json!({
            "instance": 1001, "modulename": "assign", "course": { "id": 42 }, "eventtype": "due",
            "timesort": 1800000000
        })],
    })
    .expect("validated deadlines");

    let mut connection = data::migrations::open_in_memory().expect("both schemas");
    let owned_id = "2026-202602440";
    let owned = json!({
        "id": owned_id,
        "offeringId": "202602440",
        "source": {"provider": "lms", "externalId": "42"}
    });
    connection
        .execute(
            "INSERT INTO manual_courses
         (id,value,hash,updated_at,updated_by,rev,source,deleted_at)
         VALUES(?1,?2,'test','2026-10-06T00:00:00Z','test',1,'lms',NULL)",
            rusqlite::params![owned_id, owned.to_string()],
        )
        .unwrap();
    let transaction = connection.transaction().unwrap();
    let now = chrono::Utc::now();
    apply_lms_enrolment(&transaction, &enrolment, ACCOUNT, now).expect("enrolment key");
    apply_lms_course(&transaction, &outline, ACCOUNT, now).expect("course key");
    apply_lms_deadlines(&transaction, &deadlines, ACCOUNT, now).expect("deadlines key");
    transaction.commit().unwrap();

    let row = |table: &str, id: &str| -> Value {
        let raw: String = connection
            .query_row(
                &format!("SELECT value FROM {table} WHERE id=?1 AND deleted_at IS NULL"),
                [id],
                |record| record.get(0),
            )
            .unwrap_or_else(|error| panic!("missing {table}/{id}: {error}"));
        serde_json::from_str(&raw).unwrap()
    };
    // The timetable changes only through the app's Pull review.
    let timetable: i64 = connection
        .query_row("SELECT COUNT(*) FROM timetable_courses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(timetable, 0);
    let memberships: i64 = connection
        .query_row("SELECT COUNT(*) FROM memberships", [], |r| r.get(0))
        .unwrap();
    assert_eq!(memberships, 0);
    let course = json!({"id": owned_id});
    let lms_course = row("lms_courses", "lms-course:42");
    assert_eq!(lms_course["offeringId"], "202602440");
    // The LMS category name is the quarter.
    assert_eq!(lms_course["quarters"], json!(["3Q"]));

    let material = row("lms_items", "lms-item:1002");
    assert_eq!(material["kind"], "material");
    assert_eq!(material["courseId"], course["id"]);
    assert_eq!(material["title"], "Local handout");
    assert_eq!(material["session"], 1);

    let assignment = row("assignments", "lms-assignment:1001");
    assert_eq!(assignment["kind"], "assignment");
    assert_eq!(assignment["courseId"], course["id"]);
    assert_eq!(assignment["submission"], "open");
    assert!(assignment["dueAt"].as_str().is_some());
    assert!(assignment.get("hidden").is_none());
    assert!(assignment.get("completedAt").is_none());
}

#[test]
fn kyomu_login_redirects_request_reconnection_without_records() {
    for source_url in [
        "https://kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx",
        "https://portal.titech.ac.jp/",
        "https://rp.nap.gsic.titech.ac.jp/",
    ] {
        let login = json!({
            "status": "login",
            "sourceUrl": source_url,
            "kind": "unknown",
            "data": {}
        });
        let (stage, payload) =
            parse_kyomu_payload(&login.to_string()).expect("known school login redirect");
        assert_eq!(stage, KyomuStage::ReconnectRequired);
        assert!(payload.is_none());
    }
}

#[test]
fn kyomu_requires_the_exact_read_only_page_for_each_payload_kind() {
    let mismatched = json!({
        "status": "ok",
        "sourceUrl": "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
        "kind": "registrations",
        "data": { "courses": [] }
    });
    assert!(parse_kyomu_payload(&mismatched.to_string()).is_err());

    let unapproved = json!({
        "status": "login",
        "sourceUrl": "https://example.invalid/",
        "kind": "unknown",
        "data": {}
    });
    assert!(parse_kyomu_payload(&unapproved.to_string()).is_err());
}

#[test]
fn dashboard_grid_cells_aggregate_into_contiguous_meetings() {
    let meetings = aggregate_slots(&[
        (7, 1, 3),
        (7, 1, 4),
        (7, 4, 4),
        (7, 4, 3),
        (7, 2, 1),
        (7, 2, 5),
        (9, 3, 1),
    ]);
    assert_eq!(
        meetings[&7],
        vec![
            json!({"day": 1, "periods": [3, 4]}),
            json!({"day": 2, "periods": [1, 1]}),
            json!({"day": 2, "periods": [5, 5]}),
            json!({"day": 4, "periods": [3, 4]}),
        ]
    );
    assert_eq!(meetings[&9], vec![json!({"day": 3, "periods": [1, 1]})]);
}

#[test]
fn real_moodle_shapes_keep_odd_course_records_and_aggregate_everything() {
    // One odd record (no name, a non-LMS link, a repeated id) must not keep
    // the whole course list from committing.
    let enrolment = validate_enrolment(json!({
        "language": "ja",
        "courses": [
            {"id": 8438, "fullname": "アセンブリ言語", "shortname": "CSC.T262[2026]", "idnumber": "202602427",
             "coursecategory": "3", "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=8438", "hidden": false},
            {"id": 3986, "fullname": "", "shortname": "SYN.T100[2025]", "idnumber": "202500001",
             "coursecategory": "3", "viewurl": "javascript:alert(1)", "hidden": false},
            {"id": 8438, "fullname": "Duplicate", "shortname": "DUP[2026]"}
        ],
        "timetables": [{"year": 2026, "quarter": "3",
            "slots": [{"courseId": 8438, "day": 2, "period": 3}, {"courseId": 8438, "day": 2, "period": 4},
                      {"courseId": 8438, "day": 5, "period": 3}, {"courseId": 8438, "day": 5, "period": 4},
                      {"courseId": 99999, "day": 1, "period": 1}],
            "unscheduled": []}]
    }))
    .expect("tolerant enrolment");
    assert_eq!(enrolment.courses.len(), 2);
    let odd = &enrolment.courses[1];
    assert_eq!(odd.fullname, "SYN.T100[2025]");
    assert_eq!(
        odd.viewurl,
        "https://lms.s.isct.ac.jp/2025/course/view.php?id=3986"
    );
    // A record without an id cannot be placed and fails the key.
    assert!(validate_enrolment(json!({"courses": [{"id": "broken", "fullname": "x"}]})).is_err());

    let deadlines = validate_deadlines(LmsDeadlines {
        action_events: vec![json!({
            "instance": 49871, "modulename": "assign", "activityname": "Old report", "timesort": 1761529500,
            "course": {"id": 3986}, "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=49871"
        })],
        dated_events: vec![],
    })
    .unwrap();

    let mut connection = data::migrations::open_in_memory().expect("both schemas");
    let transaction = connection.transaction().unwrap();
    let now = chrono::Utc::now();
    apply_lms_enrolment(&transaction, &enrolment, ACCOUNT, now).expect("enrolment");
    apply_lms_deadlines(&transaction, &deadlines, ACCOUNT, now).expect("deadlines");
    transaction.commit().unwrap();
    let value = |table: &str, id: &str| -> Value {
        let raw: String = connection
            .query_row(
                &format!("SELECT value FROM {table} WHERE id=?1"),
                [id],
                |r| r.get(0),
            )
            .unwrap_or_else(|error| panic!("missing {table}/{id}: {error}"));
        serde_json::from_str(&raw).unwrap()
    };
    let course = value("lms_courses", "lms-course:8438");
    assert_eq!(course["quarters"], json!(["3Q"]));
    assert_eq!(
        course["timetables"]["2026-3Q"],
        json!([{"day": 2, "periods": [3, 4]}, {"day": 5, "periods": [3, 4]}])
    );
    // Open work from a course whose outline was not read still arrives.
    let old = value("assignments", "lms-assignment:49871");
    assert_eq!(old["title"], "Old report");
    assert_eq!(old["submission"], "open");
    assert_eq!(old["lmsCourseId"], 3986);
}

#[test]
fn moodle_names_are_unescaped_once() {
    assert_eq!(
        decode_entities("Archaeology &amp; Biological Anthropology C"),
        "Archaeology & Biological Anthropology C"
    );
    assert_eq!(decode_entities("&lt;b&gt; &#39;x&#39; &#x41;"), "<b> 'x' A");
    // Not an entity, and double escaping is undone only one level.
    assert_eq!(decode_entities("R&D & more &amp;amp;"), "R&D & more &amp;");
}
