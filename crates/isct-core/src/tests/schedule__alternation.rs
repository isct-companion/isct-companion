use super::*;
use crate::schedule::{occurrences_on_date, AgendaOptions, Occurrence};
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
        "planItems":[], "events":[], "plans":[], "intensiveSchedules":[], "exceptions":[],
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
fn an_unpublished_class_stays_weekly_and_is_marked() {
    let mut data = workspace();
    data["classSchedules"] = json!([]);
    let weekly = &blocks(&data, "2026-3Q", "2026-10-30")[0];
    assert!(!alternation(weekly).scheduled && alternation(weekly).turn.is_none());
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
