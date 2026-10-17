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
    assert_eq!(
        friday.calendar_notice.as_deref(),
        Some("No classes due to Campus Festival")
    );
    assert!(
        occurrences_on_date(&data, "2026-3Q", "2026-10-09", AgendaOptions::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn period_backed_meetings_follow_profile_clock_while_custom_clocks_stay_explicit() {
    let mut data = workspace();
    let mut table = default_teaching_periods();
    table[2].start_time = "13:00".into();
    data["academicProfile"]["teachingPeriods"] = serde_json::to_value(table).unwrap();
    data["courses"][0]["meetings"] = json!([
        {"days":[1],"periods":[5,6,7,8],"startTime":"13:30","endTime":"17:05","room":"M-101"},
        {"days":[1],"periods":[],"startTime":"07:30","endTime":"09:00","room":"Custom"}
    ]);

    let template = planning_week(&data, "2026-1Q").unwrap();
    assert_eq!(template.days[0].blocks[0].start_time, "07:30");
    assert_eq!(template.days[0].blocks[1].start_time, "13:00");
    assert_eq!(template.days[0].blocks[1].end_time, "17:05");
}

#[test]
fn teaching_period_query_returns_resolved_default_and_profile_override() {
    let mut data = workspace();
    let WorkspaceQueryResult::TeachingPeriods(defaults) =
        query(&data, WorkspaceQuery::TeachingPeriods).unwrap()
    else {
        panic!("expected teaching periods")
    };
    assert_eq!(defaults, default_teaching_periods());

    let mut configured = defaults;
    configured[5].end_time = "21:00".into();
    data["academicProfile"]["teachingPeriods"] = serde_json::to_value(&configured).unwrap();
    let WorkspaceQueryResult::TeachingPeriods(resolved) =
        query(&data, WorkspaceQuery::TeachingPeriods).unwrap()
    else {
        panic!("expected teaching periods")
    };
    assert_eq!(resolved, configured);
}

#[test]
fn one_bad_stored_record_is_reported_without_hiding_the_rest_of_the_day() {
    let mut data = workspace();
    data["plans"] = json!([
        {"id":"good","title":"Study","start":"2026-04-13T18:00:00+09:00","duration":60,"rrule":"FREQ=DAILY;UNTIL=20260420","exdates":[]},
        {"id":"bad-rule","title":"Broken","start":"2026-04-13T19:00:00+09:00","duration":60,"rrule":"FREQ=DAILY;INTERVAL=garbage","exdates":[]},
        {"id":"bad-start","title":"No start","start":"tomorrow","duration":60,"exdates":[]}
    ]);
    data["scheduleOverrides"] = json!([
        {"id":"descending","targetKey":"course-weekly-0-2026-04-13","targetKind":"course","periods":[5,3],"room":"Elsewhere","hidden":false},
        {"id":"wrapped","targetKey":"elsewhere","targetKind":"course","periods":[257,258],"hidden":false}
    ]);
    let day = resolve_day_agenda(&data, "2026-1Q", "2026-04-13", AgendaOptions::default()).unwrap();
    assert_eq!(
        day.iter()
            .map(|block| block.id.as_str())
            .collect::<Vec<_>>(),
        ["course", "good"]
    );
    // The class stays as scheduled instead of disappearing.
    assert_eq!(day[0].start_time, "08:50");
    assert_eq!(day[0].room, "M-101");
    assert_eq!(day[0].overridden, None);
    let problems = schedule_problems(&data);
    assert_eq!(
        problems
            .iter()
            .map(|problem| (problem.kind, problem.id.as_str()))
            .collect::<Vec<_>>(),
        [
            (ScheduleProblemKind::Plan, "bad-rule"),
            (ScheduleProblemKind::Plan, "bad-start"),
            (ScheduleProblemKind::ScheduleOverride, "descending"),
            (ScheduleProblemKind::ScheduleOverride, "wrapped"),
        ]
    );
}

#[test]
fn malformed_period_meeting_is_reported_without_hiding_a_good_course() {
    let mut data = workspace();
    data["courses"].as_array_mut().unwrap().push(json!({
        "id":"bad-course", "termId":"2026-1Q", "title":"Broken course",
        "code":"BAD.1", "color":"#000000",
        "meetings":[{"days":[1],"periods":[257,258],"startTime":"09:00","endTime":"10:00","room":""}]
    }));
    data["memberships"].as_array_mut().unwrap().push(json!({
        "id":"bad-member", "termId":"2026-1Q", "courseId":"bad-course",
        "status":"registered", "evidence":{"provider":"school"}
    }));

    let day =
        occurrences_on_date(&data, "2026-1Q", "2026-04-13", AgendaOptions::default()).unwrap();
    assert_eq!(day.len(), 1);
    assert_eq!(day[0].id, "course");
    assert!(schedule_problems(&data).iter().any(|problem| {
        problem.kind == ScheduleProblemKind::CourseMeeting && problem.id == "bad-course:0"
    }));
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
    let WorkspaceQueryResult::WorkspaceOverview(future) = query(
        &data,
        WorkspaceQuery::WorkspaceOverview {
            term: "2027-1Q".into(),
        },
    )
    .unwrap() else {
        panic!("expected explicit quarter overview")
    };
    assert_eq!(future.default_finder_keys, vec!["1Q", "情報工学系"]);
    assert_eq!(future.courses_term, "2026-1Q");
    assert_eq!(future.term, "2027-1Q");
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
        intensive_days(
            &default_teaching_periods(),
            "2026-04-12",
            "2026-04-14",
            [3, 4]
        )
        .unwrap()
        .len(),
        3
    );
    assert!(intensive_days(
        &default_teaching_periods(),
        "2026-04-14",
        "2026-04-12",
        [3, 4]
    )
    .is_err());
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
    assert!(intensive_days(
        &default_teaching_periods(),
        "2026-04-01",
        "2026-04-02",
        [0, 10]
    )
    .is_err());
    assert!(intensive_days(
        &default_teaching_periods(),
        "2026-04-01",
        "2026-04-02",
        [4, 3]
    )
    .is_err());

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
    // Declared credits are shared across every quarter the offering names.
    let pair = json!([
      {"kind":"declared","offeringId":"202600001","quarters":[1,2],"effectiveCredits":2.0},
      {"kind":"declared","offeringId":"202600002","quarters":[1],"effectiveCredits":1.5}
    ]);
    assert_eq!(
        declared_term_credits(pair.as_array().unwrap(), "2026-1Q").unwrap(),
        Some(2.5)
    );
    // Fractions, historical spans and failed/pending results must not be
    // replaced by declaration totals or by competency points.
    let grades = json!([
        {"completed":"2025前学期","quarter":"1","credits":"0.5-0.5-0","result":"合格"},
        {"completed":"2025後学期","quarter":"3-4","credits":"1-0.5-0","effectiveCredits":1.5,"result":"75"},
        {"completed":"2026前学期","quarter":"1-4","credits":"4-0-0","result":"認定"},
        {"completed":"2024後学期","quarter":"3","credits":"2-0-0","result":"不合格"},
        {"completed":"2026後学期","quarter":"3","credits":"2-0-0","result":"未報告"},
        {"completed":"2023後学期","quarter":"3","credits":"2-0-0","result":"合格","missingSince":"x"}
    ]);
    let ledger = credits::earned_credits_by_term(grades.as_array().unwrap());
    assert_eq!(ledger["2025-1Q"], 1.0);
    assert_eq!(ledger["2025-3Q"], 0.75);
    assert_eq!(ledger["2025-4Q"], 0.75);
    assert_eq!(ledger["2026-2Q"], 1.0);
    assert_eq!(ledger["2026-3Q"], 1.0);
    assert_eq!(ledger["2024-3Q"], 0.0);
    assert!(!ledger.contains_key("2023-3Q"));
    assert_eq!(
        declared_term_credits(pair.as_array().unwrap(), "2026-3Q").unwrap(),
        None
    );
    assert_eq!(
        display_color(
            &json!({"id":"c","code":"CSC.T262","color":"#526e91","overrides":{"color":"#806a9d"}})
        ),
        "#806a9d"
    );
    // Class suffixes and year brackets name the same course, so they share a color.
    assert_eq!(
        display_color(&json!({"id":"x","code":"CSC.T241-01","color":"#526e91"})),
        display_color(&json!({"id":"other","code":"CSC.T241[2027]","color":"#556b8d"}))
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
        {"id":"past","courseId":"course","title":"Past","dueAt":"2026-04-12T10:00:00+09:00","pinned":true},
        {"id":"past-done","courseId":"course","title":"Past done","dueAt":"2026-04-11T10:00:00+09:00","submission":"graded","pinned":true},
        {"id":"done","courseId":"course","title":"Done","dueAt":"2026-04-14T10:00:00+09:00","submission":"submitted"},
        {"id":"undated-done","courseId":"course","title":"Graded without date","dueAt":null,"submission":"graded"},
        {"id":"hidden","courseId":"course","title":"Hidden","dueAt":null,"hidden":true},
        {"id":"invalid","courseId":"course","title":"Invalid","dueAt":"yesterday"},
        {"id":"undated","courseId":"course","title":"Undated","dueAt":null},
        {"id":"lms-done","courseId":"course","title":"LMS done","dueAt":"2026-04-15T10:00:00+09:00","submissionStatus":"submitted"},
        {"id":"graded","courseId":"course","title":"Graded","dueAt":"2026-04-01T10:00:00+09:00","submission":"graded","deletedAt":"2026-04-02T00:00:00Z"}
    ]);
    let view = task_view(
        &data,
        DateTime::parse_from_rfc3339("2026-04-13T08:00:00+09:00").unwrap(),
        false,
    )
    .unwrap();
    let term = &view.by_term["2026-1Q"];
    assert_eq!(term.pending_ids, vec!["soon", "later"]);
    assert_eq!(term.pinned_ids, vec!["later"]);
    assert_eq!(term.overdue_ids, vec!["past"]);
    assert_eq!(term.due_this_week_ids, vec!["soon"]);
    let completed = task_view(
        &data,
        DateTime::parse_from_rfc3339("2026-04-13T08:00:00+09:00").unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(
        completed.by_term["2026-1Q"].due_this_week_ids,
        vec!["soon", "done", "lms-done"]
    );
    assert_eq!(
        completed.by_term["2026-1Q"].overdue_ids,
        vec!["past", "past-done"]
    );
    assert_eq!(
        completed.by_term["2026-1Q"].undated_completed_ids,
        Some(vec!["undated-done".to_string()])
    );
    assert!(term.undated_completed_ids.is_none());
    assert!(!completed.deadlines.contains_key("undated-done"));
    // One submitted rule for every shell, including the LMS status field.
    assert_eq!(
        view.submitted_ids,
        vec!["done", "lms-done", "past-done", "undated-done"]
    );
    assert_eq!(term.lecture_open_ids, vec!["soon", "later", "undated"]);
    assert_eq!(term.lecture_hidden_ids, vec!["hidden"]);
    assert_eq!(view.nearest_by_course["course"], "soon");
    assert_eq!(view.deadlines["soon"].countdown_label, "2h");
    assert_eq!(view.deadlines["soon"].countdown_tone, "urgent");
    assert!(!view.deadlines.contains_key("invalid"));
}

/// A 3Q–4Q course added in 3Q: one course row (termId 3Q) with a plan item in
/// each quarter, while the calendar has moved on to 4Q.
pub(crate) fn multi_quarter_workspace() -> Value {
    let mut data = workspace();
    data["currentTerm"] = json!("2026-4Q");
    data["selectedTerm"] = json!("2026-3Q");
    data["courses"] = json!([{"id":"multi","termId":"2026-3Q","title":"Long Course","code":"MTH.B301","color":"#526e91",
        "meetings":[{"days":[1],"periods":[1,2],"startTime":"08:50","endTime":"10:30","room":"W-1"}]}]);
    data["memberships"] = json!([]);
    data["planItems"] = json!([
        {"id":"plan-3q","termId":"2026-3Q","courseId":"multi","offeringId":"202630001","state":"planned"},
        {"id":"plan-4q","termId":"2026-4Q","courseId":"multi","offeringId":"202630001","state":"planned"}
    ]);
    data["academicCalendar"] = json!([
        {"id":"q3s","academicYear":2026,"quarter":"3Q","date":"2026-10-01","type":"quarter-start","label":{"en":"3Q"}},
        {"id":"q3e","academicYear":2026,"quarter":"3Q","date":"2026-11-25","type":"quarter-end","label":{"en":"3Q end"}},
        {"id":"q4s","academicYear":2026,"quarter":"4Q","date":"2026-12-01","type":"quarter-start","label":{"en":"4Q"}},
        {"id":"q4e","academicYear":2026,"quarter":"4Q","date":"2027-02-05","type":"quarter-end","label":{"en":"4Q end"}}
    ]);
    data["assignments"] = json!([{"id":"report","courseId":"multi","title":"Report","dueAt":"2026-12-10T23:59:00+09:00"}]);
    data
}

#[test]
fn a_three_q_four_q_course_belongs_to_every_quarter_it_is_placed_in() {
    let data = multi_quarter_workspace();
    let at = |raw: &str| DateTime::parse_from_rfc3339(raw).unwrap();
    assert_eq!(scheduled_courses(&data, "2026-4Q")[0]["id"], "multi");
    assert_eq!(
        course_terms(&data, &data["courses"][0]),
        vec!["2026-3Q", "2026-4Q"]
    );

    // Timetable and Today: 4Q has the course, so Today follows 4Q.
    let WorkspaceQueryResult::WorkspaceOverview(overview) = query(
        &data,
        WorkspaceQuery::WorkspaceOverview {
            term: "2026-4Q".into(),
        },
    )
    .unwrap() else {
        panic!("overview expected")
    };
    assert_eq!(overview.today_term, "2026-4Q");
    assert_eq!(overview.courses[0]["id"], "multi");
    assert_eq!(overview.term_course_ids, vec!["multi"]);
    assert!(overview.all_terms.contains(&"2026-4Q".to_string()));
    let monday =
        resolve_day_agenda(&data, "2026-4Q", "2026-12-07", AgendaOptions::default()).unwrap();
    assert_eq!(monday[0].course_id.as_deref(), Some("multi"));
    let today = glance(&data, at("2026-12-07T07:00:00+09:00")).unwrap();
    assert_eq!(today.term, "2026-4Q");
    assert_eq!(today.agenda[0].course_id.as_deref(), Some("multi"));

    // Reminders and calendar export.
    let reminders = notification_plan(
        &data,
        at("2026-12-07T08:30:00+09:00"),
        at("2026-12-07T09:00:00+09:00"),
        Some(NotificationLocale::En),
    )
    .unwrap();
    assert!(reminders
        .items
        .iter()
        .any(|item| item.key == "class:multi-weekly-0-2026-12-07"));
    let ics = export_quarter_ics(
        &data,
        "2026-4Q",
        CalendarExportScope::Courses,
        at("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    assert!(ics.contains("UID:multi-weekly-0-2026-12-07@isctcompanion.local"));

    // Work for the course counts in 4Q, and its class times continue into 4Q.
    let tasks = task_view(&data, at("2026-12-07T07:00:00+09:00"), false).unwrap();
    assert_eq!(tasks.by_term["2026-4Q"].pending_ids, vec!["report"]);
    assert_eq!(tasks.by_term["2026-3Q"].pending_ids, vec!["report"]);
    let timeline =
        course_timeline(&data, "multi", at("2026-11-24T12:00:00+09:00"), 14, 14).unwrap();
    assert_eq!(
        timeline.last_started.as_deref(),
        Some("2026-11-23T08:50:00+09:00")
    );
    assert_eq!(timeline.next.as_deref(), Some("2026-12-07T08:50:00+09:00"));

    // Off the 4Q timetable, the course stays in 3Q only.
    let mut off = data.clone();
    off["planItems"][1]["deletedAt"] = json!("2026-12-01T00:00:00Z");
    assert!(scheduled_courses(&off, "2026-4Q").is_empty());
    let today = glance(&off, at("2026-12-07T07:00:00+09:00")).unwrap();
    assert_eq!(today.term, "2026-3Q");
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
    let fixture: Value =
        serde_json::from_str(include_str!("../schedule/fixtures/typescript.json")).unwrap();
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

#[tokio::test(start_paused = true)]
async fn delivery_starts_without_a_legacy_import_and_a_late_import_still_merges() {
    let root =
        std::env::temp_dir().join(format!("isct-reminders-no-import-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = crate::data::DataStore::initialize(root.join("workspace.db"))
        .unwrap()
        .0;
    let path = super::reminders::fired_path(store.path());
    let service = ReminderService::new(store);
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:40:00+09:00").unwrap();

    assert!(!service.tick(Arc::new(NoopSink), now).await.unwrap());
    assert!(!path.is_file());
    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    service.tick(Arc::new(NoopSink), now).await.unwrap();
    assert!(path.is_file());
    assert!(super::reminders::read_fired(&path).unwrap().is_empty());

    assert!(!service
        .import_legacy_fired(json!({"assignment:one": now.timestamp_millis()}), now)
        .await
        .unwrap());
    assert!(super::reminders::read_fired(&path)
        .unwrap()
        .contains_key("assignment:one"));
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

#[test]
fn assignment_completion_is_shared_by_mcp_tasks_and_reminders() {
    for row in [
        json!({"submission":"graded"}),
        json!({"submission":"submitted"}),
        json!({"submission":"unknown","submissionStatus":"submitted"}),
    ] {
        assert!(super::assignment_submitted(&row));
    }
    assert!(!super::assignment_submitted(&json!({"submission":"open"})));
}

#[test]
fn course_detail_projects_visible_rows_in_section_and_deadline_order() {
    let data = json!({
        "courses":[{"id":"c","code":" csc.t101 ","offeringId":"o"}],
        "offerings":[{"id":"o"}],
        "materials":[
            {"id":"later","courseId":"c","session":2},
            {"id":"first","courseId":"c","session":9,"sectionNumber":1},
            {"id":"same-section","courseId":"c","session":1},
            {"id":"deleted","courseId":"c","session":0,"deletedAt":"x"},
            {"id":"private","courseId":"c","session":0,"visible":false},
            {"id":"assignment-module","courseId":"c","module":"assign"},
            {"id":"quiz-module","courseId":"c","module":"quiz"},
            {"id":"other-course","courseId":"other","session":0}
        ],
        "assignments":[
            {"id":"no-date","courseId":"c","dueAt":null},
            {"id":"later","courseId":"c","dueAt":"2026-10-20T00:00:00Z"},
            {"id":"early","courseId":"c","dueAt":"2026-10-17T00:00:00Z"},
            {"id":"hidden","courseId":"c","hidden":true},
            {"id":"deleted","courseId":"c","deletedAt":"x"}
        ],
        "exams":[
            {"id":"late","courseCode":"CSC.T101","date":"2026-11-02","status":"scheduled"},
            {"id":"early","courseCode":"csc.t101 ","date":"2026-11-01","status":"scheduled"},
            {"id":"missing","courseCode":"CSC.T101","status":"scheduled","missingSince":"x"},
            {"id":"cancelled","courseCode":"CSC.T101","status":"cancelled"}
        ]
    });
    let request: WorkspaceQuery =
        serde_json::from_value(json!({"kind":"courseDetail","courseId":"c"})).unwrap();
    let WorkspaceQueryResult::CourseDetail(view) = query(&data, request).unwrap() else {
        panic!("wrong result kind")
    };
    assert_eq!(view.material_ids, ["first", "same-section", "later"]);
    assert_eq!(view.sections, [1, 2]);
    assert_eq!(view.assignment_ids, ["early", "later", "no-date"]);
    assert_eq!(view.exam_ids, ["early", "late"]);
    assert_eq!(view.offering_id.as_deref(), Some("o"));
    assert!(course_detail(&data, "unknown").material_ids.is_empty());

    // Links: the LMS record matched by offering, then the syllabus by offering id.
    assert_eq!(view.lms_url, None);
    assert_eq!(
        view.syllabus_url.as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses/-/-/-/o")
    );
    let mut linked = data.clone();
    linked["lmsCourses"] = json!([
        {"lmsId":7,"offeringId":"other","url":"https://lms.s.isct.ac.jp/2026/course/view.php?id=7"},
        {"lmsId":8,"offeringId":"o","url":"https://lms.s.isct.ac.jp/2026/course/view.php?id=8"}
    ]);
    linked["offerings"][0]["syllabusUrl"] = json!("https://syllabus.s.isct.ac.jp/courses/2026/x");
    let view = course_detail(&linked, "c");
    assert_eq!(
        view.lms_url.as_deref(),
        Some("https://lms.s.isct.ac.jp/2026/course/view.php?id=8")
    );
    assert_eq!(
        view.syllabus_url.as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses/2026/x")
    );
    // An LMS-sourced course links to itself and has no syllabus without an offering.
    let lms = json!({"courses":[{"id":"l","source":{"provider":"lms","externalId":"9"},"url":"https://lms.s.isct.ac.jp/2026/course/view.php?id=9"}]});
    let view = course_detail(&lms, "l");
    assert_eq!(
        view.lms_url.as_deref(),
        Some("https://lms.s.isct.ac.jp/2026/course/view.php?id=9")
    );
    assert_eq!(view.syllabus_url, None);
}

#[test]
fn query_request_contracts_reject_unstructured_inputs() {
    for request in [
        json!({"kind":"planCivilStart","plan":{}}),
        json!({"kind":"displayColor","course":{"color":42}}),
        json!({"kind":"nextPlanOccurrences","plan":{"start":"2026-10-16"}}),
        json!({"kind":"validateAcademicProfile","profile":[]}),
        json!({"kind":"academicTermForDate","date":"2026-10-16","profile":{"name":"missing years"}}),
    ] {
        assert!(serde_json::from_value::<WorkspaceQuery>(request).is_err());
    }
    let request = serde_json::from_value(
        json!({"kind":"planCivilStart","plan":{"start":"2026-10-16T09:00:00+09:00"}}),
    )
    .unwrap();
    let WorkspaceQueryResult::PlanCivilStart(value) = query(&json!({}), request).unwrap() else {
        panic!("wrong result kind")
    };
    assert_eq!(value.date, "2026-10-16");
    assert_eq!(value.time, "09:00");
}

#[test]
fn date_range_unifies_cross_quarter_view_and_ics_with_cancelled_context() {
    let mut data = workspace();
    data["academicCalendar"] = json!([
        {"id":"calendar:2026:quarter-start:3Q","academicYear":2026,"quarter":"3Q","date":"2026-09-28","type":"quarter-start"},
        {"id":"calendar:2026:quarter-end:3Q","academicYear":2026,"quarter":"3Q","date":"2026-12-02","type":"quarter-end"},
        {"id":"calendar:2026:quarter-start:4Q","academicYear":2026,"quarter":"4Q","date":"2026-12-03","type":"quarter-start"},
        {"id":"calendar:2026:quarter-end:4Q","academicYear":2026,"quarter":"4Q","date":"2027-02-08","type":"quarter-end"},
        {"id":"off","date":"2026-12-01","type":"holiday","label":{"en":"Rest day"}}
    ]);
    data["courses"][0]["termId"] = json!("2026-3Q");
    data["courses"][0]["meetings"] = json!([
        {"days":[1,2],"periods":[1,2]}, {"days":[3],"periods":[3,4]}
    ]);
    let mut fourth = data["courses"][0].clone();
    fourth["id"] = json!("fourth");
    fourth["termId"] = json!("2026-4Q");
    fourth["meetings"] = json!([{"days":[4,5],"periods":[1,2]}]);
    data["courses"].as_array_mut().unwrap().push(fourth);
    data["memberships"] = json!([
        {"id":"m3","courseId":"course","termId":"2026-3Q"},
        {"id":"m4","courseId":"fourth","termId":"2026-4Q"}
    ]);
    data["plans"] = json!([{"id":"daily","title":"Walk","start":"2026-11-30T18:00:00+09:00","duration":30,"rrule":"FREQ=DAILY;COUNT=7"}]);
    data["events"] = json!([
        {"id":"weekly3","termId":"2026-3Q","title":"Third-quarter event","days":[1],"startTime":"12:30","endTime":"13:00"},
        {"id":"weekly4","termId":"2026-4Q","title":"Fourth-quarter event","days":[1],"startTime":"12:30","endTime":"13:00"},
        {"id":"dated","termId":"2026-4Q","title":"Dated event","date":"2026-12-02","startTime":"12:30","endTime":"13:00"}
    ]);
    data["scheduleOverrides"] = json!([{"id":"move","targetKind":"course","targetKey":"course-weekly-1-2026-12-02","date":"2026-12-04","periods":[3,4]}]);
    let agendas = calendar_between(&data, "2026-11-30", "2026-12-06").unwrap();
    assert_eq!(agendas.len(), 7);
    assert!(agendas[0].occurrences.iter().any(|b| b.id == "course"));
    assert!(agendas[0].occurrences.iter().any(|b| b.id == "weekly3"));
    assert!(!agendas[0].occurrences.iter().any(|b| b.id == "weekly4"));
    assert_eq!(agendas[1].calendar_notice.as_deref(), Some("Rest day"));
    assert!(agendas[1].no_classes);
    assert!(agendas[1]
        .occurrences
        .iter()
        .any(|b| b.id == "course" && b.cancelled == Some(true)));
    assert!(!agendas[2].occurrences.iter().any(|b| b.id == "course"));
    assert!(agendas[2].occurrences.iter().any(|b| b.id == "dated"));
    assert!(agendas[3].occurrences.iter().any(|b| b.id == "fourth"));
    assert!(agendas[4]
        .occurrences
        .iter()
        .any(|b| b.id == "course" && b.overridden == Some(true)));
    for day in &agendas {
        assert_eq!(
            day.occurrences.iter().filter(|b| b.id == "daily").count(),
            1
        );
    }
    // Changing the selected quarter must never change a date's content.
    for term in ["2026-3Q", "2026-4Q"] {
        let WorkspaceQueryResult::ScheduleView(view) = query(
            &data,
            WorkspaceQuery::ScheduleView {
                term: term.into(),
                from: "2026-11-30".into(),
                to: "2026-12-06".into(),
                include_personal: true,
            },
        )
        .unwrap() else {
            panic!("view expected")
        };
        assert_eq!(
            serde_json::to_value(view.agendas).unwrap(),
            serde_json::to_value(&agendas).unwrap()
        );
    }
    let ics = export_calendar_ics(
        &data,
        "2026-11-30",
        "2026-12-06",
        CalendarExportScope::CoursesAndPlans,
        DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z").unwrap(),
    )
    .unwrap();
    let expected: Vec<_> = agendas
        .iter()
        .flat_map(|day| &day.occurrences)
        .filter(|b| {
            b.cancelled != Some(true)
                && matches!(b.kind, OccurrenceKind::Course | OccurrenceKind::Plan)
        })
        .collect();
    assert_eq!(ics.matches("BEGIN:VEVENT").count(), expected.len());
    for block in expected {
        assert!(ics.contains(&format!("UID:{}@isctcompanion.local", block.key)));
    }
    assert!(!ics.contains("UID:course-weekly-0-2026-12-01@"));
    // A quarter export uses the same range resolver, including an incoming move.
    let fourth_ics = export_quarter_ics(
        &data,
        "2026-4Q",
        CalendarExportScope::Courses,
        DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert!(fourth_ics.contains("UID:course-weekly-1-2026-12-02@"));
    assert!(!fourth_ics.contains("UID:course-weekly-0-2026-11-30@"));
    assert!(calendar_between(&data, "2026-12-06", "2026-11-30").is_err());
    assert!(calendar_between(&data, "2026-01-01", "2027-01-03").is_err());
}

#[test]
fn profile_form_preserves_identity_and_validates_or_resets_the_clock() {
    // An otherwise-default profile can change class times without inventing
    // grade labels or overriding the official quarter calendar.
    let mut profile = json!({"name":"Personal program","gradePath":[],"years":[],
        "studyEnrollments":[{"stage":"master","entryKind":"regular","entryGrade":1,"admittedTerm":"2026-1Q"}]});
    let mut table = default_teaching_periods();
    table[0].start_time = "07:30".into();
    profile["teachingPeriods"] = serde_json::to_value(&table).unwrap();
    let normalized = validate_academic_profile(&profile).unwrap();
    assert_eq!(normalized["studyEnrollments"], profile["studyEnrollments"]);
    let WorkspaceQueryResult::AcademicProfile(wire) = query(
        &workspace(),
        WorkspaceQuery::ValidateAcademicProfile {
            profile: serde_json::from_value(profile.clone()).unwrap(),
        },
    )
    .unwrap() else {
        panic!("profile expected")
    };
    assert_eq!(wire, normalized);
    let mut data = workspace();
    data["academicProfile"] = normalized;
    data["courses"][0]["meetings"][0]["periods"] = json!([1, 2]);
    assert_eq!(
        calendar_between(&data, "2026-04-13", "2026-04-13").unwrap()[0].occurrences[0].start_time,
        "07:30"
    );
    table[0].end_time = "11:00".into(); // overlaps 3–4
    profile["teachingPeriods"] = serde_json::to_value(&table).unwrap();
    assert!(validate_academic_profile(&profile).is_err());
    profile["teachingPeriods"] = Value::Null;
    data["academicProfile"] = validate_academic_profile(&profile).unwrap();
    assert_eq!(teaching_periods(&data).unwrap(), default_teaching_periods());
    profile["studyEnrollments"][0]["admittedTerm"] = json!("bad-term");
    assert!(validate_academic_profile(&profile).is_err());
}

#[test]
fn date_range_crosses_academic_years_and_merges_conflicts_once() {
    let mut data = workspace();
    data["academicCalendar"] = json!([
        {"id":"calendar:2026:quarter-start:4Q","academicYear":2026,"quarter":"4Q","date":"2026-12-03","type":"quarter-start"},
        {"id":"calendar:2026:quarter-end:4Q","academicYear":2026,"quarter":"4Q","date":"2027-03-31","type":"quarter-end"},
        {"id":"calendar:2027:quarter-start:1Q","academicYear":2027,"quarter":"1Q","date":"2027-04-01","type":"quarter-start"},
        {"id":"calendar:2027:quarter-end:1Q","academicYear":2027,"quarter":"1Q","date":"2027-06-09","type":"quarter-end"}
    ]);
    data["courses"][0]["termId"] = json!("2026-4Q");
    data["courses"][0]["meetings"][0]["days"] = json!([1, 4]);
    // One course is placed in both academic years; do not show its 4Q
    // template on Thursday before/after the actual quarter boundary.
    data["memberships"] = json!([
        {"id":"old","courseId":"course","termId":"2026-4Q"},
        {"id":"new","courseId":"course","termId":"2027-1Q"}
    ]);
    data["plans"] = json!([{"id":"plan","title":"Appointment","start":"2027-04-01T09:00:00+09:00","duration":30}]);
    let days = calendar_between(&data, "2027-03-29", "2027-04-04").unwrap();
    assert_eq!(days[0].occurrences.len(), 1);
    assert_eq!(days[3].occurrences.len(), 2);
    assert!(days[3].occurrences.iter().all(|b| b.conflict == Some(true)));
    // At an overlapping configured boundary, the same source occurrence and
    // its conflicts still appear only once.
    data["academicCalendar"][1]["date"] = json!("2027-04-02");
    let overlap = calendar_between(&data, "2027-04-01", "2027-04-01").unwrap();
    assert_eq!(overlap[0].occurrences.len(), 2);
}

#[test]
fn assignment_reminders_use_configured_times_and_reject_invalid_ones() {
    let mut data = workspace();
    data["settings"]["notifications"]["classes"] = json!(false);
    data["assignments"] =
        json!([{"id":"report","title":"Report","dueAt":"2026-04-14T18:00:00+09:00"}]);
    data["settings"]["notifications"]["assignmentLeadMinutes"] = json!([180, 30]);
    let from = DateTime::parse_from_rfc3339("2026-04-13T00:00:00+09:00").unwrap();
    let to = DateTime::parse_from_rfc3339("2026-04-14T18:00:00+09:00").unwrap();
    let plan = notification_plan(&data, from, to, Some(NotificationLocale::En)).unwrap();
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.items[0].at, "2026-04-14T15:00:00+09:00");
    assert_eq!(plan.items[0].body, "Due in 3 hours");
    assert_eq!(plan.items[1].at, "2026-04-14T17:30:00+09:00");
    assert_eq!(plan.items[1].body, "Due in 30 minutes");
    let ja = notification_plan(&data, from, to, Some(NotificationLocale::Ja)).unwrap();
    assert_eq!(ja.items[1].body, "締切まで30分");
    for invalid in [
        json!([]),
        json!([0]),
        json!([30, 30]),
        json!([10081]),
        json!([1.5]),
        json!(["60"]),
        json!([1, 2, 3, 4, 5, 6, 7, 8, 9]),
    ] {
        data["settings"]["notifications"]["assignmentLeadMinutes"] = invalid;
        assert!(notification_plan(&data, from, to, None).is_err());
    }
}

#[test]
fn contained_import_slots_do_not_make_a_course_conflict_with_itself() {
    let mut data = workspace();
    data["courses"][0]["meetings"] = json!([
        {"days":[1],"periods":[5,6,7,8],"room":"M-156"},
        {"days":[1],"periods":[7,8],"room":"M-156"},
        {"days":[4],"periods":[5,6,7,8],"room":"M-156"},
        {"days":[4],"periods":[5,6],"room":"M-156"}
    ]);
    let template = planning_week(&data, "2026-1Q").unwrap();
    assert_eq!(template.conflict_count, 0);
    assert_eq!(template.days[0].blocks.len(), 1);
    assert_eq!(template.days[3].blocks.len(), 1);
    let daily =
        occurrences_on_date(&data, "2026-1Q", "2026-04-13", AgendaOptions::default()).unwrap();
    assert_eq!(daily.len(), 1);
    assert_eq!(
        (&daily[0].start_time, &daily[0].end_time),
        (&"13:30".to_string(), &"17:05".to_string())
    );
    let mut other = data["courses"][0].clone();
    other["id"] = json!("other");
    other["title"] = json!("Other course");
    other["meetings"] = json!([{"days":[1],"periods":[7,8],"room":"M-156"}]);
    data["courses"].as_array_mut().unwrap().push(other);
    data["memberships"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"other-member","courseId":"other","termId":"2026-1Q"}));
    let clash = planning_week(&data, "2026-1Q").unwrap();
    assert_eq!(clash.conflict_count, 1);
    // Each side names the other, so a click on either block can explain it.
    let first = data["courses"][0]["id"].as_str().unwrap().to_string();
    let mut pairs: Vec<_> = clash
        .conflicts
        .iter()
        .map(|c| (c.course_id.clone(), c.with_course_id.clone(), c.day))
        .collect();
    pairs.sort();
    let mut expected = vec![
        (first.clone(), Some("other".to_string()), 1),
        ("other".to_string(), Some(first), 1),
    ];
    expected.sort();
    assert_eq!(pairs, expected);
    let title = data["courses"][0]["title"].as_str().unwrap().to_string();
    let named = |id: &str| {
        clash.days[0]
            .blocks
            .iter()
            .find(|b| b.course_id.as_deref() == Some(id))
            .and_then(|b| b.conflict_with.clone())
    };
    assert_eq!(named("other"), Some(vec![title]));
    assert_eq!(
        named(data["courses"][0]["id"].as_str().unwrap()),
        Some(vec!["Other course".to_string()])
    );
}
