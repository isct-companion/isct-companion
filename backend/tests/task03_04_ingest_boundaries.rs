// These acceptance tests include the production modules directly until the
// core crate exposes them; each included module contains APIs this test binary
// intentionally does not exercise.
#[allow(dead_code)]
#[path = "../src/contracts.rs"]
mod contracts;
#[allow(dead_code)]
#[path = "../src/data/mod.rs"]
mod data;
#[allow(dead_code)]
#[path = "../src/kyomu_ingest.rs"]
mod kyomu_ingest;
#[allow(dead_code)]
#[path = "../src/lms_ingest.rs"]
mod lms_ingest;
#[allow(dead_code)]
mod school_parser {
    pub(crate) mod kyomu {
        include!("../src/school_parser/kyomu.rs");
    }
    pub(crate) mod lms {
        include!("../src/school_parser/lms.rs");
    }
}

use kyomu_ingest::{declared_credits, gpa_summaries};
use lms_ingest::apply_lms_payload;
use rusqlite::Connection;
use school_parser::{
    kyomu::{parse_payload as parse_kyomu_payload, KyomuStage},
    lms::{aggregate_slots, decode_entities, parse_payload as parse_lms_payload, LmsStage},
};
use serde_json::{json, Value};

#[test]
fn lms_preserves_independent_sections_and_cached_bodies_on_an_outline_only_refresh() {
    let mut connection = Connection::open_in_memory().unwrap();
    data::migrations::migrate(&mut connection).unwrap();
    let base = json!({
        "status":"ok", "sourceUrl":"https://lms.s.isct.ac.jp/2025/my/", "language":"en", "coursesComplete":true,
        "courses":[{"id":42,"fullname":"Test","shortname":"TEST[2026]","coursecategory":"3","viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=42"}],
        "structures":[{"courseId":42,"complete":true,"state":{
            "section":[{"id":"9","number":9,"title":"Repeated","visible":true,"bodyHtml":"<p>Text only</p>"},{"id":"8","number":8,"title":"Repeated","visible":true}],
            "cm":[{"id":"100","sectionid":"9","module":"resource","name":"File","url":"https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=100","visible":true,"uservisible":true,"bodyHtml":"<p>File description</p>","resourceUrl":"https://lms.s.isct.ac.jp/2025/pluginfile.php/42/mod_resource/content/3/file.pdf"}]
        }}]
    });
    let (_, first) = parse_lms_payload(&base.to_string()).unwrap();
    apply_lms_payload(&mut connection, &first.unwrap()).unwrap();
    let mut refresh = base.clone();
    refresh["structures"][0]["state"]["section"][0]
        .as_object_mut()
        .unwrap()
        .remove("bodyHtml");
    let cm = refresh["structures"][0]["state"]["cm"][0]
        .as_object_mut()
        .unwrap();
    cm.remove("bodyHtml");
    cm.remove("resourceUrl");
    let (_, second) = parse_lms_payload(&refresh.to_string()).unwrap();
    apply_lms_payload(&mut connection, &second.unwrap()).unwrap();
    let course: String = connection
        .query_row(
            "SELECT value FROM lms_courses WHERE id='lms-course:42'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let course: Value = serde_json::from_str(&course).unwrap();
    assert_eq!(course["sections"].as_array().unwrap().len(), 2);
    assert_eq!(course["sections"][0]["number"], 9);
    assert_eq!(course["sections"][0]["id"], 9);
    assert_eq!(course["sections"][1]["number"], 8);
    assert_eq!(course["sections"][0]["bodyHtml"], "<p>Text only</p>");
    let item: String = connection
        .query_row(
            "SELECT value FROM lms_items WHERE id='lms-item:100'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let item: Value = serde_json::from_str(&item).unwrap();
    assert_eq!(item["bodyHtml"], "<p>File description</p>");
    assert!(item["resourceUrl"]
        .as_str()
        .unwrap()
        .contains("/content/3/"));
}

#[test]
fn lms_login_and_incomplete_results_never_become_importable_data() {
    let login = json!({
        "status": "login",
        "sourceUrl": "https://lms.s.isct.ac.jp/2025/login/index.php"
    });
    let (stage, payload) = parse_lms_payload(&login.to_string()).expect("known LMS login");
    assert_eq!(stage, LmsStage::ReconnectRequired);
    assert!(payload.is_none());

    let incomplete = json!({
        "status": "error",
        "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
        "coursesComplete": false,
        "courses": []
    });
    assert!(parse_lms_payload(&incomplete.to_string()).is_err());
}

#[test]
fn lms_rejects_payloads_claiming_an_unapproved_origin() {
    let payload = json!({
        "status": "login",
        "sourceUrl": "https://example.invalid/2025/login/index.php"
    });
    assert!(parse_lms_payload(&payload.to_string()).is_err());
}

#[test]
fn validated_lms_payload_projects_to_workspace_course_material_assignment_and_membership() {
    // Synthetic, non-personal input exercises only the deterministic projection
    // boundary. It is not presented as an authenticated LMS end-to-end capture.
    let raw = json!({
        "status": "ok",
        "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
        "language": "en",
        "coursesComplete": true,
        "courses": [{
            "id": 42,
            "fullname": "ローカル演習 / Local Exercise",
            "shortname": "SYN.T001[2026]",
            "idnumber": "202602440",
            "coursecategory": "3",
            "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=42",
            "hidden": false
        }],
        "structures": [{
            "courseId": 42,
            "complete": true,
            "state": {
                // Shapes as Moodle sends them: ids in get_state are numeric strings.
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
            }
        }],
        "actionEvents": [{
            "instance": 1001,
            "course": { "id": 42 },
            "timesort": 1800000000
        }],
        "datedEvents": [{
            "instance": 1001,
            "course": { "id": 42 },
            "eventtype": "due",
            "timesort": 1800000000
        }]
    });
    let (stage, payload) = parse_lms_payload(&raw.to_string()).expect("validated projection input");
    assert_eq!(stage, LmsStage::Ready);

    let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
    data::migrations::migrate(&mut connection).expect("schema v3");
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
    apply_lms_payload(&mut connection, &payload.unwrap()).expect("atomic LMS projection");

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
fn gpa_summary_maps_the_totals_table_by_column_labels() {
    let data = json!({"overall": [[
        ["全ての科目", "教養科目群", "専門科目群"],
        ["GPA", "GPT", "修得済単位", "合格率%", "GPA", "修得済単位", "合格率%", "GPA", "修得済単位", "合格率%"],
        ["3.12", "312.5", "64.0", "98.0", "3.40", "30.0", "100.0", "2.90", "34.0", "96.0"]
    ]]});
    let rows = gpa_summaries(&data);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["scope"], "all");
    assert_eq!(rows[0]["gpa"], 3.12);
    assert_eq!(rows[0]["gpt"], 312.5);
    assert_eq!(rows[0]["earnedCredits"], 64.0);
    assert_eq!(rows[1]["scope"], "liberal-arts");
    assert_eq!(rows[1]["earnedCredits"], 30.0);
    assert_eq!(rows[2]["scope"], "major");
    assert!(rows[2].get("gpt").is_none());
    assert!(gpa_summaries(&json!({"overall": []})).is_empty());
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

fn moodle_payload() -> Value {
    json!({
        "status": "ok",
        "sourceUrl": "https://lms.s.isct.ac.jp/2025/my/",
        "language": "ja",
        "coursesComplete": true,
        "courses": [
            {"id": 8438, "fullname": "アセンブリ言語", "shortname": "CSC.T262[2026]", "idnumber": "202602427",
             "coursecategory": "3", "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=8438", "hidden": false},
            {"id": 3986, "fullname": "過去の授業", "shortname": "SYN.T100[2025]", "idnumber": "202500001",
             "coursecategory": "3", "viewurl": "https://lms.s.isct.ac.jp/2025/course/view.php?id=3986", "hidden": false},
            {"id": "broken", "fullname": "", "viewurl": "javascript:alert(1)"}
        ],
        "structures": [{"courseId": 8438, "complete": true, "state": {
            "section": [{"id": "70", "number": 1, "title": "Week 1"}],
            "cm": [
                {"id": "135414", "sectionid": "70", "module": "assign", "name": "Report 1",
                 "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=135414", "visible": true, "uservisible": true},
                {"id": "bad", "module": "url"}
            ]}}],
        "actionEvents": [
            {"instance": 135414, "modulename": "assign", "timesort": 1795000000, "course": {"id": 8438},
             "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=135414"},
            {"instance": 49871, "modulename": "assign", "activityname": "Old report", "timesort": 1761529500,
             "course": {"id": 3986}, "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=49871"}
        ],
        "datedEvents": [],
        "timetable": {"year": 2026, "quarter": "3",
            "slots": [{"courseId": 8438, "day": 2, "period": 3}, {"courseId": 8438, "day": 2, "period": 4},
                      {"courseId": 8438, "day": 5, "period": 3}, {"courseId": 8438, "day": 5, "period": 4},
                      {"courseId": 99999, "day": 1, "period": 1}],
            "unscheduled": []},
        "assignStatus": [{"cmid": 135414, "submission": "submitted"}, {"cmid": 49871, "submission": "bogus"}]
    })
}

#[test]
fn real_moodle_shapes_keep_good_records_and_aggregate_everything() {
    let (stage, payload) =
        parse_lms_payload(&moodle_payload().to_string()).expect("tolerant parse");
    assert_eq!(stage, LmsStage::Ready);
    let payload = payload.unwrap();
    assert_eq!(payload.courses.len(), 2);
    // A dropped course means the list is not known to be complete.
    assert!(!payload.courses_complete);
    assert_eq!(payload.skipped, 2);
    // The structure lost a module, so its items are not tombstoned.
    assert!(!payload.structures[0].complete);

    let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
    data::migrations::migrate(&mut connection).expect("schema v3");
    apply_lms_payload(&mut connection, &payload).expect("apply");
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
    // The assignment page wins over the open-work list.
    let current = value("assignments", "lms-assignment:135414");
    assert_eq!(current["submission"], "submitted");
    assert!(
        current["dueAt"].as_str().is_some(),
        "due from the open-work event"
    );
    // Open work from a course whose structure was not loaded still arrives.
    let old = value("assignments", "lms-assignment:49871");
    assert_eq!(old["title"], "Old report");
    // An unknown page state is ignored; the open-work list still says open.
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

#[test]
fn declared_credits_follow_each_category_total() {
    let data = json!({"courses": [
        {"offeringId": "202600702", "unit": "kamokuUnit_110700", "unitTotal": 1.0, "quarters": [3],
         "credits": {"lecture": 0, "exercise": 0, "lab": 1}},
        {"offeringId": "202600722", "unit": "kamokuUnit_110700", "unitTotal": 1.0, "quarters": [4],
         "credits": {"lecture": 0, "exercise": 0, "lab": 1}},
        {"offeringId": "202602431", "unit": "kamokuUnit_120900", "unitTotal": 2.0, "quarters": [3],
         "credits": {"lecture": 1, "exercise": 1, "lab": 0}},
        {"offeringId": "202637648", "unit": "kamokuUnit_110610", "quarters": [3, 4],
         "credits": {"lecture": 0, "exercise": 2, "lab": 0}}
    ]});
    let rows = declared_credits(&data);
    let credit = |id: &str| {
        rows.iter().find(|r| r["offeringId"] == id).unwrap()["effectiveCredits"]
            .as_f64()
            .unwrap()
    };
    // Two 0-0-1 PE rows in a category that totals 1.0 count half each.
    assert_eq!(credit("202600702"), 0.5);
    assert_eq!(credit("202600722"), 0.5);
    assert_eq!(credit("202602431"), 2.0);
    // Without a stated total the row's own split stands.
    assert_eq!(credit("202637648"), 2.0);
    assert_eq!(rows[0]["id"], "kyomu-declared:202600702");
}
