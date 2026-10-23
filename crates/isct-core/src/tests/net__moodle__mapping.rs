use super::*;
use serde_json::json;

const WS_FILE: &str =
    "https://lms.s.isct.ac.jp/2025/webservice/pluginfile.php/9/mod_resource/content/2/notes.pdf";
const FILE: &str =
    "https://lms.s.isct.ac.jp/2025/pluginfile.php/9/mod_resource/content/2/notes.pdf";

// A generic block response must feed the existing timetable trust boundary,
// rather than silently committing an empty grid when a block is missing.
#[test]
fn dashboard_block_uses_the_school_grid_parser_and_rejects_missing_content() {
    let html = concat!(
        r#"<select name="year"><option value="7" selected>2026</option></select>"#,
        r#"<select name="quota"><option selected>3</option></select>"#,
        "<table><tr><th>Period</th><th>Mon</th><th>Tue</th><th>Wed</th><th>Thu</th><th>Fri</th></tr>",
        r#"<tr><th>1</th><td><a href="/2025/course/view.php?id=42">Course</a></td><td></td><td></td><td></td><td></td></tr></table>"#,
        r#"<table><tr><td><a href="/2025/course/view.php?id=99">Unscheduled</a></td></tr></table>"#,
    );
    let response = json!({"blocks":[
        {"name":"html","contents":{"content":html}},
        {"name":"tttimetable","visible":true,"contents":{"content":html}}
    ]});
    let grid = dashboard_timetable(&response).unwrap();
    assert_eq!(grid["slots"], json!([{"courseId":42,"day":1,"period":1}]));
    let enrolled = crate::school_parser::lms::validate_enrolment(json!({
        "courses":[{"id":42},{"id":99}], "timetables":[grid]
    }))
    .unwrap();
    assert_eq!(enrolled.timetables[0].term, "2026-3Q");
    assert_eq!(enrolled.timetables[0].unscheduled, [99]);
    assert_eq!(
        enrolled.timetables[0].meetings[&42],
        [json!({"day":1,"periods":[1,1]})]
    );
    for invalid in [
        json!({"blocks":[]}),
        json!({"blocks":[response["blocks"][0].clone()]}),
        json!({"blocks":[{"name":"tttimetable"}]}),
        json!({"blocks":[{"name":"tttimetable","contents":{"content":null}}]}),
        json!({"blocks":[{"name":"tttimetable","contents":{"content":"<table></table>"}}]}),
        json!({"blocks":[{"name":"tttimetable","visible":false,"contents":{"content":html}}]}),
    ] {
        assert!(dashboard_timetable(&invalid).is_none(), "{invalid}");
    }
}

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
fn course_contents_become_the_stored_outline() {
    let contents = json!([
        {"id": 11, "section": 0, "name": "General", "visible": 1, "uservisible": true,
         "summary": format!(r#"<p>Read <a href="{WS_FILE}">notes</a></p>"#),
         "modules": [
            {"id": 101, "instance": 7, "modname": "resource", "name": "Notes &amp; slides",
             "url": "https://lms.s.isct.ac.jp/2025/mod/resource/view.php?id=101",
             "visible": 1, "uservisible": true,
             "contents": [{"type": "file", "filename": "notes.pdf", "filepath": "/",
                           "fileurl": format!("{WS_FILE}?forcedownload=1"), "timemodified": 1_700_000_000},
                          {"type": "file", "filename": "old.pdf", "filepath": "/",
                           "fileurl": format!("{WS_FILE}?forcedownload=1"), "timemodified": 1_600_000_000}]},
            {"id": 102, "instance": 8, "modname": "url", "name": "Reading",
             "url": "https://lms.s.isct.ac.jp/2025/mod/url/view.php?id=102",
             "visible": 1, "uservisible": true,
             "contents": [{"type": "url", "filename": "Reading", "fileurl": "https://example.edu/reading"}]},
            {"id": 103, "instance": 9, "modname": "label", "name": "Note",
             "visible": 0, "uservisible": false, "description": "<p>Exam moved</p>"}
         ]},
        {"id": 12, "section": 1, "name": "Week 1", "visible": 0, "summary": "  ", "modules": []}
    ]);
    let (state, modules) = structure_from_contents(&contents).unwrap();
    let sections = state["section"].as_array().unwrap();
    assert_eq!(sections[0]["anchor"], "section-0");
    assert_eq!(
        sections[0]["bodyHtml"],
        format!(r#"<p>Read <a href="{FILE}">notes</a></p>"#)
    );
    // A blank summary leaves any cached one in place; hidden is not visible.
    assert!(sections[1].get("bodyHtml").is_none());
    assert_eq!(sections[1]["visible"], false);

    let cms = state["cm"].as_array().unwrap();
    assert_eq!(cms.len(), 3);
    assert_eq!(cms[0]["sectionid"], 11);
    assert_eq!(cms[0]["module"], "resource");
    assert_eq!(cms[0]["resourceUrl"], format!("{FILE}?forcedownload=1"));
    assert_eq!(cms[1]["resourceUrl"], "https://example.edu/reading");
    // A label has no link; its on-page text is its body.
    assert!(cms[2].get("url").is_none());
    assert_eq!(cms[2]["bodyHtml"], "<p>Exam moved</p>");
    assert_eq!(cms[2]["visible"], false);
    assert_eq!(cms[2]["uservisible"], false);

    // The newest file time is the module's content stamp; a module without
    // contents has none, so its details are never taken as unchanged.
    assert_eq!(cms[0]["modified"], 1_700_000_000);
    assert!(cms[2].get("modified").is_none());

    let (_, resource) = modules.iter().find(|(cmid, _)| *cmid == 101).unwrap();
    assert_eq!(resource.modified, 1_700_000_000);
    assert_eq!(resource.files[0].name, "notes.pdf");
    assert_eq!(resource.files[0].url, format!("{FILE}?forcedownload=1"));
    assert!(structure_from_contents(&json!({"exception": "x"})).is_none());
}

#[test]
fn events_are_keyed_by_the_cmid_their_activity_link_names() {
    let event = json!({"id": 5, "instance": 900, "modulename": "assign", "name": "Essay is due",
        "activityname": "Essay", "timesort": 1_800_000_000, "eventtype": "due", "secret": "x",
        "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=77",
        "course": {"id": 42, "fullname": "Course"}});
    let normalized = normalize_event(&event, &ACTION_EVENT_KEYS);
    assert_eq!(normalized["instance"], 77);
    assert_eq!(normalized["course"], json!({"id": 42}));
    assert!(normalized.get("secret").is_none());
    // Without an activity link the exporter's value stands.
    let course_event =
        json!({"id": 6, "instance": 3, "timestart": 1_800_000_100, "course": {"id": 42}});
    let normalized = normalize_event(&course_event, &UPCOMING_EVENT_KEYS);
    assert_eq!(normalized["instance"], 3);
    assert_eq!(normalized["timesort"], 1_800_000_100);

    let month = json!({"weeks": [{"days": [{"events": [
        {"id": 1, "modulename": "assign", "eventtype": "due", "timestart": 10, "course": {"id": 1},
         "url": "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=8"},
        {"id": 2, "modulename": "quiz", "eventtype": "open", "timestart": 11, "course": {"id": 1}},
        {"id": 3, "modulename": "forum", "eventtype": "due", "timestart": 12, "course": {"id": 1}},
        {"id": 4, "modulename": "quiz", "eventtype": "close", "timesort": 13, "course": {"id": 1}}
    ]}]}]});
    let deadlines = month_deadlines(&month);
    assert_eq!(
        deadlines
            .iter()
            .map(|event| event["id"].clone())
            .collect::<Vec<_>>(),
        [json!(1), json!(4)]
    );
    assert_eq!(deadlines[0]["instance"], 8);
    assert_eq!(deadlines[0]["timesort"], 10);
}

#[test]
fn submission_status_maps_to_what_the_assignment_page_showed() {
    let status = |submission: &str, grading: &str, canedit: bool, grade: Option<&str>| {
        let mut value = json!({"lastattempt": {"submission": {"status": submission},
            "gradingstatus": grading, "canedit": canedit}, "warnings": []});
        if let Some(grade) = grade {
            value["feedback"] = json!({"gradefordisplay": grade});
        }
        assign_state(7, &value).unwrap()
    };
    let graded = status(
        "submitted",
        "released",
        false,
        Some("<span> 85.00&nbsp;/ 100.00 </span>"),
    );
    assert_eq!(graded["submission"], "graded");
    assert_eq!(graded["grade"], "85.00 / 100.00");
    assert_eq!(graded["acceptsSubmission"], false);
    assert_eq!(
        status("submitted", "graded", false, None)["submission"],
        "graded"
    );
    assert_eq!(
        status("submitted", "notgraded", true, Some("-"))["grade"],
        json!(null)
    );
    // Only Moodle's exact "submitted" is submitted; a draft is still open.
    let draft = status("draft", "notgraded", true, None);
    assert_eq!(draft["submission"], "open");
    assert_eq!(draft["acceptsSubmission"], true);
    assert_eq!(status("new", "notgraded", true, None)["submission"], "open");
    let team = json!({"lastattempt": {"teamsubmission": {"status": "submitted"}, "gradingstatus": "notgraded"}});
    assert_eq!(assign_state(7, &team).unwrap()["submission"], "submitted");
    // A response without the student's attempt says nothing about it.
    assert!(assign_state(7, &json!({"warnings": []})).is_none());
}

#[test]
fn quiz_state_needs_a_finished_attempt_and_a_shown_grade() {
    let quiz = json!({"grade": 100, "decimalpoints": 2});
    let finished = json!({"attempts": [{"state": "inprogress"}, {"state": "finished"}]});
    let graded = quiz_state(
        9,
        &finished,
        &json!({"hasgrade": true, "grade": 85}),
        Some(&quiz),
    );
    assert_eq!(graded["submission"], "graded");
    assert_eq!(graded["grade"], "85.00 / 100.00");
    let hidden = quiz_state(9, &finished, &json!({"hasgrade": false}), Some(&quiz));
    assert_eq!(hidden["submission"], "submitted");
    assert_eq!(hidden["grade"], json!(null));
    let open = quiz_state(
        9,
        &json!({"attempts": [{"state": "inprogress"}]}),
        &json!({"hasgrade": true, "grade": 50}),
        Some(&quiz),
    );
    assert_eq!(open["submission"], "open");
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

#[test]
fn announcements_carry_subject_date_body_and_attachments() {
    let article = forum_article(&json!({
        "id": 501, "discussion": 77, "subject": "Room <change>", "created": 1_760_580_000,
        "message": format!(r#"<p>See <img src="{WS_FILE}"></p>"#),
        "attachments": [{"filename": "map.pdf", "fileurl": format!("{WS_FILE}?token=abc")}]
    }))
    .unwrap();
    assert!(article.starts_with(
        r#"<article id="announcement-p501" data-source-url="https://lms.s.isct.ac.jp/2025/mod/forum/discuss.php?d=77"><h5>Room &lt;change&gt;</h5>"#
    ));
    // 2025-10-16 02:00 UTC is 11:00 in Japan, in the form the reader parses.
    assert!(article.contains("<time>2025年 10月 16日 11:00</time>"));
    assert!(article.contains(&format!(r#"<img src="{FILE}">"#)));
    assert!(article.contains(&format!(r#"<a href="{FILE}">map.pdf</a>"#)));
    assert!(!article.contains("token"));
    assert!(forum_article(&json!({"id": 1})).is_none());
}

#[test]
fn body_budget_bounds_each_body_and_the_whole_course_read() {
    let budget = BodyBudget::new();
    let long = "あ".repeat(150_000);
    let first = budget.take(&long).unwrap();
    assert_eq!(first.chars().count(), 100_000);
    for _ in 0..29 {
        budget.take(&long).unwrap();
    }
    // 3,000,000 characters are spent; later bodies keep their cached copy.
    assert!(budget.take("x").is_none());
}
