use super::*;
use crate::schedule::SchoolReads;
use crate::schedule::{
    course_timeline, occurrences_on_date, planning_week, resolve_day_agenda, AgendaOptions,
    Occurrence,
};
use serde_json::json;

fn workspace() -> Value {
    json!({
        "currentTerm":"2026-3Q", "selectedTerm":"2026-3Q",
        "courses":[{"id":"ex","termId":"2026-3Q","offeringId":"202600215","title":"Exercises in Physics II English Class","code":"LAS.P106","color":"#526e91",
            "meetings":[{"days":[5],"periods":[7,8],"startTime":"15:25","endTime":"17:05","room":"M-B107"}]}],
        "memberships":[
            {"id":"m3","termId":"2026-3Q","courseId":"ex","status":"lms-member"},
            {"id":"m4","termId":"2026-4Q","courseId":"ex","status":"lms-member"}
        ],
        "planItems":[], "events":[], "intensiveSchedules":[], "exceptions":[],
        "scheduleOverrides":[], "exams":[],
        "classSchedules":[{"id":"class-schedule:2026:2026-LAS.P106-EN","academicYear":2026,"courseCode":"LAS.P106",
            "offeringIds":["202600215"],"quarters":["3Q","4Q"],"sourceIds":["pdf"],"sessions":[
                {"date":"2026-10-02","kind":"orientation"},
                {"date":"2026-10-16","kind":"exercise","group":"red","number":"3Q-1"},
                {"date":"2026-10-23","kind":"exercise","group":"white","number":"3Q-1"},
                {"date":"2026-11-27","kind":"exam"},
                {"date":"2026-12-04","kind":"no-class"}
            ]}],
        "academicCalendar":[
            {"id":"calendar:2026:quarter-start:3Q","academicYear":2026,"quarter":"3Q","date":"2026-09-28","type":"quarter-start","teachingRanges":[{"start":"2026-10-01","end":"2026-11-24"}]},
            {"id":"calendar:2026:quarter-end:3Q","academicYear":2026,"quarter":"3Q","date":"2026-12-02","type":"quarter-end"},
            {"id":"calendar:2026:quarter-start:4Q","academicYear":2026,"quarter":"4Q","date":"2026-12-03","type":"quarter-start"},
            {"id":"calendar:2026:quarter-end:4Q","academicYear":2026,"quarter":"4Q","date":"2027-02-08","type":"quarter-end"}
        ]
    })
}

fn blocks(workspace: &Value, term: &str, date: &str) -> Vec<Occurrence> {
    occurrences_on_date(workspace, term, date, AgendaOptions::default()).unwrap()
}

fn alternation(block: &Occurrence) -> &Alternation {
    block.alternation.as_ref().unwrap()
}

#[test]
fn published_sessions_show_only_the_chosen_group() {
    let mut data = workspace();
    data["classGroups"] = json!({"2026-3Q4Q":"white"});
    assert!(blocks(&data, "2026-3Q", "2026-10-16").is_empty());
    let white = &blocks(&data, "2026-3Q", "2026-10-23")[0];
    assert_eq!(alternation(white).turn, Some(Turn::White));
    assert_eq!(alternation(white).number.as_deref(), Some("3Q-1"));
    let orientation = &blocks(&data, "2026-3Q", "2026-10-02")[0];
    assert_eq!(alternation(orientation).turn, Some(Turn::All));
    // Unlisted dates and published cancellations have no block.
    assert!(blocks(&data, "2026-3Q", "2026-10-30").is_empty());
    assert!(blocks(&data, "2026-4Q", "2026-12-04").is_empty());
    // The exam falls after the teaching range but is a published session.
    let exam = &blocks(&data, "2026-3Q", "2026-11-27")[0];
    assert_eq!(alternation(exam).session.as_deref(), Some("exam"));
}

#[test]
fn exercise_and_lab_share_their_slot_without_a_schedule() {
    let mut data = workspace();
    data["classSchedules"] = json!([]);
    data["courses"].as_array_mut().unwrap().extend([
        json!({"id":"lab","termId":"2026-3Q","title":"Physics Experiment II","code":"LAS.P108","color":"#8a5a44",
            "meetings":[{"days":[5],"periods":[7,8],"startTime":"15:25","endTime":"17:05","room":"S-B101"}]}),
        json!({"id":"other","termId":"2026-3Q","title":"Seminar","code":"LAH.S101","color":"#445566",
            "meetings":[{"days":[5],"periods":[7,8],"startTime":"15:25","endTime":"17:05","room":""}]}),
    ]);
    data["memberships"].as_array_mut().unwrap().extend([
        json!({"id":"m5","termId":"2026-3Q","courseId":"lab","status":"lms-member"}),
        json!({"id":"m6","termId":"2026-3Q","courseId":"other","status":"lms-member"}),
    ]);
    let check = |blocks: &[Occurrence]| {
        let shared = blocks
            .iter()
            .find(|row| row.course_id.as_deref() == Some("ex"))
            .unwrap();
        assert_eq!(blocks.len(), 2, "the lab folds into the exercise block");
        assert_eq!(
            shared.title,
            "Exercises in Physics II English Class / Physics Experiment II"
        );
        assert_eq!(shared.room, "M-B107 / S-B101");
        // Without a published schedule the shared block stays weekly.
        assert!(!alternation(shared).scheduled && alternation(shared).turn.is_none());
        let pair = alternation(shared).pair.as_ref().unwrap();
        assert_eq!(
            (pair[0].course_id.as_str(), pair[1].course_id.as_str()),
            ("ex", "lab")
        );
        // The pair is not a clash, but another course in the slot still is.
        assert_eq!(
            shared.conflict_with.as_deref(),
            Some(&["Seminar".to_owned()][..])
        );
    };
    check(
        &resolve_day_agenda(
            &data,
            &SchoolReads::fixture(&data),
            "2026-3Q",
            "2026-10-30",
            AgendaOptions::default(),
        )
        .unwrap(),
    );
    let week = planning_week(&data, "2026-3Q").unwrap();
    check(&week.days[4].blocks);
    assert_eq!(week.conflict_count, 1);
    // The lab's own timeline still finds the shared block.
    let now = chrono::DateTime::parse_from_rfc3339("2026-10-30T12:00:00+09:00").unwrap();
    let timeline = course_timeline(&data, &SchoolReads::fixture(&data), "lab", now, 0, 0).unwrap();
    assert_eq!(timeline.next.as_deref(), Some("2026-10-30T15:25:00+09:00"));
}

#[test]
fn choices_name_the_next_group_session() {
    let choices = class_group_choices(&workspace(), "2026-10-15");
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].semester, "2026-3Q4Q");
    assert!(choices[0].needs_choice && choices[0].courses[0].scheduled);
    assert_eq!(choices[0].terms, ["2026-3Q", "2026-4Q"]);
    let next = choices[0].next.as_ref().unwrap();
    assert_eq!((next.date.as_str(), next.turn), ("2026-10-16", Turn::Red));
}
