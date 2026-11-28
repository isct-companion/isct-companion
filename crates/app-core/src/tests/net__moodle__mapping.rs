//! Moodle's answers carry an account's courses and grades, so none is
//! committed. The capture tests read REST answers saved from one account into
//! the directory in `MOODLE_WS_CAPTURES`, kept outside the repository. Take
//! the "Moodle mobile web service" key from the LMS's Preferences → Security
//! keys (never paste it anywhere else), and save each answer of
//! `curl -s https://lms.s.isct.ac.jp/2025/webservice/rest/server.php
//! -d moodlewsrestformat=json -d wstoken=<key> -d wsfunction=<function> <args>`
//! as `<function>.json`:
//!
//! - `core_block_get_dashboard_blocks`: `-d returncontents=1`;
//! - `core_course_get_enrolled_courses_by_timeline_classification`:
//!   `-d classification=all -d limit=0 -d offset=0 -d sort=fullname`;
//! - `core_course_get_contents`: `-d courseid=<an enrolled course with files>`;
//! - `core_calendar_get_calendar_monthly_view`: `-d year=<y> -d month=<m>
//!   -d courseid=1 -d categoryid=0 -d day=1`, a month with an assignment due;
//! - `mod_assign_get_submission_status`: `-d assignid=<an assignment's id>`;
//! - `mod_forum_get_forum_discussions`: `-d forumid=<a course's news forum>
//!   -d sortorder=-1 -d page=0 -d perpage=5`.
//!
//! `MOODLE_COURSE_ID` is the `courseid` the course contents were saved for.
//! The tests are compiled only when `MOODLE_WS_CAPTURES` names a directory
//! (`moodle_ws_captures`, `build.rs`), the commit test also only when
//! `MOODLE_COURSE_ID` is set (`moodle_course_id`); an answer missing from the
//! directory fails its test.
//!
//! `MOODLE_WS_CAPTURES=/path/to/dir MOODLE_COURSE_ID=<id> cargo test -p app-core --lib net::moodle::mapping`
use super::*;
#[cfg(moodle_ws_captures)]
use crate::net::moodle::envelope::has_warnings;
#[cfg(moodle_ws_captures)]
use serde_json::json;

const WS_FILE: &str =
    "https://lms.s.isct.ac.jp/2025/webservice/pluginfile.php/9/mod_resource/content/2/notes.pdf";
const FILE: &str =
    "https://lms.s.isct.ac.jp/2025/pluginfile.php/9/mod_resource/content/2/notes.pdf";

#[test]
fn file_links_are_stored_without_web_service_path_or_credential() {
    assert_eq!(
        plain_file_url(&format!("{WS_FILE}?forcedownload=1&token=abc")).as_deref(),
        Some(format!("{FILE}?forcedownload=1").as_str())
    );
    assert_eq!(
        plain_file_url(&format!("{WS_FILE}?token=abc")).as_deref(),
        Some(FILE)
    );
    // Never another origin, however it looks.
    assert_eq!(
        plain_file_url("https://evil.lms.s.isct.ac.jp/2025/webservice/pluginfile.php/1/x"),
        None
    );
    assert_eq!(
        plain_html(&format!(r#"<img src="{WS_FILE}">"#)),
        format!(r#"<img src="{FILE}">"#)
    );
}

#[test]
fn history_months_cover_the_enrolled_academic_years_up_to_today() {
    let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
    assert_eq!(
        months_to_read(&[2026, 2025, 2026], today),
        [
            (2026, 4),
            (2026, 5),
            (2026, 6),
            (2025, 4),
            (2025, 5),
            (2025, 6),
            (2025, 7),
            (2025, 8),
            (2025, 9),
            (2025, 10),
            (2025, 11),
            (2025, 12),
            (2026, 1),
            (2026, 2),
            (2026, 3),
        ]
    );
    let years: Vec<i32> = (2000..2010).collect();
    assert_eq!(months_to_read(&years, today).len(), 48);
}

/// One body is cut short and a whole course read stops taking bodies, so
/// later ones keep their cached copy.
#[test]
fn body_budget_bounds_each_body_and_the_whole_course_read() {
    let budget = BodyBudget::new();
    let long = "あ".repeat(150_000);
    assert!(budget.take(&long).unwrap().chars().count() < 150_000);
    assert!((0..100).any(|_| budget.take(&long).is_none()));
}

#[cfg(moodle_ws_captures)]
fn capture(function: &str) -> Value {
    let path = std::path::Path::new(env!("MOODLE_WS_CAPTURES")).join(format!("{function}.json"));
    let raw = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// One account's real answers mapped to the shapes the trust boundaries
/// take, as a token read does. The dashboard block feeds the school's grid
/// parser and places only enrolled courses. The course outline names each
/// module once and keeps no web-service file path or credential. A month
/// lists only assignment and quiz deadlines, each keyed by its activity. An
/// assignment status reads as one of the page's states, and announcements
/// become articles with no credential.
#[test]
#[cfg(moodle_ws_captures)]
fn captured_rest_answers_map_to_what_the_trust_boundaries_take() {
    let blocks = capture("core_block_get_dashboard_blocks");
    assert!(!has_warnings(&blocks));
    let grid = dashboard_timetable(&blocks).expect("the dashboard shows the timetable block");
    let timeline = capture("core_course_get_enrolled_courses_by_timeline_classification");
    let courses: Vec<Value> = timeline["courses"]
        .as_array()
        .unwrap()
        .iter()
        .map(course_row)
        .collect();
    let enrolment = adpt_isct::lms::validate_enrolment(
        json!({"language": "ja", "courses": courses, "timetables": [grid]}),
    )
    .unwrap();
    let enrolled: HashSet<u64> = enrolment.courses.iter().map(|course| course.id).collect();
    let timetable = &enrolment.timetables[0];
    assert!(!timetable.term.is_empty());
    assert!(timetable
        .meetings
        .keys()
        .chain(&timetable.unscheduled)
        .all(|id| enrolled.contains(id)));

    let contents = capture("core_course_get_contents");
    let (state, modules) = structure_from_contents(&contents).unwrap();
    let mut cmids = HashSet::new();
    assert!(modules.iter().all(|(cmid, _)| cmids.insert(*cmid)));
    let stored = state.to_string();
    assert!(!stored.contains("/webservice/pluginfile.php/") && !stored.contains("token="));
    for (_, module) in &modules {
        for file in &module.files {
            assert_eq!(
                plain_file_url(&file.url).as_deref(),
                Some(file.url.as_str())
            );
        }
    }
    adpt_isct::lms::validate_course(1, state).unwrap();

    let deadlines = month_deadlines(&capture("core_calendar_get_calendar_monthly_view"));
    assert!(!deadlines.is_empty(), "the captured month has a deadline");
    assert!(deadlines
        .iter()
        .all(|event| matches!(event["modulename"].as_str(), Some("assign" | "quiz"))));
    // None is dropped at the boundary as a site or personal event.
    let month = adpt_isct::lms::validate_history("2026-01", deadlines.clone()).unwrap();
    assert_eq!(month.events.len(), deadlines.len());

    let assign = assign_state(7, &capture("mod_assign_get_submission_status"))
        .expect("the status names the student's attempt");
    assert!(matches!(
        assign["submission"].as_str(),
        Some("open" | "submitted" | "graded")
    ));

    let discussions = capture("mod_forum_get_forum_discussions");
    assert!(!has_warnings(&discussions));
    let posts = discussions["discussions"].as_array().unwrap();
    assert!(!posts.is_empty(), "the captured forum has an announcement");
    for post in posts {
        let article = forum_article(post).unwrap();
        assert!(!article.contains("/webservice/pluginfile.php/") && !article.contains("token="));
    }
}

#[cfg(all(moodle_ws_captures, moodle_course_id))]
fn missing(connection: &rusqlite::Connection, table: &str, id: &str) -> bool {
    connection
        .query_row(
            &format!("SELECT missing_since IS NOT NULL FROM {table} WHERE id=?1"),
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

/// One captured course from its REST answers to stored rows, as a token read
/// commits it: every module of the outline is stored; the same outline read
/// again writes nothing but the key's freshness, so no view hears of a change
/// and cloud sync captures nothing; a module the outline no longer lists, and
/// a course the enrolment no longer lists, are marked missing; and a course
/// outside the enrolment is not written.
#[test]
#[cfg(all(moodle_ws_captures, moodle_course_id))]
fn a_captured_course_commits_once_and_marks_what_its_outline_dropped() {
    use crate::data::{commit_on, migrations};
    use crate::lms_ingest::{apply_lms_course, apply_lms_enrolment, stored_modules, IngestError};
    use crate::workspace_changes::{changes_for, touches_user_data, WorkspaceChange};
    let course_id: u64 = env!("MOODLE_COURSE_ID")
        .parse()
        .expect("MOODLE_COURSE_ID is a course id");
    let timeline = capture("core_course_get_enrolled_courses_by_timeline_classification");
    let courses: Vec<Value> = timeline["courses"]
        .as_array()
        .unwrap()
        .iter()
        .map(course_row)
        .collect();
    let enrolment = |courses: Vec<Value>| {
        adpt_isct::lms::validate_enrolment(json!({"language": "ja", "courses": courses})).unwrap()
    };
    let (state, _) = structure_from_contents(&capture("core_course_get_contents")).unwrap();
    let content = adpt_isct::lms::validate_course(course_id, state).unwrap();
    assert!(
        content.modules.len() >= 2,
        "the captured course lists modules"
    );

    let mut connection = migrations::open_in_memory().unwrap();
    let now = crate::clock::now();
    commit_on(&mut connection, |tx| {
        apply_lms_enrolment(tx, &enrolment(courses.clone()), now)
    })
    .unwrap();
    commit_on(&mut connection, |tx| apply_lms_course(tx, &content, now)).unwrap();
    assert_eq!(
        stored_modules(&connection, course_id).unwrap().len(),
        content.modules.len()
    );

    let (_, tables) = commit_on(&mut connection, |tx| apply_lms_course(tx, &content, now)).unwrap();
    assert_eq!(changes_for(&tables), [WorkspaceChange::Freshness].into());
    assert!(!touches_user_data(&tables));

    let mut dropped = content.clone();
    let gone = dropped.modules.pop().unwrap();
    let gone = format!("lms-item:{}", id_of(gone.get("id")).unwrap());
    let kept = format!("lms-item:{}", id_of(dropped.modules[0].get("id")).unwrap());
    commit_on(&mut connection, |tx| apply_lms_course(tx, &dropped, now)).unwrap();
    assert!(missing(&connection, "lms_items", &gone));
    assert!(!missing(&connection, "lms_items", &kept));

    let other = adpt_isct::lms::LmsCourseContent {
        course_id: courses
            .iter()
            .filter_map(|course| id_of(course.get("id")))
            .max()
            .unwrap()
            + 1,
        ..content.clone()
    };
    assert!(matches!(
        commit_on(&mut connection, |tx| apply_lms_course(tx, &other, now)),
        Err(IngestError::NotEnrolled(_))
    ));

    let without: Vec<Value> = courses
        .into_iter()
        .filter(|course| id_of(course.get("id")) != Some(course_id))
        .collect();
    commit_on(&mut connection, |tx| {
        apply_lms_enrolment(tx, &enrolment(without), now)
    })
    .unwrap();
    assert!(missing(
        &connection,
        "lms_courses",
        &format!("lms-course:{course_id}")
    ));
}
