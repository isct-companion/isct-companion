#[path = "../src/ics.rs"]
mod ics;
#[path = "../src/plans.rs"]
mod plans;

use serde_json::{json, Value};
use std::collections::BTreeMap;

const OFFICIAL_2026_CALENDAR: &str =
    include_str!("../subscriptions/data/2026/academic-calendar.json");

#[derive(Debug)]
struct Event {
    fields: BTreeMap<String, Vec<String>>,
}

impl Event {
    fn values(&self, name: &str) -> Vec<&str> {
        self.fields
            .iter()
            .filter(|(key, _)| key.split(';').next() == Some(name))
            .flat_map(|(_, values)| values.iter().map(String::as_str))
            .collect()
    }

    fn value(&self, name: &str) -> &str {
        let values = self.values(name);
        assert_eq!(values.len(), 1, "VEVENT must contain one {name}: {self:?}");
        values[0]
    }
}

/// A deliberately small, strict parser for the subset emitted by the app. It
/// verifies component nesting and lets the acceptance tests compare semantic
/// fields rather than searching arbitrary output substrings.
fn parse_ics(document: &str) -> Vec<Event> {
    assert!(document.ends_with("\r\n"), "ICS must use CRLF records");
    let lines: Vec<_> = document.trim_end_matches("\r\n").split("\r\n").collect();
    assert_eq!(lines.first(), Some(&"BEGIN:VCALENDAR"));
    assert_eq!(lines.last(), Some(&"END:VCALENDAR"));
    assert!(lines.contains(&"BEGIN:VTIMEZONE"));
    assert!(lines.contains(&"TZID:Asia/Tokyo"));

    let mut stack: Vec<&str> = Vec::new();
    let mut events = Vec::new();
    let mut current: Option<Event> = None;
    for line in lines {
        if let Some(component) = line.strip_prefix("BEGIN:") {
            stack.push(component);
            if component == "VEVENT" {
                assert!(current.is_none(), "nested VEVENT is invalid");
                current = Some(Event {
                    fields: BTreeMap::new(),
                });
            }
            continue;
        }
        if let Some(component) = line.strip_prefix("END:") {
            assert_eq!(stack.pop(), Some(component), "unbalanced ICS component");
            if component == "VEVENT" {
                events.push(current.take().expect("VEVENT body"));
            }
            continue;
        }
        assert!(line.contains(':'), "invalid content line: {line}");
        if let Some(event) = current.as_mut() {
            let (key, value) = line.split_once(':').expect("content line");
            event
                .fields
                .entry(key.to_string())
                .or_default()
                .push(value.to_string());
        }
    }
    assert!(stack.is_empty(), "all ICS components must close");
    for event in &events {
        event.value("UID");
        event.value("DTSTART");
        event.value("DTEND");
        event.value("SUMMARY");
    }
    events
}

fn official_calendar_rows() -> Vec<Value> {
    let source: Value = serde_json::from_str(OFFICIAL_2026_CALENDAR).unwrap();
    assert_eq!(
        source.pointer("/source/0/url").and_then(Value::as_str),
        Some("https://g-www.jim.titech.ac.jp/student/students/life/schedules")
    );
    let mut rows = vec![
        json!({ "date": source["quarters"]["3Q"]["start"], "type": "quarter-start" }),
        json!({ "date": source["quarters"]["3Q"]["end"], "type": "quarter-end" }),
    ];
    rows.extend(source["days"].as_array().unwrap().iter().cloned());
    rows
}

fn course_workspace() -> Value {
    json!({
        "academicProfile": { "years": [] },
        "academicCalendar": official_calendar_rows(),
        "courses": [{
            "id": "local-course-monday",
            "termId": "2026-3Q",
            "title": "Local Monday course",
            "meetings": [{
                "days": [1],
                "periods": [1, 2],
                "startTime": "08:50",
                "endTime": "10:30",
                "room": "Local room"
            }]
        }],
        "memberships": [{ "courseId": "local-course-monday", "status": "registered" }],
        "planItems": [],
        "intensiveSchedules": [],
        "plans": []
    })
}

fn event_with_uid<'a>(events: &'a [Event], uid: &str) -> &'a Event {
    events
        .iter()
        .find(|event| event.value("UID") == uid)
        .unwrap_or_else(|| panic!("missing VEVENT {uid}"))
}

#[test]
fn weekly_course_uses_official_subscription_quarter_boundary() {
    let document = ics::export_quarter(&course_workspace(), "2026-3Q").unwrap();
    let events = parse_ics(&document);
    let weekly = event_with_uid(&events, "local-course-monday-weekly-0-1@campus-life.local");

    assert_eq!(weekly.value("DTSTART"), "20260928T085000");
    assert_eq!(
        weekly.value("RRULE"),
        "FREQ=WEEKLY;BYDAY=MO;UNTIL=20261202T145959Z"
    );
}

#[test]
fn weekly_course_reproduces_official_no_class_day_as_exdate() {
    let document = ics::export_quarter(&course_workspace(), "2026-3Q").unwrap();
    let events = parse_ics(&document);
    let weekly = event_with_uid(&events, "local-course-monday-weekly-0-1@campus-life.local");
    assert_eq!(weekly.values("EXDATE"), vec!["20261012T085000"]);
}

#[test]
fn official_replacement_day_creates_a_stable_one_off_course_event() {
    let document = ics::export_quarter(&course_workspace(), "2026-3Q").unwrap();
    let events = parse_ics(&document);
    let replacement = events
        .iter()
        .find(|event| event.value("DTSTART") == "20261015T085000")
        .expect("official 2026-10-15 Monday replacement VEVENT");

    assert_eq!(replacement.value("DTSTART"), "20261015T085000");
    assert_eq!(replacement.value("DTEND"), "20261015T103000");
    assert!(replacement.values("RRULE").is_empty());
    assert_eq!(
        replacement.value("UID"),
        "local-course-monday-replacement-2026-10-15@campus-life.local",
        "replacement UID must depend on course and date, not meeting array position"
    );
}

#[test]
fn intensive_schedule_exports_each_kept_day_and_omits_removed_day() {
    let mut workspace = course_workspace();
    workspace["courses"][0]["meetings"] = json!([]);
    workspace["intensiveSchedules"] = json!([{
        "id": "intensive-local-course",
        "courseId": "local-course-monday",
        "from": "2026-10-20",
        "to": "2026-10-22",
        "periods": [1, 2],
        "days": [
            { "date": "2026-10-20", "periods": [1, 2] },
            { "date": "2026-10-21", "periods": null },
            { "date": "2026-10-22", "periods": [1, 2] }
        ]
    }]);

    let events = parse_ics(&ics::export_quarter(&workspace, "2026-3Q").unwrap());
    let starts: Vec<_> = events.iter().map(|event| event.value("DTSTART")).collect();
    assert_eq!(starts, vec!["20261020T085000", "20261022T085000"]);
    assert_eq!(events[0].value("DTEND"), "20261020T103000");
    assert_eq!(
        events[0].value("UID"),
        "local-course-monday-intensive-2026-10-20@campus-life.local"
    );
    assert_eq!(
        events[1].value("UID"),
        "local-course-monday-intensive-2026-10-22@campus-life.local"
    );
}

#[test]
fn repeating_plan_round_trips_rule_exdate_uid_and_tokyo_time() {
    let mut workspace = course_workspace();
    workspace["plans"] = json!([{
        "id": "plan-study",
        "title": "Study block",
        "start": "2026-10-06T19:00:00+09:00",
        "duration": 90,
        "rrule": "FREQ=WEEKLY;BYDAY=TU,FR;UNTIL=20261130T145959Z",
        "exdates": ["2026-10-09"]
    }]);

    let events = parse_ics(&ics::export_quarter(&workspace, "2026-3Q").unwrap());
    let plan = event_with_uid(&events, "plan-study@campus-life.local");
    assert_eq!(plan.value("DTSTART"), "20261006T190000");
    assert_eq!(plan.value("DTEND"), "20261006T203000");
    assert_eq!(
        plan.value("RRULE"),
        "FREQ=WEEKLY;BYDAY=TU,FR;UNTIL=20261130T145959Z"
    );
    assert_eq!(plan.values("EXDATE"), vec!["20261009T190000"]);
}
