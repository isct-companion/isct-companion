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
        "planItems":[], "events":[], "intensiveSchedules":[], "exceptions":[], "scheduleOverrides":[], "exams":[], "assignments":[], "registrations":[], "importantDates":[],
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
        &SchoolReads::fixture(&data),
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
    let WorkspaceQueryResult::TeachingPeriods(defaults) = query(
        &data,
        &SchoolReads::fixture(&data),
        WorkspaceQuery::TeachingPeriods,
    )
    .unwrap() else {
        panic!("expected teaching periods")
    };
    assert_eq!(defaults, default_teaching_periods());

    let mut configured = defaults;
    configured[5].end_time = "21:00".into();
    data["academicProfile"]["teachingPeriods"] = serde_json::to_value(&configured).unwrap();
    let WorkspaceQueryResult::TeachingPeriods(resolved) = query(
        &data,
        &SchoolReads::fixture(&data),
        WorkspaceQuery::TeachingPeriods,
    )
    .unwrap() else {
        panic!("expected teaching periods")
    };
    assert_eq!(resolved, configured);
}

#[test]
fn one_bad_stored_record_is_reported_without_hiding_the_rest_of_the_day() {
    let mut data = workspace();
    data["events"] = json!([
        {"id":"good","termId":"2026-1Q","title":"Study","room":"","days":[1],"startTime":"18:00","endTime":"19:00"}
    ]);
    data["scheduleOverrides"] = json!([
        {"id":"descending","targetKey":"course-weekly-0-2026-04-13","targetKind":"course","periods":[5,3],"room":"Elsewhere","hidden":false},
        {"id":"wrapped","targetKey":"elsewhere","targetKind":"course","periods":[257,258],"hidden":false}
    ]);
    let day = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-13",
        AgendaOptions::default(),
    )
    .unwrap();
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
        &SchoolReads::fixture(&data),
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
        &SchoolReads::fixture(&data),
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
fn weekly_events_survive_holidays_and_conflict() {
    let mut data = workspace();
    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"off","date":"2026-04-13","type":"holiday","label":{"en":"Off"}}));
    // "dated" is a one-date event synced from an older app: it has no
    // weekday, so it never occurs.
    data["events"] = json!([
        {"id":"lab","termId":"2026-1Q","days":[1],"title":"Lab","room":"","startTime":"09:00","endTime":"10:00"},
        {"id":"event","termId":"2026-1Q","days":[1,3],"title":"Event","room":"","startTime":"09:30","endTime":"10:00"},
        {"id":"dated","termId":"2026-1Q","date":"2026-04-13","title":"Dated","room":"","startTime":"09:00","endTime":"10:00"}
    ]);
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
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["lab", "event"]
    );
    assert!(rows.iter().all(|row| row.kind == OccurrenceKind::Event));
    assert!(rows.iter().all(|row| row.conflict == Some(true)));
}

#[test]
fn intensive_dates_are_strict() {
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
    // A weekly event keeps its own weekday; only classes follow the
    // replacement day.
    data["events"] = json!([{"id":"event","termId":"2026-1Q","days":[1],"title":"Personal","room":"","startTime":"09:00","endTime":"10:00"}]);
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

    data["events"] = json!([]);
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
    let rows = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-13",
        Default::default(),
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, OccurrenceKind::Exam);
    assert_eq!(rows[0].course_id.as_deref(), Some("course"));
    assert_eq!(rows[0].start_time, "10:45");
    assert_eq!(rows[0].room, "W9-101");

    data["exams"][0]["periods"] = json!([1, 2]);
    data["events"] = json!([{"id":"event","termId":"2026-1Q","title":"Interview","days":[1],"room":"Online","startTime":"08:50","endTime":"10:30"}]);
    let rows = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-13",
        Default::default(),
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .find(|row| row.kind == OccurrenceKind::Exam)
            .unwrap()
            .conflict_with
            .clone(),
        Some(vec!["Interview".to_string()])
    );
    // Overlapping blocks share the grid side by side, the exam drawn last.
    let lanes: Vec<_> = rows
        .iter()
        .map(|row| row.grid.as_ref().map(|slot| (slot.lane, slot.lanes)))
        .collect();
    assert_eq!(lanes, [Some((0, 2)), Some((1, 2))]);

    // An exam timed inside periods fills the periods it touches and keeps
    // its own times for the label.
    data["exams"][0]["periods"] = Value::Null;
    data["exams"][0]["startTime"] = json!("09:30");
    data["exams"][0]["endTime"] = json!("10:20");
    data["events"] = json!([]);
    let rows = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-13",
        Default::default(),
    )
    .unwrap();
    assert_eq!(rows[0].start_time, "09:30");
    assert_eq!(
        rows[0].grid,
        Some(GridSlot {
            start_time: "08:50".into(),
            end_time: "10:30".into(),
            lane: 0,
            lanes: 1,
        })
    );

    data["exams"] = json!([]);
    data["events"] = json!([]);
    data["scheduleOverrides"] = json!([{"id":"override","targetKey":"course-weekly-0-2026-04-13","targetKind":"course","date":"2026-04-14","periods":[5,6],"room":"Moved","hidden":false}]);
    assert!(resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-13",
        Default::default()
    )
    .unwrap()
    .is_empty());
    let moved = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-14",
        Default::default(),
    )
    .unwrap();
    assert_eq!(moved[0].room, "Moved");
    assert_eq!(moved[0].start_time, "13:30");
    assert_eq!(moved[0].overridden, Some(true));
}

#[test]
fn ics_and_cancelled_context_cover_desktop_boundaries() {
    let mut data = workspace();
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

    data["events"] = json!([{"id":"seminar","termId":"2026-1Q","days":[1],"title":"Weekly seminar","room":"","startTime":"18:00","endTime":"19:00"}]);
    data["academicCalendar"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"off","date":"2026-04-13","type":"holiday","label":{"en":"Off"}}));
    let standard = export_quarter_ics(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        CalendarExportScope::Courses,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    let expanded = export_quarter_ics(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        CalendarExportScope::CoursesAndEvents,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert!(!standard.contains("SUMMARY:Weekly seminar"));
    // Every Monday of the quarter, the holiday included.
    assert!(expanded.contains("UID:seminar-2026-04-13@isctcompanion.local"));
    assert!(expanded.contains("UID:seminar-2026-04-20@isctcompanion.local"));
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
        let rows=occurrences_on_date(&data,"2026-1Q","2026-04-13",AgendaOptions{include_cancelled_courses:true,events:Some(vec![json!({"id":"personal","termId":"2026-1Q","title":"Personal","days":[1],"room":"","startTime":"09:00","endTime":"10:00"})]),..Default::default()}).unwrap();
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
    let plan = notification_plan(
        &data,
        &SchoolReads::fixture(&data),
        now,
        horizon,
        Some(NotificationLocale::Ja),
    )
    .unwrap();
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
        &SchoolReads::fixture(data),
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
        {"id":"missed","courseId":"course","title":"Missed","dueAt":"2026-04-10T10:00:00+09:00","acceptsSubmission":true},
        {"id":"old","courseId":"course","title":"Old","dueAt":"2026-03-20T10:00:00+09:00","submission":"submitted"},
        {"id":"orphan","courseId":"gone","title":"Unlinked","dueAt":"2026-04-12T12:00:00+09:00"},
        {"id":"done","courseId":"course","title":"Done","dueAt":"2026-04-14T10:00:00+09:00","submission":"submitted"},
        {"id":"undated-done","courseId":"course","title":"Graded without date","dueAt":null,"submission":"graded"},
        {"id":"hidden","courseId":"course","title":"Hidden","dueAt":null,"hidden":true},
        {"id":"invalid","courseId":"course","title":"Invalid","dueAt":"yesterday"},
        {"id":"undated","courseId":"course","title":"Undated","dueAt":null},
        {"id":"lms-done","courseId":"course","title":"LMS done","dueAt":"2026-04-15T10:00:00+09:00","submissionStatus":"submitted"},
        {"id":"graded","courseId":"course","title":"Graded","dueAt":"2026-04-01T10:00:00+09:00","submission":"graded","deletedAt":"2026-04-02T00:00:00Z"}
    ]);
    let now = DateTime::parse_from_rfc3339("2026-04-13T08:00:00+09:00").unwrap();
    let view = task_view(&data, now).unwrap();
    let term = &view.by_term["2026-1Q"];
    assert_eq!(term.pending_ids, vec!["soon", "later"]);
    // Today: open work, pinned work until it is submitted (even overdue),
    // and the last week's misses, including unlinked work of this quarter.
    assert_eq!(term.today.pinned_ids, vec!["past", "later"]);
    assert_eq!(term.today.due_this_week_ids, vec!["soon"]);
    assert!(term.today.later_ids.is_empty() && term.today.undated_ids.is_empty());
    assert_eq!(term.today.past_recent_ids, vec!["orphan", "missed"]);
    assert!(term.today.past_earlier_ids.is_empty() && term.today.hidden_ids.is_empty());
    // Record: every assignment whatever its state; the current quarter folds
    // past work older than two weeks.
    assert_eq!(term.record.pinned_ids, vec!["later"]);
    assert_eq!(
        term.record.due_this_week_ids,
        vec!["soon", "done", "lms-done"]
    );
    assert_eq!(
        term.record.undated_ids,
        vec!["undated-done", "invalid", "undated"]
    );
    assert_eq!(
        term.record.past_recent_ids,
        vec!["orphan", "past", "past-done", "missed"]
    );
    assert_eq!(term.record.past_earlier_ids, vec!["old"]);
    assert_eq!(term.record.hidden_ids, vec!["hidden"]);
    assert!(!view.deadlines.contains_key("undated-done"));
    // One submitted rule for every shell, including the LMS status field.
    assert_eq!(
        view.submitted_ids,
        vec!["done", "lms-done", "old", "past-done", "undated-done"]
    );
    // Closed for hand-in: submitted work, and past deadlines unless LMS last
    // showed the page still taking late work ("missed").
    assert_eq!(
        view.closed_ids,
        vec![
            "done",
            "lms-done",
            "old",
            "orphan",
            "past",
            "past-done",
            "undated-done"
        ]
    );
    // Another quarter lists all of its past work at once; unlinked work
    // belongs only to the current quarter.
    let mut ended = data.clone();
    ended["currentTerm"] = json!("2026-2Q");
    let ended = task_view(&ended, now).unwrap();
    let ended = &ended.by_term["2026-1Q"].record;
    assert_eq!(
        ended.past_recent_ids,
        vec!["past", "past-done", "missed", "old"]
    );
    assert!(ended.past_earlier_ids.is_empty());
    assert_eq!(ended.hidden_ids, vec!["hidden"]);
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
        &SchoolReads::fixture(&data),
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
    let monday = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-4Q",
        "2026-12-07",
        AgendaOptions::default(),
    )
    .unwrap();
    assert_eq!(monday[0].course_id.as_deref(), Some("multi"));
    let today = glance(
        &data,
        &SchoolReads::fixture(&data),
        at("2026-12-07T07:00:00+09:00"),
    )
    .unwrap();
    assert_eq!(today.term, "2026-4Q");
    assert_eq!(today.agenda[0].course_id.as_deref(), Some("multi"));

    // Reminders and calendar export.
    let reminders = notification_plan(
        &data,
        &SchoolReads::fixture(&data),
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
        &SchoolReads::fixture(&data),
        "2026-4Q",
        CalendarExportScope::Courses,
        at("2026-12-01T00:00:00Z"),
    )
    .unwrap();
    assert!(ics.contains("UID:multi-weekly-0-2026-12-07@isctcompanion.local"));

    // Work for the course counts in 4Q, and its class times continue into 4Q.
    let tasks = task_view(&data, at("2026-12-07T07:00:00+09:00")).unwrap();
    assert_eq!(tasks.by_term["2026-4Q"].pending_ids, vec!["report"]);
    assert_eq!(tasks.by_term["2026-3Q"].pending_ids, vec!["report"]);
    let timeline = course_timeline(
        &data,
        &SchoolReads::fixture(&data),
        "multi",
        at("2026-11-24T12:00:00+09:00"),
        14,
        14,
    )
    .unwrap();
    assert_eq!(
        timeline.last_started.as_deref(),
        Some("2026-11-23T08:50:00+09:00")
    );
    assert_eq!(timeline.next.as_deref(), Some("2026-12-07T08:50:00+09:00"));

    // Off the 4Q timetable, the course stays in 3Q only.
    let mut off = data.clone();
    off["planItems"][1]["deletedAt"] = json!("2026-12-01T00:00:00Z");
    assert!(scheduled_courses(&off, "2026-4Q").is_empty());
    let today = glance(
        &off,
        &SchoolReads::fixture(&off),
        at("2026-12-07T07:00:00+09:00"),
    )
    .unwrap();
    assert_eq!(today.term, "2026-3Q");
}

/// A Monday with two classes, recent materials and assignments in every group.
fn phone_workspace() -> Value {
    let mut data = workspace();
    data["courses"].as_array_mut().unwrap().push(json!({"id":"afternoon","termId":"2026-1Q","title":"Afternoon","code":"CSC.T300","color":"#526e91","meetings":[{"days":[1],"startTime":"13:30","endTime":"15:10","room":"W9-325"}]}));
    data["memberships"].as_array_mut().unwrap().push(json!({"id":"member-2","termId":"2026-1Q","courseId":"afternoon","status":"registered","evidence":{"provider":"school"}}));
    data["materials"] = json!([
        {"id":"slides-1","courseId":"course","title":"Slides 1","type":"pdf","sectionNumber":1,"updatedAt":"2026-04-06T08:00:00+09:00"},
        {"id":"slides-2","courseId":"course","title":"Slides 2","type":"pdf","sectionNumber":2,"updatedAt":"2026-04-13T08:00:00+09:00"},
        {"id":"quiz","courseId":"course","title":"Quiz","type":"link","module":"quiz","sectionNumber":3,"updatedAt":"2026-04-13T08:00:00+09:00"}
    ]);
    data["assignments"] = json!([
        {"id":"late","courseId":"course","title":"Late","dueAt":"2026-04-11T23:59:00+09:00"},
        {"id":"ancient","courseId":"course","title":"Ancient","dueAt":"2026-03-01T23:59:00+09:00"},
        {"id":"tonight","courseId":"afternoon","title":"Tonight","dueAt":"2026-04-13T23:59:00+09:00"},
        {"id":"soon","courseId":"course","title":"Soon","dueAt":"2026-04-16T12:00:00+09:00"},
        {"id":"next-week","courseId":"afternoon","title":"Next week","dueAt":"2026-04-22T23:59:00+09:00"},
        {"id":"undated","courseId":"course","title":"Undated","dueAt":null},
        {"id":"pin","courseId":"course","title":"Pinned","dueAt":"2026-04-25T23:59:00+09:00","pinned":true},
        {"id":"done","courseId":"course","title":"Done","dueAt":"2026-04-14T23:59:00+09:00","submission":"submitted"},
        {"id":"graded","courseId":"afternoon","title":"Graded","dueAt":"2026-04-10T23:59:00+09:00","submission":"graded","grade":"8/10","updatedAt":"2026-04-12T09:00:00+09:00"},
        {"id":"gone","courseId":"course","title":"Gone","dueAt":"2026-04-15T23:59:00+09:00","hidden":true}
    ]);
    data
}

#[test]
fn today_moves_through_the_day_and_board_resolves_the_task_timeline() {
    let data = phone_workspace();
    let today = |raw: &str| {
        let WorkspaceQueryResult::Today(view) = query(
            &data,
            &SchoolReads::fixture(&data),
            serde_json::from_value(json!({"kind":"today","now":raw})).unwrap(),
        )
        .unwrap() else {
            panic!("today expected")
        };
        view
    };
    let ids = |rows: &[DeadlineItem]| -> Vec<String> {
        rows.iter().map(|row| row.assignment_id.clone()).collect()
    };

    let stack = |view: &TodayView| -> Vec<(String, String)> {
        view.upcoming
            .iter()
            .map(|item| {
                (
                    item.occurrence.date.clone(),
                    item.occurrence.course_id.clone().unwrap_or_default(),
                )
            })
            .collect()
    };

    // Before the first class: the stack leads with it, its period from the
    // table, and runs on into the next class day for three classes.
    let morning = today("2026-04-13T08:00:00+09:00");
    assert_eq!(morning.phase, TodayPhase::BeforeClasses);
    assert_eq!(morning.teaching_week, Some(2));
    assert!(morning.hero.is_none());
    assert_eq!(morning.upcoming[0].periods, Some([1, 2]));
    assert_eq!(
        stack(&morning),
        [
            ("2026-04-13".into(), "course".into()),
            ("2026-04-13".into(), "afternoon".into()),
            ("2026-04-20".into(), "course".into()),
        ]
    );
    assert_eq!(
        morning.next_boundary.as_deref(),
        Some("2026-04-13T08:50:00+09:00")
    );

    // In class: newest material and this course's open work, then today's other deadlines.
    let class = today("2026-04-13T09:00:00+09:00");
    assert_eq!(class.phase, TodayPhase::InClass);
    assert_eq!(class.class_materials[0].material_id, "slides-2");
    assert!(class.class_materials[0].added_today);
    // Without a section title, the row names the lesson by its number.
    assert_eq!(
        class.class_materials[0].section_label.as_deref(),
        Some("Lesson 2")
    );
    assert!(class
        .class_materials
        .iter()
        .all(|m| m.material_id != "quiz"));
    assert_eq!(ids(&class.class_assignments), ["soon"]);
    // The class in progress is the hero, never also the stack's first card.
    assert_eq!(
        class.hero.as_ref().unwrap().occurrence.course_id.as_deref(),
        Some("course")
    );
    assert_eq!(
        class.upcoming[0].occurrence.course_id.as_deref(),
        Some("afternoon")
    );
    assert_eq!(
        class.next_boundary.as_deref(),
        Some("2026-04-13T10:30:00+09:00")
    );

    let between = today("2026-04-13T11:00:00+09:00");
    assert_eq!(between.phase, TodayPhase::BetweenClasses);
    assert!(between.hero.is_none());
    assert_eq!(
        between.upcoming[0].occurrence.course_id.as_deref(),
        Some("afternoon")
    );

    // After classes: tonight's deadline leads, then the next class day.
    let evening = today("2026-04-13T21:10:00+09:00");
    assert_eq!(evening.phase, TodayPhase::AfterClasses);
    // The deadline shown leaves at its due time, before midnight's boundary.
    assert_eq!(
        evening.next_boundary.as_deref(),
        Some("2026-04-13T23:59:00+09:00")
    );
    assert_eq!(evening.deadline_hero.unwrap().assignment_id, "tonight");
    assert_eq!(evening.upcoming[0].occurrence.date, "2026-04-20");

    // A day without classes shows deadlines, recent changes and the next class.
    let mut off = data.clone();
    off["exceptions"] = json!([{"id":"off","date":"2026-04-13","type":"no-class","label":"Trip"}]);
    let WorkspaceQueryResult::Today(holiday) = query(
        &off,
        &SchoolReads::fixture(&off),
        WorkspaceQuery::Today {
            now: "2026-04-13T12:00:00+09:00".into(),
        },
    )
    .unwrap() else {
        panic!("today expected")
    };
    assert_eq!(holiday.phase, TodayPhase::NoClasses);
    assert!(holiday.no_classes);
    assert_eq!(holiday.calendar_notice.as_deref(), Some("Trip"));
    assert!(holiday.hero.is_none());
    // The FFI crosses as JSON, so the variant tag must survive a round trip.
    let crossed: TodayView =
        serde_json::from_value(serde_json::to_value(&holiday).unwrap()).unwrap();
    let recent: Vec<_> = crossed
        .recent
        .iter()
        .map(|item| match item {
            RecentItem::Grade(row) => row.assignment_id.clone(),
            RecentItem::Material(row) => row.material_id.clone(),
        })
        .collect();
    assert_eq!(recent, ["slides-2", "graded"]);
    assert_eq!(holiday.upcoming[0].occurrence.date, "2026-04-20");
    // An item opened since its last change is no longer new; one opened
    // before a newer change is new again.
    off["recentSeen"] = json!({
        "material:slides-2": "2026-04-13T08:00:00+09:00",
        "grade:graded": "2026-04-11T09:00:00+09:00",
    });
    let WorkspaceQueryResult::Today(seen) = query(
        &off,
        &SchoolReads::fixture(&off),
        WorkspaceQuery::Today {
            now: "2026-04-13T12:00:00+09:00".into(),
        },
    )
    .unwrap() else {
        panic!("today expected")
    };
    let recent: Vec<_> = seen
        .recent
        .iter()
        .map(|item| match item {
            RecentItem::Grade(row) => row.assignment_id.clone(),
            RecentItem::Material(row) => row.material_id.clone(),
        })
        .collect();
    assert_eq!(recent, ["graded"]);
    // Read all records what is new now under the keys and versions that
    // hide it, so nothing is new afterwards.
    let now = chrono::DateTime::parse_from_rfc3339("2026-04-13T12:00:00+09:00").unwrap();
    let unseen = crate::schedule::unseen_recent(&off, now);
    let keys: Vec<_> = unseen.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["grade:graded"]);
    for (key, version) in unseen {
        off["recentSeen"][key] = json!(version);
    }
    let WorkspaceQueryResult::Today(read) = query(
        &off,
        &SchoolReads::fixture(&off),
        WorkspaceQuery::Today {
            now: "2026-04-13T12:00:00+09:00".into(),
        },
    )
    .unwrap() else {
        panic!("today expected")
    };
    assert!(read.recent.is_empty());

    // The board resolves the quarter's one task timeline: what is left to do,
    // then every assignment whatever its state.
    let WorkspaceQueryResult::AssignmentBoard(board) = query(
        &data,
        &SchoolReads::fixture(&data),
        WorkspaceQuery::AssignmentBoard {
            now: "2026-04-13T12:00:00+09:00".into(),
            term: None,
        },
    )
    .unwrap() else {
        panic!("board expected")
    };
    assert_eq!(board.term, "2026-1Q");
    assert_eq!(ids(&board.today.pinned), ["pin"]);
    assert_eq!(ids(&board.today.due_this_week), ["tonight", "soon"]);
    assert_eq!(ids(&board.today.later), ["next-week"]);
    // Overdue for a week at most, hidden and submitted work never.
    assert_eq!(ids(&board.today.past_recent), ["late"]);
    assert_eq!(board.today.past_recent[0].tone, "past");
    assert_eq!(board.today.due_this_week[0].tone, "urgent");
    assert!(board.today.undated.is_empty() && board.today.hidden.is_empty());
    assert_eq!(
        ids(&board.record.due_this_week),
        ["tonight", "done", "soon"]
    );
    assert_eq!(ids(&board.record.undated), ["undated"]);
    assert_eq!(ids(&board.record.past_recent), ["late", "graded"]);
    assert_eq!(ids(&board.record.past_earlier), ["ancient"]);
    assert_eq!(ids(&board.record.hidden), ["gone"]);
    assert_eq!(board.record.past_recent[1].grade.as_deref(), Some("8/10"));
    assert!(board.record.past_recent[1].submitted);
    // Another quarter, asked by name, is read from the same timeline: none of
    // 1Q's work, and a malformed name is refused.
    let other = |term: &str| {
        query(
            &data,
            &SchoolReads::fixture(&data),
            WorkspaceQuery::AssignmentBoard {
                now: "2026-04-13T12:00:00+09:00".into(),
                term: Some(term.into()),
            },
        )
    };
    let Ok(WorkspaceQueryResult::AssignmentBoard(second)) = other("2026-2Q") else {
        panic!("board expected")
    };
    assert_eq!(second.term, "2026-2Q");
    assert!(second.record.due_this_week.is_empty() && second.record.past_recent.is_empty());
    assert!(other("spring").is_err());
}

#[test]
fn ics_folds_utf8() {
    let mut data = workspace();
    data["events"] = json!([{"id":"event","termId":"2026-1Q","days":[1],"title":"日本語".repeat(30),"room":"","startTime":"22:30","endTime":"23:30"}]);
    let ics = export_quarter_ics(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        CalendarExportScope::CoursesAndEvents,
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert!(ics.contains("DTEND;TZID=Asia/Tokyo:20260413T233000"));
    assert!(ics.split("\r\n").all(|line| line.len() <= 75));
}

#[test]
fn query_wire_is_camel_case_and_tagged() {
    let request:WorkspaceQuery=serde_json::from_value(json!({"kind":"dayAgenda","term":"2026-1Q","date":"2026-04-13","options":{"includePersonal":false}})).unwrap();
    let result = query(&workspace(), &SchoolReads::default(), request).unwrap();
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
            resolve_day_agenda(
                workspace,
                &SchoolReads::fixture(workspace),
                "2026-1Q",
                "2026-04-20",
                AgendaOptions::default()
            )
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
        serde_json::to_value(academic_year_calendar(workspace, 2026).unwrap()).unwrap(),
        expected["academicYear"]
    );
    let ics = export_quarter_ics(
        workspace,
        &SchoolReads::fixture(workspace),
        "2026-1Q",
        CalendarExportScope::Courses,
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
    let store = crate::data::DataStore::initialize_dir(root.clone())
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
        super::reminders::read_fired(&super::reminders::fired_path(store.user_path()))
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
    let store = crate::data::DataStore::initialize_dir(root.clone())
        .unwrap()
        .0;
    let path = super::reminders::fired_path(store.user_path());
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
async fn reminder_ticks_after_shutdown_are_rejected_as_stopped() {
    let root = std::env::temp_dir().join(format!("isct-reminder-service-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = crate::data::DataStore::initialize_dir(root.clone())
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
        "courses":[{"id":"c","code":" csc.t101 ","offeringId":"o","termId":"2026-3Q"}],
        "memberships":[{"courseId":"c","termId":"2026-3Q","status":"lms-member","evidence":{"provider":"lms"}}],
        "offerings":[{"id":"o","code":"LAS.A101","format":{"kinds":["lecture"]}}],
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
            {"id":"late","academicYear":2026,"quarter":"3Q","courseCode":"CSC.T101","date":"2026-11-02","status":"scheduled"},
            {"id":"early","academicYear":2026,"quarter":"3Q","courseCode":"csc.t101 ","date":"2026-11-01","status":"scheduled"},
            {"id":"missing","courseCode":"CSC.T101","status":"scheduled","missingSince":"x"},
            {"id":"cancelled","courseCode":"CSC.T101","status":"cancelled"}
        ]
    });
    let request: WorkspaceQuery =
        serde_json::from_value(json!({"kind":"courseDetail","courseId":"c"})).unwrap();
    let WorkspaceQueryResult::CourseDetail(view) =
        query(&data, &SchoolReads::fixture(&data), request).unwrap()
    else {
        panic!("wrong result kind")
    };
    assert_eq!(view.material_ids, ["first", "same-section", "later"]);
    assert_eq!(view.sections, [1, 2]);
    assert_eq!(view.assignment_ids, ["early", "later", "no-date"]);
    assert_eq!(view.exam_ids, ["early", "late"]);
    assert_eq!(view.format.unwrap().kinds, ["lecture"]);
    // The course's catalog code finds it too.
    assert_eq!(
        super::search::lecture_search(
            &data,
            &SchoolReads::fixture(&data),
            Some("2026-3Q"),
            "las.a101"
        )
        .course_ids,
        ["c"]
    );
    assert!(
        course_detail(&data, &SchoolReads::fixture(&data), "unknown")
            .material_ids
            .is_empty()
    );
    // Files go where desktop already put them: `folderComponent` replaces
    // every other UTF-16 unit, so an emoji takes two underscores.
    let files = view.files.unwrap();
    assert_eq!(
        (files.term.as_str(), files.course.as_str()),
        ("2026-3Q", "_csc.t101_")
    );
    assert!(
        course_detail(&data, &SchoolReads::fixture(&data), "unknown")
            .files
            .is_none()
    );
    assert_eq!(crate::storage::folder_name("線形代数🙂"), "______");
    assert_eq!(crate::storage::folder_name(".."), "course");
    assert_eq!(crate::storage::folder_name(&"a".repeat(130)).len(), 120);

    // Links: the LMS record matched by offering, then the syllabus page
    // Core builds from the offering id, in the content language.
    assert_eq!(view.lms_url, None);
    assert_eq!(
        view.syllabus_url.as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses/-/-/-/o?hl=en")
    );
    let mut japanese = data.clone();
    japanese["settings"] = json!({"contentLanguage":"ja"});
    assert_eq!(
        course_detail(&japanese, &SchoolReads::fixture(&japanese), "c")
            .syllabus_url
            .as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses/-/-/-/o")
    );
    let mut linked = data.clone();
    linked["lmsCourses"] = json!([
        {"lmsId":7,"offeringId":"other","url":"https://lms.s.isct.ac.jp/2026/course/view.php?id=7"},
        {"lmsId":8,"offeringId":"o","url":"https://lms.s.isct.ac.jp/2026/course/view.php?id=8"}
    ]);
    // A stored link of another form never reaches the shell.
    linked["offerings"][0]["syllabusUrl"] = json!("https://syllabus.s.isct.ac.jp/courses?jwc=o");
    let view = course_detail(&linked, &SchoolReads::fixture(&linked), "c");
    assert_eq!(
        view.lms_url.as_deref(),
        Some("https://lms.s.isct.ac.jp/2026/course/view.php?id=8")
    );
    assert_eq!(view.lms_course_id, Some(8));
    assert_eq!(
        view.syllabus_url.as_deref(),
        Some("https://syllabus.s.isct.ac.jp/courses/-/-/-/o?hl=en")
    );
    // Without an offering, the offering a シラバス link names (the 教務
    // `courses?jwc=` form included); other links are not syllabus pages.
    for (url, expected) in [
        (
            "https://syllabus.s.isct.ac.jp/courses?jwc=202602431",
            Some("https://syllabus.s.isct.ac.jp/courses/-/-/-/202602431?hl=en"),
        ),
        (
            "https://syllabus.s.isct.ac.jp/courses/2026/CSC.T263/202602427",
            Some("https://syllabus.s.isct.ac.jp/courses/-/-/-/202602427?hl=en"),
        ),
        ("https://example.invalid/courses?jwc=202602431", None),
    ] {
        let manual = json!({"courses":[{"id":"m","url":url}]});
        assert_eq!(
            course_detail(&manual, &SchoolReads::fixture(&manual), "m")
                .syllabus_url
                .as_deref(),
            expected,
            "{url}"
        );
    }
    // An LMS-sourced course links to itself and has no syllabus without an offering.
    let lms = json!({"courses":[{"id":"l","source":{"provider":"lms","externalId":"9"},"url":"https://lms.s.isct.ac.jp/2026/course/view.php?id=9"}]});
    let view = course_detail(&lms, &SchoolReads::fixture(&lms), "l");
    assert_eq!(
        view.lms_url.as_deref(),
        Some("https://lms.s.isct.ac.jp/2026/course/view.php?id=9")
    );
    assert_eq!(view.lms_course_id, Some(9));
    assert_eq!(view.syllabus_url, None);
}

#[test]
fn query_request_contracts_reject_unstructured_inputs() {
    for request in [
        json!({"kind":"displayColor","course":{"color":42}}),
        json!({"kind":"validateAcademicProfile","profile":[]}),
        json!({"kind":"academicTermForDate","date":"2026-10-16","profile":{"name":"missing years"}}),
    ] {
        assert!(serde_json::from_value::<WorkspaceQuery>(request).is_err());
    }
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
    let mut maybe = data["courses"][1].clone();
    maybe["id"] = json!("maybe");
    maybe["meetings"] = json!([{"days":[4],"periods":[5,6]}]);
    data["courses"].as_array_mut().unwrap().push(fourth);
    data["courses"].as_array_mut().unwrap().push(maybe);
    data["memberships"] = json!([
        {"id":"m3","courseId":"course","termId":"2026-3Q"},
        {"id":"m4","courseId":"fourth","termId":"2026-4Q"}
    ]);
    // Only planned for 4Q: on the timetable, without a membership there.
    data["planItems"] = json!([{"id":"p4","courseId":"maybe","termId":"2026-4Q"}]);
    data["events"] = json!([
        {"id":"weekly3","termId":"2026-3Q","title":"Third-quarter event","days":[1],"startTime":"12:30","endTime":"13:00"},
        {"id":"weekly4","termId":"2026-4Q","title":"Fourth-quarter event","days":[1],"startTime":"12:30","endTime":"13:00"},
        {"id":"dated","termId":"2026-4Q","title":"Dated event","date":"2026-12-02","startTime":"12:30","endTime":"13:00"}
    ]);
    data["scheduleOverrides"] = json!([{"id":"move","targetKind":"course","targetKey":"course-weekly-1-2026-12-02","date":"2026-12-04","periods":[3,4]}]);
    let agendas = calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2026-11-30",
        "2026-12-06",
    )
    .unwrap();
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
    // A one-date event synced from an older app has no weekday to repeat on.
    assert!(!agendas
        .iter()
        .any(|day| day.occurrences.iter().any(|b| b.id == "dated")));
    assert!(agendas[3].occurrences.iter().any(|b| b.id == "fourth"));
    // Planned is judged against each block's own quarter, so it holds
    // whichever quarter the view is asked from (the loop below).
    let planned = |day: &DatedAgenda, id: &str| {
        day.occurrences
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.planned)
    };
    assert_eq!(planned(&agendas[0], "course"), Some(None));
    assert_eq!(planned(&agendas[3], "fourth"), Some(None));
    assert_eq!(planned(&agendas[3], "maybe"), Some(Some(true)));
    assert!(agendas[4]
        .occurrences
        .iter()
        .any(|b| b.id == "course" && b.overridden == Some(true)));
    assert_eq!(
        agendas[0]
            .occurrences
            .iter()
            .filter(|b| b.id == "weekly3")
            .count(),
        1
    );
    // Changing the selected quarter must never change a date's content.
    for term in ["2026-3Q", "2026-4Q"] {
        let WorkspaceQueryResult::ScheduleView(view) = query(
            &data,
            &SchoolReads::fixture(&data),
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
        &SchoolReads::fixture(&data),
        "2026-11-30",
        "2026-12-06",
        CalendarExportScope::CoursesAndEvents,
        DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z").unwrap(),
    )
    .unwrap();
    let expected: Vec<_> = agendas
        .iter()
        .flat_map(|day| &day.occurrences)
        .filter(|b| {
            b.cancelled != Some(true)
                && matches!(b.kind, OccurrenceKind::Course | OccurrenceKind::Event)
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
        &SchoolReads::fixture(&data),
        "2026-4Q",
        CalendarExportScope::Courses,
        DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z").unwrap(),
    )
    .unwrap();
    assert!(fourth_ics.contains("UID:course-weekly-1-2026-12-02@"));
    assert!(!fourth_ics.contains("UID:course-weekly-0-2026-11-30@"));
    assert!(calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2026-12-06",
        "2026-11-30"
    )
    .is_err());
    assert!(calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2026-01-01",
        "2027-01-03"
    )
    .is_err());
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
        &SchoolReads::default(),
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
        calendar_between(
            &data,
            &SchoolReads::fixture(&data),
            "2026-04-13",
            "2026-04-13"
        )
        .unwrap()[0]
            .occurrences[0]
            .start_time,
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
    data["events"] = json!([{"id":"event","termId":"2027-1Q","days":[4],"title":"Appointment","room":"","startTime":"09:00","endTime":"09:30"}]);
    let days = calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2027-03-29",
        "2027-04-04",
    )
    .unwrap();
    assert_eq!(days[0].occurrences.len(), 1);
    assert_eq!(days[3].occurrences.len(), 2);
    assert!(days[3].occurrences.iter().all(|b| b.conflict == Some(true)));
    // At an overlapping configured boundary, the same source occurrence and
    // its conflicts still appear only once.
    data["academicCalendar"][1]["date"] = json!("2027-04-02");
    let overlap = calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2027-04-01",
        "2027-04-01",
    )
    .unwrap();
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
    let plan = notification_plan(
        &data,
        &SchoolReads::fixture(&data),
        from,
        to,
        Some(NotificationLocale::En),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 2);
    assert_eq!(plan.items[0].at, "2026-04-14T15:00:00+09:00");
    assert_eq!(plan.items[0].body, "Due in 3 hours");
    assert_eq!(plan.items[1].at, "2026-04-14T17:30:00+09:00");
    assert_eq!(plan.items[1].body, "Due in 30 minutes");
    let ja = notification_plan(
        &data,
        &SchoolReads::fixture(&data),
        from,
        to,
        Some(NotificationLocale::Ja),
    )
    .unwrap();
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
        assert!(notification_plan(&data, &SchoolReads::fixture(&data), from, to, None).is_err());
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

/// A weekly event is part of the quarter template: a class over it is a
/// conflict, and a quarter without events is offered the latest earlier
/// quarter's.
#[test]
fn weekly_events_clash_with_classes_and_carry_into_an_empty_quarter() {
    let mut data = workspace();
    data["courses"][0]["meetings"] = json!([{"days":[1],"periods":[7,8],"room":"M-156"}]);
    data["events"] = json!([
        {"id":"lab","termId":"2026-1Q","days":[1],"title":"Lab meeting","room":"","startTime":"15:00","endTime":"16:00"},
        {"id":"club","termId":"2026-1Q","days":[5],"title":"Club","room":"","startTime":"18:00","endTime":"19:00"},
        {"id":"old","termId":"2025-4Q","days":[2],"title":"Old seminar","room":"","startTime":"10:00","endTime":"11:00"}
    ]);
    let week = planning_week(&data, "2026-1Q").unwrap();
    assert_eq!(week.conflict_count, 1);
    let course = data["courses"][0]["id"].as_str().unwrap();
    assert_eq!(
        week.conflicts
            .iter()
            .map(|c| (
                c.course_id.as_str(),
                c.with_course_id.as_deref(),
                c.with_title.as_str()
            ))
            .collect::<Vec<_>>(),
        [(course, None, "Lab meeting")]
    );
    assert!(week.days[4].blocks.iter().all(|b| b.conflict != Some(true)));
    assert!(week.carry_over.is_none());

    let later = planning_week(&data, "2026-2Q").unwrap();
    let offer = later.carry_over.unwrap();
    assert_eq!((offer.term_id.as_str(), offer.count), ("2026-1Q", 2));
    // Nothing earlier than the oldest quarter with events.
    assert!(planning_week(&data, "2025-4Q")
        .unwrap()
        .carry_over
        .is_none());
    assert!(planning_week(&data, "2025-3Q")
        .unwrap()
        .carry_over
        .is_none());
}

#[test]
fn exam_lists_agendas_and_reminders_share_term_class_identity_and_personal_edits() {
    let mut data = workspace();
    data["exams"] = json!([
        {"id":"mine","sourceKey":"reference:mine","academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":"2026-04-14","periods":[1,2],"status":"scheduled"},
        {"id":"other-class","academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241-02","date":"2026-04-14","status":"scheduled"},
        {"id":"old-year","academicYear":2025,"quarter":"1Q","courseCode":"CSC.T241","date":"2025-04-14","status":"scheduled"},
        {"id":"stale","academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":"2026-04-14","status":"scheduled","missingSince":"x"}
    ]);
    let views = exam_views(&data, &SchoolReads::fixture(&data), "2026-1Q");
    assert_eq!(views.len(), 2);
    let mine = views.iter().find(|row| row.id == "mine").unwrap();
    assert_eq!(mine.course_id.as_deref(), Some("course"));
    assert_eq!(mine.start_time.as_deref(), Some("08:50"));
    let other = views.iter().find(|row| row.id == "other-class").unwrap();
    assert_eq!(other.course_id, None);
    assert_eq!(other.start_time, None); // Unannounced times still have a visible card.
    assert_eq!(
        exam_views(&data, &SchoolReads::fixture(&data), "2025-1Q")[0].course_id,
        None
    );
    assert_eq!(
        course_detail(&data, &SchoolReads::fixture(&data), "course").exam_ids,
        ["mine"]
    );

    data["scheduleOverrides"] = json!([{"id":"edit","targetKey":"reference:mine","targetKind":"exam","courseId":"course","date":"2026-10-19","startTime":"00:30","endTime":"01:30","room":"My room","hidden":false}]);
    data["currentTerm"] = json!("2026-3Q");
    data["settings"]["notifications"] =
        json!({"classes":false,"exams":true,"examLeadMinutes":1440,"assignments":false});
    let now = DateTime::parse_from_rfc3339("2026-10-18T00:20:00+09:00").unwrap();
    let horizon = now + chrono::Duration::minutes(20);
    let plan = notification_plan(&data, &SchoolReads::fixture(&data), now, horizon, None).unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].key, "class:reference:mine");
    assert!(matches!(plan.items[0].priority, NotificationPriority::Exam));
    assert!(plan.items[0].body.contains("My room"));
    let rows = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-10-19",
        Default::default(),
    )
    .unwrap();
    assert_eq!(rows[0].start_time, "00:30");
    assert_eq!(
        course_detail(&data, &SchoolReads::fixture(&data), "course").exams[0].room,
        "My room"
    );
    data["settings"]["notifications"]["exams"] = json!(false);
    assert!(
        notification_plan(&data, &SchoolReads::fixture(&data), now, horizon, None)
            .unwrap()
            .items
            .is_empty()
    );
    data["settings"]["notifications"]["exams"] = json!(true);
    // A deleted (unlinked) exam is no longer the student's: no reminder.
    data["scheduleOverrides"][0]["courseId"] = json!("");
    assert!(
        notification_plan(&data, &SchoolReads::fixture(&data), now, horizon, None)
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(
        exam_views(&data, &SchoolReads::fixture(&data), "2026-1Q")
            .iter()
            .find(|exam| exam.id == "mine")
            .unwrap()
            .course_id,
        None
    );
}

#[test]
fn exam_timetable_sessions_outrank_the_weekly_pattern_but_not_manual_edits() {
    let mut data = workspace();
    data["academicCalendar"][0]["examRanges"] = json!([{"start":"2026-05-28","end":"2026-06-09"}]);
    let row = |id: &str, kind: &str, date: &str, periods: [u8; 2]| json!({"id":id,"kind":kind,"sourceKey":format!("reference:{id}"),"academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":date,"periods":periods,"room":{"ja":"W8E-E207"},"label":{"ja":"授業"},"status":"scheduled"});
    data["exams"] = json!([
        // The weekly class meets Mondays 1-2.
        row("moved", "class", "2026-04-13", [3, 4]),
        row("extra", "class", "2026-04-15", [5, 6]),
        row("final", "exam", "2026-04-16", [3, 4]),
        row("kept", "class", "2026-04-20", [7, 8]),
        row("exam-week", "class", "2026-05-28", [1, 2]),
        row("monday-exam", "exam", "2026-04-27", [5, 6]),
        row("edited-monday-exam", "exam", "2026-05-11", [5, 6]),
    ]);
    // A hand-edited weekly session outranks the timetable.
    data["scheduleOverrides"] = json!([
        {"id":"mine","targetKey":"course-weekly-0-2026-04-20","targetKind":"course","room":"My room","hidden":false},
        {"id":"mine-too","targetKey":"course-weekly-0-2026-05-11","targetKind":"course","room":"My room","hidden":false}
    ]);
    let agenda = |date| {
        resolve_day_agenda(
            &data,
            &SchoolReads::fixture(&data),
            "2026-1Q",
            date,
            Default::default(),
        )
        .unwrap()
        .into_iter()
        .map(|block| (block.kind, block.key, block.start_time, block.room))
        .collect::<Vec<_>>()
    };
    let session = |key: &str, start: &str, room: &str| {
        (
            OccurrenceKind::Course,
            key.to_string(),
            start.to_string(),
            room.to_string(),
        )
    };
    assert_eq!(
        agenda("2026-04-13"),
        [session("reference:moved", "10:45", "W8E-E207")]
    );
    assert_eq!(
        agenda("2026-04-15"),
        [session("reference:extra", "13:30", "W8E-E207")]
    );
    assert_eq!(agenda("2026-04-16")[0].0, OccurrenceKind::Exam);
    assert_eq!(
        agenda("2026-04-20"),
        [
            session("course-weekly-0-2026-04-20", "08:50", "My room"),
            session("reference:kept", "15:25", "W8E-E207"),
        ]
    );

    // An exam replaces the weekly class too, unless the user edited it.
    let kinds = |date| {
        agenda(date)
            .into_iter()
            .map(|block| block.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds("2026-04-27"), [OccurrenceKind::Exam]);
    assert_eq!(
        kinds("2026-05-11"),
        [OccurrenceKind::Course, OccurrenceKind::Exam]
    );

    // The exam period is not a day without classes when something is on.
    let days = calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2026-05-28",
        "2026-05-29",
    )
    .unwrap();
    assert_eq!(
        days.iter()
            .map(|day| (day.no_classes, day.calendar_notice.as_deref()))
            .collect::<Vec<_>>(),
        [(false, Some("Exam period")), (true, Some("Exam period"))]
    );

    let views = exam_views(&data, &SchoolReads::fixture(&data), "2026-1Q");
    assert_eq!(views.iter().filter(|view| view.kind == "class").count(), 4);
    let detail = course_detail(&data, &SchoolReads::fixture(&data), "course");
    assert_eq!(detail.exams.len(), 7);
    assert_eq!(
        detail.exam_ids,
        ["final", "monday-exam", "edited-monday-exam"]
    );
    assert_eq!(
        super::exams::upcoming_exams(&data, &SchoolReads::fixture(&data), "2026-04-01")
            .into_iter()
            .map(|view| view.id)
            .collect::<Vec<_>>(),
        ["final", "monday-exam", "edited-monday-exam"]
    );
    assert!(exam_association(
        &data,
        &SchoolReads::fixture(&data),
        "reference:extra",
        "course"
    )
    .unwrap()
    .conflicts
    .iter()
    .all(|view| view.kind == "exam"));
}

#[test]
fn a_user_mark_decides_what_a_session_is() {
    let mut data = workspace();
    data["academicCalendar"][0]["examRanges"] = json!([{"start":"2026-05-28","end":"2026-06-09"}]);
    data["exams"] = json!([{"id":"final","kind":"exam","sourceKey":"reference:final","academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":"2026-04-23","periods":[3,4],"room":{"ja":"W8E-E207"},"label":{"ja":"期末"},"status":"scheduled"}]);
    let mark = |data: &mut Value, key: &str, mark: &str| {
        let row =
            super::exams::marked_session_override(data, &SchoolReads::fixture(data), key, mark)
                .unwrap();
        let rows = data["scheduleOverrides"].as_array_mut().unwrap();
        rows.retain(|existing| existing["targetKey"] != key);
        if let Some(mut row) = row {
            row["id"] = json!(key);
            rows.push(row);
        }
    };
    let agenda = |data: &Value, date| {
        resolve_day_agenda(
            data,
            &SchoolReads::fixture(data),
            "2026-1Q",
            date,
            Default::default(),
        )
        .unwrap()
        .into_iter()
        .map(|block| (block.kind, block.cancelled))
        .collect::<Vec<_>>()
    };
    let calendar = |data: &Value, date: &str| {
        calendar_between(data, &SchoolReads::fixture(data), date, date).unwrap()[0]
            .occurrences
            .iter()
            .map(|block| (block.kind, block.cancelled))
            .collect::<Vec<_>>()
    };
    // Weekly Monday 1-2: cancelled stays visible greyed in the calendar only.
    mark(&mut data, "course-weekly-0-2026-04-13", "cancelled");
    assert!(agenda(&data, "2026-04-13").is_empty());
    assert_eq!(
        calendar(&data, "2026-04-13"),
        [(OccurrenceKind::Course, Some(true))]
    );
    // A weekly session marked as an exam is an exam, and every exam list
    // carries it under the session's own key.
    mark(&mut data, "course-weekly-0-2026-04-20", "exam");
    assert_eq!(agenda(&data, "2026-04-20"), [(OccurrenceKind::Exam, None)]);
    let listed = |data: &Value| {
        exam_views(data, &SchoolReads::fixture(data), "2026-1Q")
            .into_iter()
            .map(|view| (view.key, view.kind, view.origin, view.course_id))
            .collect::<Vec<_>>()
    };
    let session = |key: &str, kind: &str| {
        (
            key.to_string(),
            kind.to_string(),
            "session".to_string(),
            Some("course".to_string()),
        )
    };
    assert!(listed(&data).contains(&session("course-weekly-0-2026-04-20", "exam")));
    assert_eq!(
        course_detail(&data, &SchoolReads::fixture(&data), "course")
            .exams
            .iter()
            .filter(|view| view.origin == "session")
            .count(),
        1
    );
    // A quiz is an exam block of its own kind; Today counts it unless the
    // user switched quizzes off.
    mark(&mut data, "course-weekly-0-2026-04-27", "quiz");
    let quiz = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-27",
        Default::default(),
    )
    .unwrap();
    assert_eq!(
        quiz.iter()
            .map(|block| (block.kind, block.quiz))
            .collect::<Vec<_>>(),
        [(OccurrenceKind::Exam, Some(true))]
    );
    assert!(listed(&data).contains(&session("course-weekly-0-2026-04-27", "quiz")));
    let upcoming = |data: &Value| {
        super::exams::upcoming_exams(data, &SchoolReads::fixture(data), "2026-04-01")
            .into_iter()
            .map(|view| view.key)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        upcoming(&data),
        [
            "course-weekly-0-2026-04-20",
            "reference:final",
            "course-weekly-0-2026-04-27"
        ]
    );
    data["settings"]["notifications"]["quizzes"] = json!(false);
    assert!(!upcoming(&data).contains(&"course-weekly-0-2026-04-27".to_string()));
    data["settings"]["notifications"]["quizzes"] = json!(true);
    // Clicking the checked mark again returns the session to its sources:
    // the record goes, so a later calendar change still reaches it.
    mark(&mut data, "course-weekly-0-2026-04-27", "class");
    assert!(!data["scheduleOverrides"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["targetKey"] == "course-weekly-0-2026-04-27"));
    assert_eq!(
        agenda(&data, "2026-04-27"),
        [(OccurrenceKind::Course, None)]
    );
    // A marked session moved to another day keeps its mark there.
    data["scheduleOverrides"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["targetKey"] == "course-weekly-0-2026-04-20")
        .unwrap()["date"] = json!("2026-04-21");
    assert_eq!(agenda(&data, "2026-04-21"), [(OccurrenceKind::Exam, None)]);
    assert!(exam_views(&data, &SchoolReads::fixture(&data), "2026-1Q")
        .iter()
        .any(|view| view.key == "course-weekly-0-2026-04-20"
            && view.date == "2026-04-21"
            && view.source.date == "2026-04-20"
            && view.edited));
    // Deleting the exam drops only the mark: the class stays on its new day,
    // and marking it again brings the exam back.
    let row = super::exams::removed_exam_override(
        &data,
        &SchoolReads::fixture(&data),
        "course-weekly-0-2026-04-20",
    )
    .unwrap();
    let rows = data["scheduleOverrides"].as_array_mut().unwrap();
    rows.retain(|existing| existing["targetKey"] != "course-weekly-0-2026-04-20");
    rows.extend(row.map(|mut row| {
        row["id"] = json!("course-weekly-0-2026-04-20");
        row
    }));
    assert_eq!(
        agenda(&data, "2026-04-21"),
        [(OccurrenceKind::Course, None)]
    );
    assert!(!listed(&data)
        .iter()
        .any(|row| row.0 == "course-weekly-0-2026-04-20"));
    assert!(super::exams::removed_exam_override(
        &data,
        &SchoolReads::fixture(&data),
        "course-weekly-0-2026-04-20"
    )
    .is_err());
    mark(&mut data, "course-weekly-0-2026-04-20", "exam");
    assert_eq!(agenda(&data, "2026-04-21"), [(OccurrenceKind::Exam, None)]);
    // The session gives way to the published exam of its course that day.
    data["exams"].as_array_mut().unwrap().push(json!({"id":"monday","kind":"exam","sourceKey":"reference:monday","academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":"2026-05-11","periods":[1,2],"room":{"ja":"W8E-E207"},"label":{"ja":"中間"},"status":"scheduled"}));
    mark(&mut data, "course-weekly-0-2026-05-11", "exam");
    assert_eq!(agenda(&data, "2026-05-11"), [(OccurrenceKind::Exam, None)]);
    assert!(!listed(&data)
        .iter()
        .any(|row| row.0 == "course-weekly-0-2026-05-11"));
    // A session the exam period cancels runs when the user says it does.
    assert!(agenda(&data, "2026-06-01").is_empty());
    mark(&mut data, "course-weekly-0-2026-06-01", "class");
    assert_eq!(
        agenda(&data, "2026-06-01"),
        [(OccurrenceKind::Course, None)]
    );
    assert!(data["scheduleOverrides"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["targetKey"] == "course-weekly-0-2026-06-01" && row["mark"] == "class"));
    let day = calendar_between(
        &data,
        &SchoolReads::fixture(&data),
        "2026-06-01",
        "2026-06-01",
    )
    .unwrap();
    assert!(!day[0].no_classes);

    // Exam-timetable rows: cancel, turn into a class, then back to the source.
    mark(&mut data, "reference:final", "cancelled");
    assert!(agenda(&data, "2026-04-23").is_empty());
    assert_eq!(
        calendar(&data, "2026-04-23"),
        [(OccurrenceKind::Exam, Some(true))]
    );
    assert!(!upcoming(&data).contains(&"reference:final".to_string()));
    mark(&mut data, "reference:final", "class");
    assert_eq!(
        agenda(&data, "2026-04-23"),
        [(OccurrenceKind::Course, None)]
    );
    mark(&mut data, "reference:final", "exam");
    assert!(!data["scheduleOverrides"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["targetKey"] == "reference:final"));
    assert_eq!(agenda(&data, "2026-04-23"), [(OccurrenceKind::Exam, None)]);
    assert!(super::exams::marked_session_override(
        &data,
        &SchoolReads::fixture(&data),
        "reference:final",
        "maybe"
    )
    .is_err());
}

#[test]
fn a_correction_survives_timetable_changes_and_flags_the_ones_it_hides() {
    let mut data = workspace();
    let row = |key: &str, date: &str| json!({"id":key,"kind":"exam","sourceKey":key,"academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":date,"periods":[3,4],"room":{"ja":"W8E-E207"},"label":{"ja":"期末"},"status":"scheduled"});
    data["exams"] = json!([row("reference:final", "2026-04-23")]);
    let save = |data: &mut Value, key: &str, change: Option<Value>| {
        let adopted = super::exams::adopted_exam_override(data, &SchoolReads::fixture(data), key);
        let rows = data["scheduleOverrides"].as_array_mut().unwrap();
        rows.retain(|existing| {
            existing["targetKey"] != key
                && Some(existing["id"].as_str().unwrap_or_default()) != adopted.as_deref()
        });
        if let Some(mut change) = change {
            change["id"] = json!(format!("override:{key}"));
            rows.push(change);
        }
    };
    let correct = |data: &mut Value, key: &str, date: &str| {
        let change = super::exams::corrected_exam_override(
            data,
            &SchoolReads::fixture(data),
            key,
            super::exams::ExamCorrection {
                date: Some(date.into()),
                ..Default::default()
            },
        )
        .unwrap();
        save(data, key, change);
    };
    let view = |data: &Value| {
        exam_views(data, &SchoolReads::fixture(data), "2026-1Q")
            .into_iter()
            .next()
            .unwrap()
    };
    // The student hears in class that the exam moved to the 24th.
    correct(&mut data, "reference:final", "2026-04-24");
    assert_eq!(view(&data).date, "2026-04-24");
    assert!(view(&data).conflict.is_none());
    // A refresh that leaves the date alone, or changes a field the student
    // did not correct, keeps the correction without a conflict.
    data["exams"][0]["room"] = json!({"ja":"W9-321"});
    assert_eq!(
        (view(&data).date, view(&data).room),
        ("2026-04-24".to_string(), "W9-321".to_string())
    );
    assert!(view(&data).conflict.is_none());
    // The office catches up: nothing to resolve.
    data["exams"][0]["date"] = json!("2026-04-24");
    assert!(view(&data).conflict.is_none());
    assert!(!view(&data).edited);
    // The office moves it again: the student's date stays, with a conflict
    // on the view and on the calendar block.
    data["exams"][0]["date"] = json!("2026-04-27");
    let moved = view(&data);
    assert_eq!(
        (moved.date.as_str(), moved.source.date.as_str()),
        ("2026-04-24", "2026-04-27")
    );
    assert_eq!(moved.conflict.unwrap().base.date, "2026-04-23");
    let block = resolve_day_agenda(
        &data,
        &SchoolReads::fixture(&data),
        "2026-1Q",
        "2026-04-24",
        Default::default(),
    )
    .unwrap();
    assert_eq!(block[0].source_changed, Some(true));
    // Marking the exam keeps the conflict; only editing the schedule decides.
    let marked = super::exams::marked_session_override(
        &data,
        &SchoolReads::fixture(&data),
        "reference:final",
        "quiz",
    )
    .unwrap();
    save(&mut data, "reference:final", marked);
    assert!(view(&data).conflict.is_some());
    // "Keep mine": saving the student's date again records today's source.
    correct(&mut data, "reference:final", "2026-04-24");
    assert!(view(&data).conflict.is_none());
    assert_eq!(view(&data).kind, "quiz");

    // The timetable renames the row when the exam moves. The correction
    // follows the one row of the same quarter, code and kind, as a conflict.
    data["exams"] = json!([row("reference:final-renamed", "2026-05-01")]);
    let renamed = view(&data);
    assert_eq!(
        (
            renamed.key.as_str(),
            renamed.date.as_str(),
            renamed.kind.as_str()
        ),
        ("reference:final-renamed", "2026-04-24", "quiz")
    );
    assert!(renamed.conflict.is_some_and(|conflict| !conflict.removed));
    assert_eq!(
        super::exams::adopted_exam_override(
            &data,
            &SchoolReads::fixture(&data),
            "reference:final-renamed"
        )
        .as_deref(),
        Some("override:reference:final")
    );
    // Saving moves the correction to the new key ("use the published date").
    correct(&mut data, "reference:final-renamed", "2026-05-01");
    assert!(data["scheduleOverrides"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["targetKey"] != "reference:final"));
    assert_eq!(view(&data).date, "2026-05-01");
    assert!(view(&data).conflict.is_none());

    // Two candidate rows are a guess, never followed: the exam reads as
    // removed and keeps the student's values.
    correct(&mut data, "reference:final-renamed", "2026-05-02");
    data["exams"] = json!([
        row("reference:a", "2026-05-08"),
        row("reference:b", "2026-05-09")
    ]);
    let removed = exam_views(&data, &SchoolReads::fixture(&data), "2026-1Q")
        .into_iter()
        .find(|view| view.key == "reference:final-renamed")
        .unwrap();
    assert_eq!(removed.date, "2026-05-02");
    assert!(removed.conflict.is_some_and(|conflict| conflict.removed));
    // A removed exam can still be deleted from the schedule.
    let deleted = super::exams::removed_exam_override(
        &data,
        &SchoolReads::fixture(&data),
        "reference:final-renamed",
    )
    .unwrap();
    save(&mut data, "reference:final-renamed", deleted);
    assert!(exam_views(&data, &SchoolReads::fixture(&data), "2026-1Q")
        .iter()
        .any(|view| view.key == "reference:final-renamed" && view.course_id.is_none()));
}

#[test]
fn reminders_follow_marks_exam_week_classes_and_timetable_exams() {
    let mut data = workspace();
    data["academicCalendar"][0]["examRanges"] = json!([{"start":"2026-05-28","end":"2026-06-09"}]);
    data["settings"]["notifications"] = json!({"classes":true,"exams":true,"classLeadMinutes":10,"examLeadMinutes":60,"assignments":false});
    let row = |id: &str, kind: &str, date: &str, periods: [u8; 2]| json!({"id":id,"kind":kind,"sourceKey":format!("reference:{id}"),"academicYear":2026,"quarter":"1Q","courseCode":"CSC.T241","date":date,"periods":periods,"room":{"ja":"W8E-E207"},"label":{"ja":"授業"},"status":"scheduled"});
    data["exams"] = json!([
        row("week-class", "class", "2026-05-28", [5, 6]),
        row("final", "exam", "2026-05-29", [3, 4]),
        row("dropped", "exam", "2026-06-02", [3, 4]),
    ]);
    for (key, mark) in [
        ("course-weekly-0-2026-04-13", "cancelled"),
        ("course-weekly-0-2026-04-20", "exam"),
        ("course-weekly-0-2026-05-11", "quiz"),
        ("course-weekly-0-2026-06-01", "class"),
        ("reference:dropped", "cancelled"),
    ] {
        let mut row =
            super::exams::marked_session_override(&data, &SchoolReads::fixture(&data), key, mark)
                .unwrap()
                .unwrap();
        row["id"] = json!(key);
        data["scheduleOverrides"].as_array_mut().unwrap().push(row);
    }
    let now = DateTime::parse_from_rfc3339("2026-04-12T00:00:00+09:00").unwrap();
    let horizon = DateTime::parse_from_rfc3339("2026-06-03T00:00:00+09:00").unwrap();
    let plan = notification_plan(&data, &SchoolReads::fixture(&data), now, horizon, None).unwrap();
    let items: Vec<_> = plan
        .items
        .iter()
        .map(|item| {
            (
                item.at.as_str(),
                matches!(item.priority, NotificationPriority::Exam),
                item.title.starts_with("Exam · ") || item.title.starts_with("Quiz · "),
            )
        })
        .collect();
    assert!(plan
        .items
        .iter()
        .any(|item| item.at == "2026-05-11T07:50:00+09:00" && item.title.starts_with("Quiz · ")));
    // Weekly Mondays from 04-27 to 05-25 remind as classes; 04-13 is
    // cancelled and 04-20 is an exam with the exam lead.
    assert!(!items.iter().any(|item| item.0.starts_with("2026-04-13")));
    assert!(items.contains(&("2026-04-20T07:50:00+09:00", true, true)));
    assert!(items.contains(&("2026-04-27T08:40:00+09:00", false, false)));
    // The timetable's exam-week class is a class; its final is an exam.
    assert!(items.contains(&("2026-05-28T13:20:00+09:00", false, false)));
    assert!(items.contains(&("2026-05-29T09:45:00+09:00", true, true)));
    // The exam period stops the weekly class, unless the user says it runs;
    // a timetable exam the user cancelled never reminds.
    assert!(items.contains(&("2026-06-01T08:40:00+09:00", false, false)));
    assert!(!items.iter().any(|item| item.0.starts_with("2026-06-02")));
    assert_eq!(
        items.iter().filter(|item| item.1).count(),
        3,
        "only the marked exam, the quiz and the timetable final remind as exams"
    );
    // With quizzes off, the quiz reminds as its class.
    data["settings"]["notifications"]["quizzes"] = json!(false);
    let quiz_off =
        notification_plan(&data, &SchoolReads::fixture(&data), now, horizon, None).unwrap();
    assert!(quiz_off
        .items
        .iter()
        .any(|item| item.at == "2026-05-11T08:40:00+09:00"
            && matches!(item.priority, NotificationPriority::Class)));
    data["settings"]["notifications"]["quizzes"] = json!(true);
    // Turning exams off keeps the exam-week class, a class reminder.
    data["settings"]["notifications"]["exams"] = json!(false);
    let classes_only =
        notification_plan(&data, &SchoolReads::fixture(&data), now, horizon, None).unwrap();
    assert!(classes_only
        .items
        .iter()
        .all(|item| matches!(item.priority, NotificationPriority::Class)));
    assert!(classes_only
        .items
        .iter()
        .any(|item| item.at == "2026-05-28T13:20:00+09:00"));
}

#[test]
fn timetable_year_labels_every_week_with_its_quarter() {
    let mut data = workspace();
    let calendar = data["academicCalendar"].as_array_mut().unwrap();
    calendar[0]["examRanges"] = json!([{"start":"2026-06-03","end":"2026-06-09"}]);
    calendar.push(json!({"id":"calendar:2026:quarter-start:2Q","academicYear":2026,"quarter":"2Q","date":"2026-06-10","type":"quarter-start","label":{"en":"start"}}));
    calendar.push(json!({"id":"calendar:2026:quarter-end:2Q","academicYear":2026,"quarter":"2Q","date":"2026-08-05","type":"quarter-end","label":{"en":"end"}}));

    let year = calendar::timetable_year(&data, 2026).unwrap();
    let week = |monday: &str| {
        year.weeks
            .iter()
            .find(|week| week.monday == monday)
            .map(|week| (week.term.as_deref(), week.week, week.exams))
            .unwrap()
    };
    // April 1 is a Wednesday: the year starts on its Monday and ends with
    // the week holding March 31.
    assert_eq!(year.weeks.first().unwrap().monday, "2026-03-30");
    assert_eq!(year.weeks.last().unwrap().monday, "2027-03-29");
    assert_eq!(week("2026-03-30"), (None, None, false));
    // Counted as Today counts: the week of the quarter's first day is week 1.
    assert_eq!(week("2026-04-06"), (Some("2026-1Q"), Some(1), false));
    assert_eq!(week("2026-04-13"), (Some("2026-1Q"), Some(2), false));
    assert_eq!(week("2026-06-01"), (Some("2026-1Q"), Some(9), true));
    // A week shared by two quarters belongs to the earlier one.
    assert_eq!(week("2026-06-08"), (Some("2026-1Q"), Some(10), true));
    assert_eq!(week("2026-06-15"), (Some("2026-2Q"), Some(2), false));
    assert_eq!(week("2026-08-10"), (None, None, false));
    // Jumping to a quarter lands on its first teaching week; quarters without
    // known dates are left out rather than guessed.
    let quarters: Vec<_> = year
        .quarters
        .iter()
        .map(|quarter| (quarter.term.as_str(), quarter.first_week.as_str()))
        .collect();
    assert_eq!(
        quarters,
        [("2026-1Q", "2026-04-06"), ("2026-2Q", "2026-06-08")]
    );
    assert!(calendar::timetable_year(&data, 9999).is_err());
}

#[test]
fn timetable_axis_gives_every_period_one_row_and_lunch_half_a_row() {
    let axis = timetable_axis(&default_teaching_periods(), None, None).unwrap();
    let rows: Vec<_> = axis
        .rows
        .iter()
        .map(|row| (row.periods, row.top, row.bottom))
        .collect();
    assert_eq!(
        rows,
        [
            (Some([1, 2]), 0.0, 2.0),
            (Some([3, 4]), 2.0, 4.0),
            (None, 4.0, 4.5),
            (Some([5, 6]), 4.5, 6.5),
            (Some([7, 8]), 6.5, 8.5),
            (Some([9, 10]), 8.5, 10.5),
            (Some([11, 12]), 10.5, 12.5),
        ]
    );
    assert_eq!(axis.length, 12.5);
    let at = |clock: &str| {
        let (hour, minute) = clock.split_once(':').unwrap();
        axis.offset(hour.parse::<u32>().unwrap() * 60 + minute.parse::<u32>().unwrap())
    };
    // A class that ends at its block's end fills the cell: the 15-minute
    // break after it folds into the rule.
    assert_eq!(
        (at("08:50"), at("10:30"), at("10:40"), at("10:45")),
        (0.0, 2.0, 2.0, 2.0)
    );
    // Times inside a block, such as an exam, keep their proportion.
    assert_eq!(at("09:40"), 1.0);
    // Lunch maps into its band; the day's edges clamp.
    assert!((4.0..4.5).contains(&at("12:55")));
    assert_eq!((at("07:30"), at("22:00")), (0.0, 12.5));

    let broken = [TeachingPeriod {
        periods: [1, 2],
        start_time: "10:00".into(),
        end_time: "09:00".into(),
    }];
    assert!(timetable_axis(&broken, None, None).is_err());
    // A custom class outside the teaching day widens the axis at the edge
    // blocks' pace (50 minutes a row) instead of being cut off.
    let wide = timetable_axis(&default_teaching_periods(), Some("07:30"), Some("22:00")).unwrap();
    assert_eq!(wide.offset(7 * 60 + 30), 0.0);
    assert_eq!(wide.offset(8 * 60 + 50), 1.6);
    assert_eq!(wide.rows[0].top, 1.6);
    assert_eq!(wide.length, 1.6 + 12.5 + 1.5);
    assert_eq!(wide.offset(22 * 60), wide.length);
    // Times inside the teaching day leave it as it is.
    assert_eq!(
        timetable_axis(&default_teaching_periods(), Some("09:00"), Some("20:00")).unwrap(),
        axis
    );
    let request: WorkspaceQuery = serde_json::from_value(json!({"kind":"timetableAxis"})).unwrap();
    assert!(matches!(
        query(&workspace(), &SchoolReads::default(), request).unwrap(),
        WorkspaceQueryResult::TimetableAxis(view) if view.length == 12.5
    ));
}

#[test]
fn assignment_detail_reads_the_lms_text_without_moodle_dates() {
    let body = concat!(
        r#"<div data-region="activity-dates"><strong>Opened:</strong> 1 Oct</div>"#,
        r#"<p>Write a parser &amp; test it.</p><br>"#,
        r#"<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/introattachment/0/sheet%201.pdf">Sheet</a>"#,
        r#"<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/introattachment/0/sheet%201.pdf">again</a>"#,
        r#"<a href="https://example.com/pluginfile.php/2/x.pdf">elsewhere</a>"#,
        r#"<div data-region="activity-information">Done: view</div>"#,
    );
    let data = json!({
        "courses":[{"id":"c","code":"CSC.T273","termId":"2026-3Q"}],
        "materials":[{"id":"m","courseId":"c","cmid":5,"module":"assign","bodyHtml":body,"sectionNumber":6,"sectionTitle":"第6回"}],
        "assignments":[
            {"id":"a","courseId":"c","title":"Report","details":"","dueAt":"2026-10-22T04:30:00Z","lmsCmid":5,"submission":"open","sourceUrl":"https://lms.s.isct.ac.jp/2026/mod/assign/view.php?id=5"},
            {"id":"gone","courseId":"c","title":"Old","details":"","deletedAt":"x"}
        ]
    });
    let request: WorkspaceQuery =
        serde_json::from_value(json!({"kind":"assignmentDetail","assignmentId":"a"})).unwrap();
    let WorkspaceQueryResult::AssignmentDetail(Some(view)) =
        query(&data, &SchoolReads::fixture(&data), request).unwrap()
    else {
        panic!("wrong result kind")
    };
    assert_eq!(
        view.body_html,
        concat!(
            "<p>Write a parser &amp; test it.</p><br>",
            r#"<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/introattachment/0/sheet%201.pdf">Sheet</a>"#,
            r#"<a href="https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/introattachment/0/sheet%201.pdf">again</a>"#,
            r#"<a href="https://example.com/pluginfile.php/2/x.pdf">elsewhere</a>"#,
        )
    );
    // One entry per LMS file, named by its decoded file name.
    assert_eq!(
        view.attachments,
        [AssignmentAttachment {
            name: "sheet 1.pdf".into(),
            url: "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_assign/introattachment/0/sheet%201.pdf".into(),
        }]
    );
    assert_eq!(view.files.unwrap().course, "CSC.T273");
    assert!(assignment_detail(&data, "gone").is_none());
    // Names decode; a link to a folder falls back to its text; plain http,
    // relative links and other hosts are not attachments.
    let mut named = data.clone();
    named["materials"][0]["bodyHtml"] = json!(concat!(
        r#"<a href="https://lms.s.isct.ac.jp/pluginfile.php/1/%E8%AA%B2%E9%A1%8C.pdf">課題</a>"#,
        r#"<a href="https://lms.s.isct.ac.jp/pluginfile.php/1/mod_assign/intro/">instructions</a>"#,
        r#"<a href="http://lms.s.isct.ac.jp/pluginfile.php/1/x.zip">plain</a>"#,
        r#"<a href="/pluginfile.php/relative.zip">relative</a>"#,
    ));
    let names: Vec<_> = assignment_detail(&named, "a")
        .unwrap()
        .attachments
        .into_iter()
        .map(|file| file.name)
        .collect();
    assert_eq!(names, ["課題.pdf", "instructions"]);
    // Without a module row the page still shows the assignment's own fields.
    let mut bare = data.clone();
    bare["materials"] = json!([]);
    let view = assignment_detail(&bare, "a").unwrap();
    assert!(view.body_html.is_empty() && view.attachments.is_empty());
    assert_eq!(view.title, "Report");
}

#[test]
fn course_content_follows_the_desktop_lecture_desk() {
    for (title, number) in [
        ("第2回", Some(2)),
        ("第 12 回 クラス", Some(12)),
        ("第３回", Some(3)),
        ("Lecture 4", Some(4)),
        ("Week 10: Graphs", Some(10)),
        ("5. Recursion", Some(5)),
        ("一般", None),
        ("連絡先・オフィスアワー", None),
        ("2026年度 講義資料", None),
    ] {
        let read = course_content::lesson_title(title).map(|title| title.number);
        assert_eq!(read, number, "{title}");
    }

    // Sections and their text come from the LMS record; section numbers and
    // lesson numbers differ (section 6 is 第5回). Moodle may send a section
    // id as a string, and its files still sit in it.
    let data = json!({
        "courses":[{"id":"c","termId":"2026-3Q","offeringId":"o"}],
        "lmsCourses":[{"lmsId":8,"offeringId":"o","sections":[
            {"id":100,"number":0,"title":"一般","bodyHtml":""},
            {"id":"105","number":5,"title":"第4回","bodyHtml":"<div class=\"summarytext\"><div>授業計画：配列<br>課題：確認</div></div><ul data-for=\"cmlist\"></ul>"},
            {"id":106,"number":6,"title":"第5回","bodyHtml":"<script>x()</script><p>授業計画：継承</p>"},
            {"id":107,"number":7,"title":"第6回","bodyHtml":""},
            {"id":108,"number":8,"title":"第7回","bodyHtml":"<p>hidden</p>","visible":false},
            {"id":109,"number":9,"title":"第8回","bodyHtml":""}
        ]}],
        "materials":[
            {"id":"g","courseId":"c","session":0,"sectionId":100,"module":"forum","title":"お知らせ","bodyHtml":"<article>Room changed</article>"},
            {"id":"a","courseId":"c","session":5,"sectionId":105,"module":"resource","title":"slides","url":"https://lms.s.isct.ac.jp/mod/resource/view.php?id=1"},
            {"id":"l","courseId":"c","session":5,"sectionId":105,"module":"label","title":"note","bodyHtml":"<p>Bring a laptop</p>"},
            {"id":"b","courseId":"c","session":6,"sectionId":106,"module":"page","title":"Reading","bodyHtml":"<p>Read chapter 3</p>"},
            {"id":"task","courseId":"c","session":7,"sectionId":107,"module":"assign","cmid":5,"title":"Report"},
            {"id":"quiz","courseId":"c","session":7,"sectionId":107,"module":"quiz","title":"Quiz"}
        ],
        "assignments":[
            {"id":"r","courseId":"c","title":"Report","details":"","lmsCmid":5},
            {"id":"loose","courseId":"c","title":"Essay","details":""},
            {"id":"gone","courseId":"c","title":"Gone","details":"","lmsCmid":5,"hidden":true}
        ]
    });
    let request: WorkspaceQuery = serde_json::from_value(
        json!({"kind":"courseContent","courseId":"c","now":"2026-11-24T12:00:00+09:00"}),
    )
    .unwrap();
    let WorkspaceQueryResult::CourseContent(Some(view)) =
        query(&data, &SchoolReads::fixture(&data), request).unwrap()
    else {
        panic!("wrong result kind")
    };
    let lessons: Vec<_> = view
        .lessons
        .iter()
        .map(|lesson| {
            (
                lesson.section,
                lesson.number,
                lesson.label.as_str(),
                lesson.posted,
            )
        })
        .collect();
    // Hidden sections drop out, and so do empty ones that are not lessons; a
    // lesson with nothing posted yet keeps its place. With 第7回 hidden the
    // numbers skip, so the rail counts the lessons in order.
    assert_eq!(
        lessons,
        [
            (0, None, "一般", true),
            (5, Some(4), "1", true),
            (6, Some(5), "2", true),
            (7, Some(6), "3", false),
            (9, Some(8), "4", false)
        ]
    );
    assert_eq!(view.current_section, Some(6));
    // An assignment sits where its LMS activity does; one without an
    // activity is the course's own, and a hidden one is in neither.
    assert_eq!(view.lessons[3].assignment_ids, ["r"]);
    assert_eq!(view.other_assignment_ids, ["loose"]);
    let fifth = &view.lessons[2];
    assert_eq!(fifth.summary_html, "<p>授業計画：継承</p>");
    // A folded lesson is headed by its plan's first line, without the label.
    let summaries: Vec<_> = view
        .lessons
        .iter()
        .map(|lesson| lesson.summary.as_str())
        .collect();
    assert_eq!(summaries, ["", "配列", "継承", "", ""]);
    assert_eq!(
        view.lessons[1].summary_html,
        "<div><div>授業計画：配列<br>課題：確認</div></div><ul></ul>"
    );
    let items: Vec<_> = view.lessons[1]
        .items
        .iter()
        .map(|item| (item.kind, item.body_html.as_deref()))
        .collect();
    assert_eq!(
        items,
        [
            (LessonItemKind::File, None),
            (LessonItemKind::Label, Some("<p>Bring a laptop</p>"))
        ]
    );
    assert_eq!(view.lessons[0].items[0].kind, LessonItemKind::Forum);
    assert!(view.lessons[3].items.is_empty());
    let now = DateTime::parse_from_rfc3339("2026-11-24T12:00:00+09:00").unwrap();
    assert!(course_content(&data, &SchoolReads::fixture(&data), "unknown", now).is_none());

    // Linear Algebra II【X】's sections: lectures and recitations interleave.
    // The series with more lessons shows bare numbers, the other its own
    // word from the title; a label alone does not post a lesson.
    let titles = [
        "一般",
        "連絡先・オフィスアワー",
        "第1回",
        "第2回",
        "演習第1回",
        "第3回",
        "第4回",
        "演習第2回",
        "第5回",
        "第6回",
        "演習第3回",
        "第7回",
        "演習第4回",
        "第8回",
        "演習第5回",
        "第9回",
        "第10回",
        "演習第6回",
        "第11回",
        "第12回",
        "第13回",
        "演習第7回",
        "第14回",
    ];
    // 一般 holds the course's schedule table, which Core marks against the
    // lesson the course is at.
    let schedule =
        "<table><tr><td>1回目</td><td>行列</td></tr><tr><td>2回目</td><td>行列式</td></tr></table>";
    let mut algebra = data.clone();
    algebra["lmsCourses"][0]["sections"] = titles
        .iter()
        .enumerate()
        .map(|(number, title)| {
            let body = match number {
                0 => schedule,
                1 => "<p>Office hours</p>",
                2 => "<p>Course schedule: Basic syntax</p><p>Objectives: …</p>",
                _ => "",
            };
            json!({"id":200 + number,"number":number,"title":title,"bodyHtml":body})
        })
        .collect();
    algebra["materials"] = json!([
        {"id":"g","courseId":"c","session":0,"sectionId":200,"module":"forum","title":"お知らせ"},
        {"id":"s","courseId":"c","session":3,"sectionId":203,"module":"resource","title":"slides"},
        {"id":"n","courseId":"c","session":4,"sectionId":204,"module":"label","title":"board","bodyHtml":"<p><img src=\"https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_label/intro/board.png\" alt=\"\"></p>"}
    ]);
    let view = course_content(&algebra, &SchoolReads::fixture(&algebra), "c", now).unwrap();
    let labels: Vec<_> = view
        .lessons
        .iter()
        .map(|lesson| lesson.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "一般",
            "連絡先・オフィスアワー",
            "1",
            "2",
            "演習 1",
            "3",
            "4",
            "演習 2",
            "5",
            "6",
            "演習 3",
            "7",
            "演習 4",
            "8",
            "演習 5",
            "9",
            "10",
            "演習 6",
            "11",
            "12",
            "13",
            "演習 7",
            "14"
        ]
    );
    assert_eq!(view.current_section, Some(3));
    assert_eq!(view.lessons[2].summary, "Basic syntax");
    assert_eq!(
        view.lessons[0].summary_html,
        concat!(
            r#"<table><tbody><tr class="lmsRowPast"><td>1回目</td><td>行列</td></tr>"#,
            r#"<tr class="lmsRowNow"><td>2回目</td><td>行列式</td></tr></tbody></table>"#
        )
    );
    // A label of pictures only is marked for laying out side by side.
    assert!(view.lessons[4].items[0].image_only);

    // One series whose titles skip and repeat a number counts in order.
    algebra["materials"] = json!([]);
    algebra["lmsCourses"][0]["sections"] = ["第7回", "第9回", "第8回", "第11回", "第11回"]
        .iter()
        .enumerate()
        .map(|(number, title)| json!({"id":300 + number,"number":number,"title":title}))
        .collect();
    let view = course_content(&algebra, &SchoolReads::fixture(&algebra), "c", now).unwrap();
    let rail: Vec<_> = view
        .lessons
        .iter()
        .map(|lesson| (lesson.number, lesson.label.as_str()))
        .collect();
    // Before anything is posted, the course is at its first lesson.
    assert_eq!(view.current_section, Some(0));
    assert_eq!(
        rail,
        [
            (Some(7), "1"),
            (Some(9), "2"),
            (Some(8), "3"),
            (Some(11), "4"),
            (Some(11), "5")
        ]
    );

    // Without an LMS record, materials name the sections; one numbered
    // section is not a lesson course.
    let mut plain = data.clone();
    plain["lmsCourses"] = json!([]);
    plain["materials"] = json!([
        {"id":"g","courseId":"c","session":0,"sectionTitle":"一般","title":"Syllabus"},
        {"id":"h","courseId":"c","session":0,"sectionTitle":"一般","title":"Guide"},
        {"id":"a","courseId":"c","session":3,"sectionTitle":"第1回","title":"slides"},
        {"id":"r","courseId":"c","session":4,"sectionTitle":"Reading","title":"paper"}
    ]);
    let view = course_content(&plain, &SchoolReads::fixture(&plain), "c", now).unwrap();
    assert!(view.lessons.iter().all(|lesson| lesson.number.is_none()));
    let labels: Vec<_> = view
        .lessons
        .iter()
        .map(|lesson| lesson.label.as_str())
        .collect();
    assert_eq!(labels, ["一般", "第1回", "Reading"]);
    // The syllabus link is the course page's own, not course content.
    assert_eq!(view.lessons[0].items.len(), 1);
    assert_eq!(view.lessons[0].items[0].title, "Guide");
    assert_eq!(view.current_section, Some(4));

    // A mark for the class it is recorded against (the next one here, as
    // the last was not today) wins over "last lesson with material"; once a
    // later class has begun, the course is at the lesson after the mark.
    let mut marked = multi_quarter_workspace();
    marked["courses"][0]["offeringId"] = json!("202630001");
    marked["lmsCourses"] = json!([{"lmsId":9,"offeringId":"202630001","sections":[
        {"id":101,"number":1,"title":"第1回","bodyHtml":"<p>a</p>"},
        {"id":102,"number":2,"title":"第2回","bodyHtml":"<p>b</p>"},
        {"id":103,"number":3,"title":"第3回","bodyHtml":""}
    ]}]);
    marked["materials"] = json!([
        {"id":"s3","courseId":"multi","session":3,"sectionId":103,"module":"resource","title":"slides"}
    ]);
    let view = course_content(&marked, &SchoolReads::fixture(&marked), "multi", now).unwrap();
    assert_eq!(view.class_at.as_deref(), Some("2026-12-07T08:50:00+09:00"));
    assert_eq!(view.current_section, Some(3));
    assert_eq!(view.lessons[0].section_id, Some(101));
    // The latest mark counts wherever it sits in the log, and a mark for a
    // section this course does not have is no mark.
    marked["courses"][0]["overrides"] = json!({"lessonLog":[
        {"occurrence":"2026-12-07T08:50:00+09:00","sectionId":101},
        {"occurrence":"2026-11-09T08:50:00+09:00","sectionId":103},
        {"occurrence":"2026-12-14T08:50:00+09:00","sectionId":999}
    ]});
    assert_eq!(
        course_content(&marked, &SchoolReads::fixture(&marked), "multi", now)
            .unwrap()
            .current_section,
        Some(1)
    );
    marked["courses"][0]["overrides"]["lessonLog"][0]["occurrence"] =
        json!("2026-11-16T08:50:00+09:00");
    assert_eq!(
        course_content(&marked, &SchoolReads::fixture(&marked), "multi", now)
            .unwrap()
            .current_section,
        Some(2)
    );
}

/// A shell keeps a result until an event names a change in its `reads`, so
/// a query must list every change whose records alter its answer. Without
/// the fields a change feeds, a query that does not read it must answer the
/// same.
#[test]
fn every_query_lists_the_changes_its_result_reads() {
    let mut data = phone_workspace();
    data["events"] = json!([{"id":"event","termId":"2026-1Q","days":[1],"title":"Club","room":"","startTime":"18:00","endTime":"19:00"}]);
    data["courses"].as_array_mut().unwrap().push(json!({"id":"planned","termId":"2026-1Q","title":"Planned","code":"CSC.T400","meetings":[{"days":[2],"startTime":"10:45","endTime":"12:25"}]}));
    data["planItems"] = json!([{"id":"item","termId":"2026-1Q","courseId":"planned"}]);
    data["exceptions"] =
        json!([{"id":"mine","date":"2026-04-20","type":"no-class","label":"Trip"}]);
    data["registrations"] = json!([{"id":"r","termId":"2026-1Q","code":"CSC.T241","credits":2}]);
    data["grades"] =
        json!([{"id":"g","termId":"2026-1Q","code":"CSC.T241","credits":2,"grade":"A"}]);
    let now = "2026-04-13T10:00:00+09:00";
    let queries = [
        json!({"kind":"workspaceOverview","term":"2026-1Q"}),
        json!({"kind":"planningWeek","term":"2026-1Q"}),
        json!({"kind":"scheduleView","term":"2026-1Q","from":"2026-04-12","to":"2026-04-25"}),
        json!({"kind":"dayAgenda","term":"2026-1Q","date":"2026-04-13"}),
        json!({"kind":"taskView","now":now}),
        json!({"kind":"today","now":now}),
        json!({"kind":"assignmentBoard","now":now}),
        json!({"kind":"courseDetail","courseId":"course"}),
        json!({"kind":"courseContent","courseId":"course","now":now}),
        json!({"kind":"lectureSearch","term":"2026-1Q","query":"slides"}),
        json!({"kind":"scheduledCourses","term":"2026-1Q"}),
        json!({"kind":"quarterRange","term":"2026-1Q"}),
        json!({"kind":"academicYear","year":2026}),
        json!({"kind":"declaredCredits","term":"2026-1Q"}),
        json!({"kind":"glance","now":now}),
        json!({"kind":"notificationPlan","now":now,"horizon":"2026-04-20T10:00:00+09:00"}),
        json!({"kind":"classGroups","today":"2026-04-13"}),
        json!({"kind":"records","fields":["events","exceptions"]}),
    ];
    let answer = |data: &Value, request: &WorkspaceQuery| {
        serde_json::to_value(query(data, &SchoolReads::fixture(data), request.clone()).unwrap())
            .unwrap()
    };
    for raw in queries {
        let request: WorkspaceQuery = serde_json::from_value(raw).unwrap();
        let full = answer(&data, &request);
        for change in crate::workspace_changes::WorkspaceChange::ALL {
            if request.reads().contains(&change) {
                continue;
            }
            let Some(fields) = crate::workspace_changes::fed_fields(change) else {
                panic!("{request:?} must read {change:?}, which re-reads every field");
            };
            let mut without = data.clone();
            for field in fields {
                without.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(
                answer(&without, &request),
                full,
                "{request:?} depends on {change:?} but does not read it"
            );
        }
    }
}

#[test]
fn the_month_rule_puts_january_to_march_in_the_previous_academic_year() {
    use chrono::{NaiveDate, TimeZone, Utc};
    let term = |y, m, d| Term::by_month(NaiveDate::from_ymd_opt(y, m, d).unwrap()).to_string();
    assert_eq!(term(2027, 3, 31), "2026-4Q");
    assert_eq!(term(2026, 4, 1), "2026-1Q");
    assert_eq!(term(2026, 5, 31), "2026-1Q");
    assert_eq!(term(2026, 6, 1), "2026-2Q");
    assert_eq!(term(2026, 9, 30), "2026-2Q");
    assert_eq!(term(2026, 10, 1), "2026-3Q");
    assert_eq!(term(2026, 11, 30), "2026-3Q");
    assert_eq!(term(2026, 12, 1), "2026-4Q");
    assert_eq!(term(2027, 1, 1), "2026-4Q");
    // 15:00 UTC on 31 March is already 1 April in Japan; a minute earlier
    // is still the last day of the previous academic year.
    let in_japan = |h, m| {
        let now = Utc.with_ymd_and_hms(2026, 3, 31, h, m, 0).unwrap();
        Term::by_month(japan_today(now)).to_string()
    };
    assert_eq!(in_japan(14, 59), "2025-4Q");
    assert_eq!(in_japan(15, 0), "2026-1Q");
}

#[test]
fn calendar_uids_keep_unreserved_marks_and_escape_every_other_byte() {
    // Exported UIDs must not change between releases, or calendars that
    // imported an earlier export duplicate every event.
    assert_eq!(
        super::ics::percent_encode("plan:2026-4Q/a_b.c!~*'(x) y–é"),
        "plan%3A2026-4Q%2Fa_b.c!~*'(x)%20y%E2%80%93%C3%A9"
    );
}
