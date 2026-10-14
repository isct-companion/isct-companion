use chrono::DateTime;
use serde_json::{json, Value};
use std::{future::Future, pin::Pin, sync::Arc};

use super::calendar::quarter_range;
use super::*;

fn workspace() -> Value {
    json!({
        "currentTerm":"2026-1Q", "selectedTerm":"2026-1Q", "terms":[],
        "settings":{"contentLanguage":"en","notifications":{"classes":true,"assignments":true,"classLeadMinutes":10}},
        "academicProfile":{"years":[]},
        "courses":[{"id":"course","termId":"2026-1Q","title":"授業, A; B","code":"CSC.T241","color":"#526e91","meetings":[{"days":[1],"startTime":"08:50","endTime":"10:30","room":"M-101"}]}],
        "memberships":[{"id":"member","termId":"2026-1Q","courseId":"course","status":"registered","evidence":{"provider":"school"}}],
        "planItems":[], "events":[], "plans":[], "intensiveSchedules":[], "exceptions":[], "scheduleOverrides":[], "exams":[], "assignments":[], "registrations":[], "importantDates":[],
        "academicCalendar":[
          {"id":"calendar:2026:quarter-start:1Q","academicYear":2026,"quarter":"1Q","date":"2026-04-07","type":"quarter-start","label":{"en":"start"},"teachingRanges":[{"start":"2026-04-07","end":"2026-05-27"}]},
          {"id":"calendar:2026:quarter-end:1Q","academicYear":2026,"quarter":"1Q","date":"2026-06-09","type":"quarter-end","label":{"en":"end"}}
        ]
    })
}

#[test]
fn quarter_and_calendar_precedence_match_desktop() {
    let mut data = workspace();
    assert_eq!(
        quarter_range(&data, "2026-1Q").unwrap().unwrap().from,
        "2026-04-07"
    );
    data["academicCalendar"].as_array_mut().unwrap().push(json!({"id":"published","date":"2026-04-13","type":"replacement","replacementDay":2,"label":{"en":"Tuesday"}}));
    data["exceptions"] =
        json!([{"id":"mine","date":"2026-04-13","type":"no-class","label":"Trip"}]);
    assert_eq!(
        calendar::calendar_state(&data, "2026-04-13").unwrap()["id"],
        "mine"
    );
    assert!(occurrences_on_date(
        &data,
        "2026-1Q",
        "2026-04-13",
        AgendaOptions {
            include_personal: true,
            ..Default::default()
        }
    )
    .unwrap()
    .is_empty());
}

#[test]
fn planning_template_and_dated_festival_view_keep_distinct_semantics() {
    let mut data = workspace();
    data["currentTerm"] = json!("2026-3Q");
    data["selectedTerm"] = json!("2026-3Q");
    data["courses"][0]["termId"] = json!("2026-3Q");
    data["memberships"][0]["termId"] = json!("2026-3Q");
    data["courses"][0]["meetings"][0]["days"] = json!([2, 5]);
    data["academicCalendar"] = json!([
        {"id":"calendar:2026:quarter-start:3Q","academicYear":2026,"quarter":"3Q","date":"2026-09-28","type":"quarter-start","label":{"en":"3Q starts"}},
        {"id":"calendar:2026:quarter-end:3Q","academicYear":2026,"quarter":"3Q","date":"2026-12-02","type":"quarter-end","label":{"en":"3Q ends"}},
        {"id":"festival","academicYear":2026,"date":"2026-10-09","type":"no-class","label":{"en":"No classes due to Campus Festival","ja":"工大祭のため授業休み"}}
    ]);

    let template = planning_week(&data, "2026-3Q").unwrap();
    assert_eq!(template.days[4].blocks.len(), 1);
    let view = query(
        &data,
        WorkspaceQuery::ScheduleView {
            term: "2026-3Q".into(),
            from: "2026-10-05".into(),
            to: "2026-10-11".into(),
            include_personal: false,
        },
    )
    .unwrap();
    let WorkspaceQueryResult::ScheduleView(view) = view else {
        panic!("expected schedule view")
    };
    let friday = view
        .agendas
        .iter()
        .find(|day| day.date == "2026-10-09")
        .unwrap();
    assert_eq!(friday.occurrences.len(), 1);
    assert_eq!(friday.occurrences[0].cancelled, Some(true));
    assert!(friday.no_classes);
    assert_eq!(friday.calendar_notice.as_deref(), Some("Campus Festival"));
    assert!(
        occurrences_on_date(&data, "2026-3Q", "2026-10-09", AgendaOptions::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn overview_uses_live_terms_global_timetable_and_static_department_name() {
    let mut data = workspace();
    data["selectedTerm"] = json!("2099-4Q");
    data["settings"]["departmentId"] = json!("undergraduate:cs");
    data["courses"].as_array_mut().unwrap().push(json!({
        "id":"future-course","termId":"2027-1Q","title":"Future","credits":2,
        "meetings":[]
    }));
    data["memberships"].as_array_mut().unwrap().push(json!({
        "id":"future-membership","termId":"2027-1Q","courseId":"future-course","status":"registered"
    }));
    let WorkspaceQueryResult::WorkspaceOverview(view) = query(
        &data,
        WorkspaceQuery::WorkspaceOverview {
            term: String::new(),
        },
    )
    .unwrap() else {
        panic!("expected workspace overview")
    };
    assert_eq!(view.courses_term, "2026-1Q");
    assert_eq!(view.term, "2026-1Q");
    assert_eq!(view.today_term, "2026-1Q");
    assert!(!view.all_terms.contains(&"2099-4Q".to_string()));
    assert!(view.all_terms.contains(&"2027-1Q".to_string()));
    assert_eq!(view.enrolled_course_ids, vec!["course"]);
    assert!(view
        .on_timetable_course_ids
        .contains(&"future-course".to_string()));
    assert_eq!(
        view.default_finder_keys,
        vec!["curr".to_string(), "情報工学系".to_string()]
    );
}

#[test]
fn civil_plans_survive_holidays_and_conflict() {
    let mut data = workspace();
    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"off","date":"2026-04-13","type":"holiday","label":{"en":"Off"}}));
    data["events"] = json!([{"id":"event","termId":"2026-1Q","date":"2026-04-13","title":"Event","room":"","startTime":"09:30","endTime":"10:00"}]);
    data["plans"] = json!([{"id":"plan","title":"Plan","start":"2026-04-13T09:00:00+09:00","duration":60,"exdates":[]}]);
    let rows = occurrences_on_date(
        &data,
        "2026-1Q",
        "2026-04-13",
        AgendaOptions {
            include_personal: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![OccurrenceKind::Plan, OccurrenceKind::Event]
    );
    assert!(rows.iter().all(|row| row.conflict == Some(true)));
}

#[test]
fn recurrence_and_intensive_dates_are_strict() {
    let plan = json!({"start":"2026-03-30T09:00:00+09:00","rrule":"FREQ=MONTHLY;BYDAY=1MO;COUNT=3","exdates":["2026-05-04"]});
    assert_eq!(
        next_plan_occurrences(&plan, 5).unwrap(),
        vec!["2026-04-06T09:00:00+09:00", "2026-06-01T09:00:00+09:00"]
    );
    assert_eq!(
        next_plan_occurrences(
            &json!({
                "start":"2026-04-12T15:30:00Z",
                "rrule":"FREQ=DAILY;UNTIL=20260413T153000Z",
                "exdates":[]
            }),
            5
        )
        .unwrap(),
        vec!["2026-04-13T00:30:00+09:00", "2026-04-14T00:30:00+09:00"]
    );
    assert_eq!(
        intensive_days("2026-04-12", "2026-04-14", [3, 4])
            .unwrap()
            .len(),
        3
    );
    assert!(intensive_days("2026-04-14", "2026-04-12", [3, 4]).is_err());
}

#[test]
fn advanced_calendar_and_course_rules_match_desktop_cases() {
    let mut data = workspace();
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-06", Default::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-13", Default::default())
            .unwrap()
            .len(),
        1
    );
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-06-08", Default::default())
            .unwrap()
            .is_empty()
    );
    data["academicCalendar"][0]["teachingRanges"] = json!([
        {"start":"2026-04-07","end":"2026-04-14"},
        {"start":"2026-04-27","end":"2026-05-27"}
    ]);
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-20", Default::default())
            .unwrap()
            .is_empty()
    );
    data["academicCalendar"][0]["teachingRanges"] =
        json!([{"start":"2026-04-07","end":"2026-05-27"}]);

    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"replacement","date":"2026-04-14","type":"replacement",
            "replacementDay":1,"label":{"en":"Monday classes"}
        }));
    data["plans"] = json!([{"id":"plan","title":"Personal","start":"2026-04-13T09:00:00+09:00","duration":60,"rrule":"FREQ=WEEKLY;COUNT=2","exdates":[]}]);
    let replacement =
        occurrences_on_date(&data, "2026-1Q", "2026-04-14", Default::default()).unwrap();
    assert_eq!(
        replacement.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![OccurrenceKind::Course]
    );
    data["exceptions"] =
        json!([{"id":"manual","date":"2026-04-14","type":"no-class","label":"Override"}]);
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-14", Default::default())
            .unwrap()
            .is_empty()
    );

    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"missing","date":"2026-04-13","type":"holiday","label":{"en":"Old"},
            "missingSince":"2026-04-01T00:00:00Z"
        }));
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-13", Default::default())
            .unwrap()
            .iter()
            .any(|row| row.kind == OccurrenceKind::Course)
    );

    data["plans"] = json!([]);
    data["courses"][0]["meetings"] = json!([]);
    data["intensiveSchedules"] = json!([{
        "id":"intensive","courseId":"course","from":"2026-04-12","to":"2026-04-14",
        "periods":[1,2],"days":[{"date":"2026-04-12","periods":[3,4]},{"date":"2026-04-14","periods":null}]
    }]);
    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"intensive-holiday","date":"2026-04-12","type":"holiday","label":{"en":"Off"}
        }));
    assert_eq!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-12", Default::default()).unwrap()[0]
            .start_time,
        "10:45"
    );
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-13", Default::default())
            .unwrap()
            .is_empty()
    );
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-14", Default::default())
            .unwrap()
            .is_empty()
    );

    data["intensiveSchedules"] = json!([]);
    data["courses"][0]["meetings"] = json!([{"days":[1],"startTime":"08:50","endTime":"10:30","room":"M-101","validFrom":"2026-04-20","validTo":"2026-04-27"}]);
    data["courses"][0]["overrides"] = json!({"title":"My title"});
    assert!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-13", Default::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        occurrences_on_date(&data, "2026-1Q", "2026-04-20", Default::default()).unwrap()[0].title,
        "My title"
    );
}

#[test]
fn exams_and_single_occurrence_overrides_preserve_identity_and_conflicts() {
    let mut data = workspace();
    data["memberships"][0]["evidence"] = json!({"provider":"school"});
    data["exams"] = json!([{"id":"exam","academicYear":2026,"level":"undergraduate","quarter":"1Q","courseCode":"CSC.T241","date":"2026-04-13","label":{"ja":"試験"},"periods":[3,4],"room":{"ja":"W9-101"},"rooms":[],"status":"scheduled","sourceIds":["notice"],"sourcePages":[1]}]);
    let rows = resolve_day_agenda(&data, "2026-1Q", "2026-04-13", Default::default()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, OccurrenceKind::Exam);
    assert_eq!(rows[0].course_id.as_deref(), Some("course"));
    assert_eq!(rows[0].start_time, "10:45");
    assert_eq!(rows[0].room, "W9-101");

    data["exams"][0]["periods"] = json!([1, 2]);
    data["events"] = json!([{"id":"event","termId":"2026-1Q","title":"Interview","date":"2026-04-13","room":"Online","startTime":"08:50","endTime":"10:30","blocking":true}]);
    let rows = resolve_day_agenda(&data, "2026-1Q", "2026-04-13", Default::default()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|row| row.kind == OccurrenceKind::Exam)
            .unwrap()
            .conflict_with
            .clone(),
        Some(vec!["Interview".to_string()])
    );

    data["exams"] = json!([]);
    data["events"] = json!([]);
    data["scheduleOverrides"] = json!([{"id":"override","targetKey":"course-weekly-0-2026-04-13","targetKind":"course","date":"2026-04-14","periods":[5,6],"room":"Moved","hidden":false}]);
    assert!(
        resolve_day_agenda(&data, "2026-1Q", "2026-04-13", Default::default())
            .unwrap()
            .is_empty()
    );
    let moved = resolve_day_agenda(&data, "2026-1Q", "2026-04-14", Default::default()).unwrap();
    assert_eq!(moved[0].room, "Moved");
    assert_eq!(moved[0].start_time, "13:30");
    assert_eq!(moved[0].overridden, Some(true));
}

#[test]
fn recurrence_ics_and_cancelled_context_cover_desktop_boundaries() {
    let mut data = workspace();
    let plan = json!({"id":"plan","title":"Personal plan","start":"2026-04-06T09:00:00+09:00","duration":60,"rrule":"FREQ=WEEKLY;INTERVAL=2;COUNT=2","exdates":[]});
    assert!(occurrences_on_date(
        &json!({"plans":[plan.clone()]}),
        "2026-1Q",
        "2026-04-13",
        Default::default()
    )
    .unwrap()
    .is_empty());
    assert_eq!(next_plan_occurrences(&plan, 5).unwrap().len(), 2);
    assert!(intensive_days("2026-04-01", "2026-04-02", [0, 10]).is_err());
    assert!(intensive_days("2026-04-01", "2026-04-02", [4, 3]).is_err());

    data["plans"] = json!([plan]);
    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"off","date":"2026-04-13","type":"holiday","label":{"en":"Off"}}));
    let standard = export_quarter_ics(
        &data,
        "2026-1Q",
        CalendarExportScope::Courses,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    let expanded = export_quarter_ics(
        &data,
        "2026-1Q",
        CalendarExportScope::CoursesAndPlans,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert!(!standard.contains("SUMMARY:Personal plan"));
    assert!(expanded.contains("SUMMARY:Personal plan"));
    let range = calendar::quarter_range(&data, "2026-1Q").unwrap().unwrap();
    let day_count =
        (parse_civil_date(&range.to).unwrap() - parse_civil_date(&range.from).unwrap()).num_days();
    let expected_events: usize = (0..=day_count)
        .map(|offset| {
            occurrences_on_date(
                &data,
                "2026-1Q",
                &add_days(&range.from, offset).unwrap(),
                AgendaOptions {
                    include_personal: false,
                    ..Default::default()
                },
            )
            .unwrap()
            .len()
        })
        .sum();
    assert_eq!(standard.matches("BEGIN:VEVENT").count(), expected_events);

    for kind in ["holiday", "no-class", "break"] {
        data["academicCalendar"]
            .as_array_mut()
            .unwrap()
            .retain(|row| row.get("id") != Some(&json!("off")));
        data["academicCalendar"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"off","date":"2026-04-13","type":kind,"label":{"en":"Off"}}));
        let rows=occurrences_on_date(&data,"2026-1Q","2026-04-13",AgendaOptions{include_cancelled_courses:true,events:Some(vec![json!({"id":"personal","termId":"2026-1Q","title":"Personal","date":"2026-04-13","room":"","startTime":"09:00","endTime":"10:00"})]),..Default::default()}).unwrap();
        assert!(rows
            .iter()
            .any(|row| row.kind == OccurrenceKind::Course && row.cancelled == Some(true)));
        let personal = rows.iter().find(|row| row.id == "personal").unwrap();
        assert_eq!(personal.cancelled, None);
        assert!(rows.iter().all(|row| row.conflict != Some(true)));
    }
}

#[test]
fn timestamps_reject_locale_and_relaxed_rfc3339_forms() {
    assert!(parse_timestamp("2026-04-13T08:40:00Z").is_ok());
    assert!(parse_timestamp("2026-04-13 08:40:00Z").is_err());
    assert!(parse_timestamp("2026-04-13t08:40:00z").is_err());
    assert!(parse_timestamp("04/13/2026 08:40").is_err());
}

#[test]
fn credits_and_color_match_typescript_contract() {
    let rows = json!([
      {"kind":"declared","offeringId":"202600001","quarters":[1,2],"effectiveCredits":2.0},
      {"kind":"declared","offeringId":"202600002","quarters":[1],"effectiveCredits":1.5}
    ]);
    assert_eq!(
        declared_term_credits(rows.as_array().unwrap(), "2026-1Q").unwrap(),
        Some(2.5)
    );
    let course = json!({"id":"x","code":"CSC.T241-01","color":"#526e91"});
    assert_eq!(
        display_color(&course),
        display_color(&json!({"id":"other","code":"CSC.T241[2027]","color":"#556b8d"}))
    );
}

#[test]
fn academic_year_credits_and_colors_preserve_source_semantics() {
    assert_eq!(calendar::academic_year_of("2027-03-31").unwrap(), 2026);
    assert_eq!(calendar::academic_year_of("2026-04-01").unwrap(), 2026);
    let mut data = workspace();
    data["academicCalendar"] = json!([
        {"id":"start","academicYear":2026,"quarter":"3Q","date":"2026-09-28","type":"quarter-start","label":{"en":"3Q starts"}},
        {"id":"end","academicYear":2026,"quarter":"3Q","date":"2026-12-02","type":"quarter-end","label":{"en":"3Q ends"}},
        {"id":"festival","academicYear":2026,"date":"2026-10-09","type":"no-class","label":{"en":"Campus Festival"}},
        {"id":"replacement","academicYear":2026,"date":"2026-10-15","type":"replacement","replacementDay":1,"label":{"en":"Monday classes"}},
        {"id":"missing","academicYear":2026,"date":"2026-11-03","type":"no-class","label":{"en":"Missing"},"missingSince":"2026-10-05T00:00:00Z"}
    ]);
    data["exceptions"] = json!([
        {"id":"own-1","date":"2026-10-15","type":"no-class","label":"Lab trip"},
        {"id":"own-2","date":"2027-01-05","type":"break","label":"Winter break"}
    ]);
    let view = academic_year_calendar(&data, 2026).unwrap();
    assert!(view.published);
    let serialized = serde_json::to_value(&view).unwrap();
    assert_eq!(serialized["days"][0]["origin"], "official");
    assert_eq!(serialized["days"][1]["origin"], "changed");
    assert_eq!(serialized["days"][1]["official"]["replacementDay"], 1);
    assert_eq!(serialized["days"][2]["origin"], "own");
    assert_eq!(serialized["quarters"][0]["origin"], "official");
    data["academicProfile"]["years"] = json!([{"academicYear":2026,"quarterStarts":{"1Q":"2026-04-07","2Q":"2026-06-10","3Q":"2026-10-01","4Q":"2026-12-10"}}]);
    let owned = serde_json::to_value(academic_year_calendar(&data, 2026).unwrap()).unwrap();
    assert_eq!(owned["quarters"][2]["origin"], "own");
    assert_eq!(owned["quarters"][2]["from"], "2026-10-01");
    let unpublished = academic_year_calendar(&data, 2027).unwrap();
    assert!(!unpublished.published);

    let rows = json!([
      {"kind":"declared","offeringId":"202636840","quarters":[3],"effectiveCredits":2},
      {"kind":"declared","offeringId":"202637648","quarters":[3,4],"effectiveCredits":2},
      {"kind":"declared","offeringId":"202600702","quarters":[3],"effectiveCredits":0.5},
      {"kind":"declared","offeringId":"202602427","quarters":[3],"effectiveCredits":2,"deletedAt":"x"}
    ]);
    assert_eq!(
        declared_term_credits(rows.as_array().unwrap(), "2026-3Q").unwrap(),
        Some(3.5)
    );
    assert_eq!(
        declared_term_credits(rows.as_array().unwrap(), "2025-3Q").unwrap(),
        None
    );
    assert_eq!(
        display_color(
            &json!({"id":"c","code":"CSC.T262","color":"#526e91","overrides":{"color":"#806a9d"}})
        ),
        "#806a9d"
    );
    let colors: [String; 5] = [
        "CSC.T262",
        "CSC.T273",
        "CSC.T252",
        "LAH.T319",
        "LAW.W221-12",
    ]
    .map(|code| display_color(&json!({"id":code,"code":code,"color":"#526e91"})));
    assert!(
        colors
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 2
    );
    for code in ["-manual", "[manual"] {
        assert_eq!(
            display_color(&json!({"id":"stable-id","code":code,"color":"#526e91"})),
            display_color(&json!({"id":"stable-id","code":"","color":"#526e91"}))
        );
    }
}

#[test]
fn notification_plan_uses_jst_and_explicit_locale() {
    let mut data = workspace();
    data["assignments"] = json!([{"id":"assignment","courseId":"course","title":"Report","dueAt":"2026-04-14T23:59:00+09:00"}]);
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:00:00+09:00").unwrap();
    let horizon = DateTime::parse_from_rfc3339("2026-04-14T23:00:00+09:00").unwrap();
    let plan = notification_plan(&data, now, horizon, Some(NotificationLocale::Ja)).unwrap();
    assert!(plan
        .items
        .iter()
        .any(|item| item.key.starts_with("class:") && item.body.contains("10分後")));
    assert!(plan.items.iter().any(
        |item| item.key == "assignment:assignment:1440" && item.body.contains("締切まで24時間")
    ));
}

fn due_plan(data: &Value, now: &str) -> NotificationPlan {
    let through = DateTime::parse_from_rfc3339(now).unwrap();
    notification_plan(
        data,
        through - chrono::Duration::minutes(5),
        through,
        Some(NotificationLocale::En),
    )
    .unwrap()
}

#[test]
fn class_reminders_preserve_lead_grace_calendar_and_toggle_policy() {
    let mut data = workspace();
    assert!(due_plan(&data, "2026-04-13T08:39:00+09:00")
        .items
        .is_empty());
    let reminder = due_plan(&data, "2026-04-13T08:40:30+09:00")
        .items
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(reminder.title, "授業, A; B");
    assert_eq!(reminder.body, "Starts in 10 min · 08:50 · M-101");
    assert!(due_plan(&data, "2026-04-13T08:46:00+09:00")
        .items
        .is_empty());

    data["settings"]["notifications"]["classLeadMinutes"] = json!(30);
    assert_eq!(due_plan(&data, "2026-04-13T08:20:00+09:00").items.len(), 1);
    data["exceptions"] = json!([
        {"id":"off","date":"2026-04-13","type":"no-class","label":"Off"},
        {"id":"monday-on-wednesday","date":"2026-04-15","type":"replacement","replacementDay":1,"label":"Monday classes"}
    ]);
    assert!(due_plan(&data, "2026-04-13T08:20:00+09:00")
        .items
        .is_empty());
    assert_eq!(due_plan(&data, "2026-04-15T08:20:00+09:00").items.len(), 1);
    data["settings"]["notifications"]["classes"] = json!(false);
    assert!(due_plan(&data, "2026-04-15T08:20:00+09:00")
        .items
        .is_empty());
}

#[test]
fn assignment_reminders_preserve_leads_and_open_work_filter() {
    let mut data = workspace();
    data["assignments"] = json!([{
        "id":"assignment","courseId":"course","title":"Local report",
        "dueAt":"2026-04-14T23:59:00+09:00"
    }]);
    let one_day = due_plan(&data, "2026-04-13T23:59:00+09:00");
    assert_eq!(one_day.items[0].body, "Due in 24 hours · 授業, A; B");
    assert_eq!(
        due_plan(&data, "2026-04-14T22:59:30+09:00").items[0].key,
        "assignment:assignment:60"
    );

    data["assignments"] = json!([
        {"id":"done","courseId":"course","title":"Done","dueAt":"2026-04-14T23:59:00+09:00","submission":"submitted"},
        {"id":"hidden","courseId":"course","title":"Hidden","dueAt":"2026-04-14T23:59:00+09:00","hidden":true},
        {"id":"undated","courseId":"course","title":"Undated","dueAt":null}
    ]);
    assert!(due_plan(&data, "2026-04-13T23:59:00+09:00")
        .items
        .is_empty());
}

#[test]
fn task_view_strictly_classifies_today_and_lecture_work() {
    let mut data = workspace();
    data["assignments"] = json!([
        {"id":"soon","courseId":"course","title":"Soon","dueAt":"2026-04-13T10:00:00+09:00"},
        {"id":"later","courseId":"course","title":"Later","dueAt":"2026-04-22T10:00:00+09:00","pinned":true},
        {"id":"past","courseId":"course","title":"Past","dueAt":"2026-04-12T10:00:00+09:00"},
        {"id":"done","courseId":"course","title":"Done","dueAt":"2026-04-14T10:00:00+09:00","submission":"submitted"},
        {"id":"hidden","courseId":"course","title":"Hidden","dueAt":null,"hidden":true},
        {"id":"invalid","courseId":"course","title":"Invalid","dueAt":"yesterday"},
        {"id":"undated","courseId":"course","title":"Undated","dueAt":null}
    ]);
    let view = task_view(
        &data,
        DateTime::parse_from_rfc3339("2026-04-13T08:00:00+09:00").unwrap(),
    )
    .unwrap();
    let term = &view.by_term["2026-1Q"];
    assert_eq!(term.pending_ids, vec!["soon", "later"]);
    assert_eq!(term.pinned_ids, vec!["later"]);
    assert_eq!(term.overdue_ids, vec!["past"]);
    assert_eq!(term.due_this_week_ids, vec!["soon"]);
    assert_eq!(term.earlier_due_this_week_ids, vec!["soon", "done"]);
    assert_eq!(term.lecture_open_ids, vec!["soon", "later", "undated"]);
    assert_eq!(term.lecture_hidden_ids, vec!["hidden"]);
    assert_eq!(view.nearest_by_course["course"], "soon");
    assert_eq!(view.deadlines["soon"].countdown_label, "2h");
    assert_eq!(view.deadlines["soon"].countdown_tone, "urgent");
    assert!(!view.deadlines.contains_key("invalid"));
}

#[test]
fn ics_folds_utf8_and_rolls_overnight() {
    let mut data = workspace();
    data["plans"] = json!([{"id":"plan","title":"日本語".repeat(30),"start":"2026-04-13T23:30:00+09:00","duration":90,"exdates":[]}]);
    let ics = export_quarter_ics(
        &data,
        "2026-1Q",
        CalendarExportScope::CoursesAndPlans,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert!(ics.contains("DTEND;TZID=Asia/Tokyo:20260414T010000"));
    assert!(ics.split("\r\n").all(|line| line.len() <= 75));
}

#[test]
fn query_wire_is_camel_case_and_tagged() {
    let request:WorkspaceQuery=serde_json::from_value(json!({"kind":"dayAgenda","term":"2026-1Q","date":"2026-04-13","options":{"includePersonal":false}})).unwrap();
    let result = query(&workspace(), request).unwrap();
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire["kind"], "dayAgenda");
    assert!(wire.get("value").unwrap().is_array());
}

#[test]
fn matches_committed_typescript_schedule_trace() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/typescript.json")).unwrap();
    let workspace = &fixture["workspace"];
    let expected = &fixture["expected"];
    assert_eq!(
        serde_json::to_value(super::calendar::quarter_range(workspace, "2026-1Q").unwrap())
            .unwrap(),
        expected["range"]
    );
    assert_eq!(
        serde_json::to_value(
            occurrences_on_date(workspace, "2026-1Q", "2026-04-13", AgendaOptions::default())
                .unwrap()
        )
        .unwrap(),
        expected["holidayOccurrences"]
    );
    assert_eq!(
        serde_json::to_value(
            resolve_day_agenda(workspace, "2026-1Q", "2026-04-20", AgendaOptions::default())
                .unwrap()
        )
        .unwrap(),
        expected["ordinaryAgenda"]
    );
    assert_eq!(display_color(&workspace["courses"][0]), expected["color"]);
    assert_eq!(
        declared_term_credits(workspace["registrations"].as_array().unwrap(), "2026-1Q").unwrap(),
        expected["credits"].as_f64()
    );
    assert_eq!(
        serde_json::to_value(next_plan_occurrences(&workspace["plans"][0], 5).unwrap()).unwrap(),
        expected["nextPlanOccurrences"]
    );
    assert_eq!(
        serde_json::to_value(academic_year_calendar(workspace, 2026).unwrap()).unwrap(),
        expected["academicYear"]
    );
    let ics = export_quarter_ics(
        workspace,
        "2026-1Q",
        CalendarExportScope::CoursesAndPlans,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert_eq!(ics, expected["ics"]);
}

struct NoopSink;
impl NotificationSink for NoopSink {
    fn deliver<'a>(
        &'a self,
        _item: NotificationItem,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotificationSinkError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

struct FailingSink;
impl NotificationSink for FailingSink {
    fn deliver<'a>(
        &'a self,
        _item: NotificationItem,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotificationSinkError>> + Send + 'a>> {
        Box::pin(async { Err(NotificationSinkError::Delivery("offline".into())) })
    }
}

#[test]
fn fired_log_is_durable_and_prunes_old_entries() {
    let root = std::env::temp_dir().join(format!("isct-reminders-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("fired.json");
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:40:00+09:00").unwrap();
    std::fs::write(&path, br#"{"old":0}"#).unwrap();
    assert!(super::reminders::claim_fired(&path, "class:one", now).unwrap());
    assert!(!super::reminders::claim_fired(&path, "class:one", now).unwrap());
    let stored: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(stored.get("old").is_none());
    assert!(stored.get("class:one").is_some());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test(start_paused = true)]
async fn failed_delivery_rolls_back_claim_and_can_retry_after_next_wake() {
    let root = std::env::temp_dir().join(format!("isct-reminders-retry-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("fired.json");
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:40:00+09:00").unwrap();
    let item = NotificationItem {
        key: "assignment:one".into(),
        at: now.to_rfc3339(),
        title: "Due".into(),
        body: "Soon".into(),
        deep_link: "/courses/one".into(),
        priority: NotificationPriority::Deadline,
    };
    let result = super::reminders::deliver_items(
        path.clone(),
        Arc::new(FailingSink),
        vec![item.clone()],
        now,
    )
    .await;
    assert!(matches!(result, Err(ReminderError::Delivery(_))));
    assert!(!super::reminders::read_fired(&path)
        .unwrap()
        .contains_key(&item.key));

    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    assert!(super::reminders::deliver_items(
        path.clone(),
        Arc::new(NoopSink),
        vec![item.clone()],
        now,
    )
    .await
    .unwrap());
    assert!(super::reminders::read_fired(&path)
        .unwrap()
        .contains_key(&item.key));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn legacy_fired_history_is_merged_before_delivery_starts() {
    let root = std::env::temp_dir().join(format!("isct-reminders-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = crate::data::DataStore::initialize(root.join("workspace.db"))
        .unwrap()
        .0;
    let service = ReminderService::new(store.clone());
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:40:00+09:00").unwrap();

    assert!(!service.tick(Arc::new(NoopSink), now).await.unwrap());
    assert!(service
        .import_legacy_fired(json!({"assignment:one": now.timestamp_millis()}), now)
        .await
        .unwrap());
    assert!(!service
        .import_legacy_fired(json!({"assignment:one": now.timestamp_millis() - 1}), now)
        .await
        .unwrap());
    assert!(
        super::reminders::read_fired(&super::reminders::fired_path(store.path()))
            .unwrap()
            .contains_key("assignment:one")
    );
    service.shutdown().await;
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn reminder_service_owns_shutdown_and_permission_gate() {
    let root = std::env::temp_dir().join(format!("isct-reminder-service-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = crate::data::DataStore::initialize(root.join("workspace.db"))
        .unwrap()
        .0;
    let service = ReminderService::new(store);
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:40:00+09:00").unwrap();
    assert!(!service.tick(Arc::new(NoopSink), now).await.unwrap());
    service.shutdown().await;
    assert!(matches!(
        service.tick(Arc::new(NoopSink), now).await,
        Err(ReminderError::Task(
            crate::runtime_tasks::RuntimeTaskError::Stopped
        ))
    ));
    std::fs::remove_dir_all(root).unwrap();
}
