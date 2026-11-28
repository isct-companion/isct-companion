//! The capture test reads the course list a real account's token read gets,
//! saved into the directory in `MOODLE_WS_CAPTURES` as
//! `crates/app-core/src/tests/net__moodle__mapping.rs` describes
//! (`core_course_get_enrolled_courses_by_timeline_classification.json`), and
//! is compiled only when that variable names a directory
//! (`moodle_ws_captures`, `build.rs`); a missing file fails the test.
//!
//! `MOODLE_WS_CAPTURES=/path/to/dir cargo test -p isct-companion --test school_ingest_boundaries`
use adpt_isct::lms::{aggregate_slots, decode_entities};
#[cfg(moodle_ws_captures)]
use adpt_isct::lms::{validate_enrolment, LmsCourse, LmsEnrolment};
#[cfg(moodle_ws_captures)]
use app_core::{data, lms_ingest::apply_lms_enrolment};
use serde_json::json;
#[cfg(moodle_ws_captures)]
use serde_json::Value;

#[cfg(moodle_ws_captures)]
fn course(enrolment: &LmsEnrolment, id: u64) -> &LmsCourse {
    enrolment
        .courses
        .iter()
        .find(|course| course.id == id)
        .unwrap_or_else(|| panic!("course {id} was not read"))
}

/// One odd record beside a real course list (a course without a name and
/// with a link off the LMS, an id listed twice) must not keep the whole list
/// from committing: the course takes its short name and its own LMS page, the
/// first record of a repeated id stands, and every course commits. Only a
/// record without an id fails the key. The commit leaves the timetable to
/// the app's Pull review.
#[test]
#[cfg(moodle_ws_captures)]
fn odd_records_beside_a_captured_course_list_still_commit() {
    let path = std::path::Path::new(env!("MOODLE_WS_CAPTURES"))
        .join("core_course_get_enrolled_courses_by_timeline_classification.json");
    let raw = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let timeline: Value =
        serde_json::from_slice(&raw).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let courses = timeline["courses"].as_array().unwrap().clone();
    assert!(
        courses.len() >= 2,
        "the captured account is enrolled in two courses"
    );
    let ids: Vec<u64> = courses
        .iter()
        .map(|course| course["id"].as_u64().unwrap())
        .collect();
    let enrolment =
        |courses: Vec<Value>| validate_enrolment(json!({"language": "ja", "courses": courses}));
    let plain = enrolment(courses.clone()).expect("the captured list");

    let mut odd = courses.clone();
    odd[1]["fullname"] = json!("");
    odd[1]["viewurl"] = json!("javascript:alert(1)");
    let mut repeated = courses[0].clone();
    repeated["fullname"] = courses[1]["fullname"].clone();
    odd.push(repeated);
    let read = enrolment(odd).expect("odd records do not fail the key");
    assert_eq!(read.courses.len(), plain.courses.len());
    assert_eq!(
        course(&read, ids[0]).fullname,
        course(&plain, ids[0]).fullname
    );
    let renamed = course(&read, ids[1]);
    assert_eq!(renamed.fullname, decode_entities(&renamed.shortname));
    // An unsafe link falls back to the course's own LMS page, whatever path
    // the capture's year used.
    assert!(
        renamed.viewurl.starts_with("https://lms.s.isct.ac.jp/")
            && renamed
                .viewurl
                .ends_with(&format!("/course/view.php?id={}", ids[1])),
        "{}",
        renamed.viewurl
    );

    let mut without_id = courses.clone();
    without_id[0].as_object_mut().unwrap().remove("id");
    assert!(enrolment(without_id).is_err());

    let mut connection = data::migrations::open_in_memory().expect("both schemas");
    let transaction = connection.transaction().unwrap();
    apply_lms_enrolment(&transaction, &read, app_core::clock::now()).expect("enrolment");
    transaction.commit().unwrap();
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(count("lms_courses"), read.courses.len() as i64);
    assert_eq!(count("timetable_courses"), 0);
    assert_eq!(count("memberships"), 0);
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
fn moodle_names_are_unescaped_once() {
    assert_eq!(
        decode_entities("Archaeology &amp; Biological Anthropology C"),
        "Archaeology & Biological Anthropology C"
    );
    assert_eq!(decode_entities("&lt;b&gt; &#39;x&#39; &#x41;"), "<b> 'x' A");
    // Not an entity, and double escaping is undone only one level.
    assert_eq!(decode_entities("R&D & more &amp;amp;"), "R&D & more &amp;");
}
