use super::*;
fn eligibility_offering(id: &str, code: &str) -> Value {
    json!({
        "id":id, "year":2026, "code":code, "quarters":["3Q"],
        "unitCode":"342300", "title":{"en":code}, "format":{"kinds":[]},
        "kyomu":{"department":"情報工学系","group":"A","mark":"recommended"}
    })
}

#[test]
fn first_year_eligibility_uses_the_planned_year_and_keeps_all_course_search_visible() {
    let offerings = vec![
        eligibility_offering("100", "CSC.T101"),
        eligibility_offering("200", "CSC.T201"),
        eligibility_offering("humanities", "LAH.T209-01"),
        eligibility_offering("law", "LAW.W221-12"),
        eligibility_offering("300", "CSC.T301"),
    ];
    let workspace = json!({
        "currentTerm":"2026-3Q",
        "academicProfile":{"admissionYear":2026,"studyEnrollments":[{
            "stage":"undergraduate","entryKind":"regular","entryGrade":1,
            "admittedTerm":"2026-1Q"
        }]},
        "settings":{"departmentId":"cs"},
        "recommendedCourses":[
            {"admissionYear":2026,"departmentId":"cs","code":"CSC.T101","group":"A"},
            {"admissionYear":2026,"departmentId":"cs","code":"CSC.T201","group":"B"}
        ],
        "courses":[], "memberships":[], "planItems":[]
    });
    let request = |term: &str,
                   keys: Vec<&str>,
                   include_ineligible: bool,
                   action: Option<FinderAction>| FinderRequest {
        year: term[..4].parse().unwrap(),
        term: term.into(),
        include_ineligible,
        keys: keys.into_iter().map(str::to_string).collect(),
        input: String::new(),
        action,
    };

    let default = finder_view(
        &workspace,
        offerings.clone(),
        request("2026-3Q", vec!["3Q"], false, None),
    )
    .unwrap();
    assert_eq!(
        default.ineligible_ids,
        vec!["200", "300", "humanities", "law"]
    );
    assert_eq!(
        default.eligibility["100"].status,
        CourseEligibilityStatus::Allowed
    );
    assert_eq!(
        default.eligibility["humanities"].reason,
        crate::study_profile::CourseEligibilityReason::UndergraduateFirstYearUpperLevel
    );
    assert_eq!(
        default
            .rows
            .iter()
            .map(|row| text(&row.course, "id"))
            .collect::<Vec<_>>(),
        vec!["100"]
    );
    assert!(default.context.recommended.contains_key("CSC.T101"));
    assert!(!default.context.recommended.contains_key("CSC.T201"));

    let all = finder_view(
        &workspace,
        offerings.clone(),
        request("2026-3Q", vec!["3Q"], true, None),
    )
    .unwrap();
    assert_eq!(
        all.rows.len(),
        5,
        "explicit all-course search stays visible"
    );

    let pasted = finder_view(
        &workspace,
        offerings.clone(),
        request(
            "2026-3Q",
            vec![],
            false,
            Some(FinderAction::Paste {
                text: "CSC.T101 CSC.T201 LAH.T209-01 LAW.W221-12 CSC.T301".into(),
            }),
        ),
    )
    .unwrap();
    assert_eq!(pasted.pasted_ids, vec!["100"]);
    assert_eq!(
        pasted.ineligible_codes,
        vec!["CSC.T201", "LAH.T209-01", "LAW.W221-12", "CSC.T301"]
    );

    let second_year = finder_view(
        &workspace,
        offerings.clone(),
        request("2027-3Q", vec![], false, None),
    )
    .unwrap();
    assert!(second_year.ineligible_ids.is_empty());

    let mut unknown = workspace;
    unknown["academicProfile"] = json!({});
    let unknown =
        finder_view(&unknown, offerings, request("2026-3Q", vec![], false, None)).unwrap();
    assert!(unknown.ineligible_ids.is_empty());
    assert!(unknown
        .eligibility
        .values()
        .all(|result| result.status == CourseEligibilityStatus::Unknown));
}

#[test]
fn eligibility_does_not_guess_without_study_identity_or_course_number() {
    let upper_level = eligibility_offering("upper", "CSC.T301");
    assert_eq!(
        catalog_course_eligibility(&json!({"academicProfile":{}}), "2026-3Q", &upper_level).status,
        CourseEligibilityStatus::Unknown
    );
    let undergraduate = json!({"academicProfile":{"studyEnrollments":[{
        "stage":"undergraduate","entryKind":"regular","entryGrade":1,
        "admittedTerm":"2026-1Q"
    }]}});
    assert_eq!(
        catalog_course_eligibility(
            &undergraduate,
            "2026-3Q",
            &eligibility_offering("special", "SPECIAL")
        )
        .status,
        CourseEligibilityStatus::Unknown
    );
    assert_eq!(
        catalog_course_eligibility(
            &undergraduate,
            "2026-3Q",
            &eligibility_offering("foreign", "SPECIAL.2026")
        )
        .status,
        CourseEligibilityStatus::Unknown
    );
}

#[test]
fn replacement_ignores_same_course_and_nonweekly_results() {
    let fixture: Value =
        serde_json::from_str(include_str!("../course_query/fixtures.json")).unwrap();
    let placed = json!([
        {"code":"X.1","meetings":[{"days":[2],"periods":[3,4]}]},
        {"code":"X.2","meetings":[{"days":[2],"periods":[5,6]}]},
        {"code":"X.3","meetings":[{"days":[],"periods":[]}]},
        {"code":"CSC.T262","meetings":[{"days":[2],"periods":[3,4]}]}
    ]);
    assert_eq!(
        clashing_courses(&fixture["courses"][0], placed.as_array().unwrap())
            .iter()
            .map(|row| text(row, "code"))
            .collect::<Vec<_>>(),
        vec!["X.1"]
    );
    assert!(clashing_courses(&fixture["courses"][3], placed.as_array().unwrap()).is_empty());
}
#[test]
fn a_three_q_four_q_course_keeps_its_periods_busy_in_four_q() {
    let workspace = json!({
        "currentTerm":"2026-4Q", "settings":{},
        "courses":[{"id":"multi","termId":"2026-3Q","code":"MTH.B301",
            "meetings":[{"days":[1],"periods":[1,2]}]}],
        "memberships":[],
        "planItems":[
            {"id":"plan-3q","termId":"2026-3Q","courseId":"multi"},
            {"id":"plan-4q","termId":"2026-4Q","courseId":"multi"}
        ]
    });
    let view = finder_view(
        &workspace,
        vec![],
        serde_json::from_value(json!({"year":2026,"term":"2026-4Q"})).unwrap(),
    )
    .unwrap();
    assert_eq!(view.context.busy.len(), 1);
    assert_eq!(
        (
            view.context.busy[0].day,
            view.context.busy[0].from,
            view.context.busy[0].to
        ),
        (1, 1, 2)
    );
}
#[test]
fn partial_offerings_have_stable_finder_defaults() {
    let row = normalize_offering(
        json!({"id":"a","code":"ABC.A123","title":{"ja":"A"},"quarters":["3Q"],"format":{}}),
    )
    .unwrap();
    assert_eq!(row["detailed"], false);
    assert_eq!(row["instructionLanguage"], "other");
    assert_eq!(row["format"]["kinds"], json!([]));
    assert_eq!(total_credits(&row), 0.0);
}
#[test]
fn matches_original_typescript_finder_traces() {
    let fixture: Value =
        serde_json::from_str(include_str!("../course_query/fixtures.json")).unwrap();
    let ctx: FinderContext = serde_json::from_value(fixture["context"].clone()).unwrap();
    let courses = fixture["courses"].as_array().unwrap();
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let mut ctx = ctx.clone();
        let input = &case["input"];
        if input["bare"] == true {
            ctx.recommended.clear();
        }
        let keys: Vec<String> =
            serde_json::from_value(input.get("keys").cloned().unwrap_or(json!([]))).unwrap();
        let factors = factors_of(&keys, &ctx);
        let actual = match text(case, "kind") {
            "condition" => serde_json::to_value(condition(text(input, "word"), &ctx)).unwrap(),
            "factors" => serde_json::to_value(factors).unwrap(),
            "run" => json!(run(courses, &factors.factors, &ctx, "")),
            "suggest" => json!(suggest(text(input, "input"), &keys, courses, &ctx)),
            "live" => json!(live_text(text(input, "input"), &keys, courses, &ctx)),
            other => panic!("unknown trace {other}"),
        };
        assert_eq!(actual, case["expected"], "case {index}: {}", case["input"]);
    }
}

#[test]
fn teacher_room_and_overlap_filters_run_on_the_stored_catalog() {
    let context = FinderContext::default();
    let keys = ["teacher:Tanaka", "room:W9", "3Q", "火3-4"].map(str::to_string);
    let factors = factors_of(&keys, &context).factors;
    let course = |id: &str, teacher: &str, room: &str, periods: [u8; 2]| {
        json!({
            "id":id,"code":id,"quarters":["3Q"],"title":{"ja":id},
            "instructors":[{"en":teacher}],
            "schedule":[{"kind":"weekly","day":2,"periods":periods,"room":{"ja":room}}]
        })
    };
    let courses = [
        course("match", "TANAKA", "W9-324", [1, 4]),
        course("wrong-room", "Tanaka", "M-B07", [3, 4]),
        course("wrong-time", "Tanaka", "W9-324", [5, 6]),
        course("wrong-teacher", "Suzuki", "W9-324", [3, 4]),
    ];
    let rows = run(&courses, &factors, &context, "");
    assert_eq!(
        rows.iter()
            .map(|row| text(&row.course, "id"))
            .collect::<Vec<_>>(),
        vec!["match"]
    );
}

#[test]
fn official_filters_come_from_the_form_and_never_require_subscription_recommendations() {
    let mut workspace = json!({"currentTerm":"2026-3Q", "settings":{},"courses":[],"memberships":[],"planItems":[],
    "recommendedCourses":[{"code":"FAKE.T400","group":"A"}],
    "kyomuSearchOptions":[
        {"id":"ctl00_ContentPlaceHolder1_cblKamokuCodeSeries_0","value":"on","label":"400","family":"level"},
        {"id":"drpKamokuKbnMidashiSenmon","value":"121100","label":"研究関連科目","family":"category"}
    ]});
    let request = |keys: Value, input: &str| {
        serde_json::from_value(json!({
            "year":2026,"term":"2026-3Q","keys":keys,"input":input
        }))
        .unwrap()
    };
    let courses = vec![eligibility_offering("fresh", "CSC.T401")];
    let view = finder_view(
        &workspace,
        courses.clone(),
        request(json!(["400", "研究関連科目"]), "400"),
    )
    .unwrap();
    assert!(view.context.recommended.is_empty());
    // Official form choices match the stored row locally, with no guide marks.
    assert_eq!(view.rows.len(), 1);
    workspace["kyomuSearchOptions"].as_array_mut().unwrap().extend([
        json!({"id":"graduate_level","value":"live-600","label":"600","family":"level"}),
        json!({"id":"graduate_period","value":"live-12","label":"12時限","family":"start"}),
        json!({"id":"graduate_language","value":"live-en","label":"英語","family":"language"}),
        json!({"id":"future_filter","value":"live-new","label":"新しい授業形式","family":"other"}),
    ]);
    let categories = finder_view(&workspace, courses.clone(), request(json!([]), "")).unwrap();
    assert!(categories.suggestions.iter().any(
        |suggestion| matches!(suggestion, Suggestion::Category { key, .. } if key == "level:")
    ));
    let levels = finder_view(&workspace, courses.clone(), request(json!([]), "level:6")).unwrap();
    assert!(levels.live.is_empty());
    assert_eq!(levels.suggestions.len(), 1);
    assert!(
        matches!(&levels.suggestions[0], Suggestion::Condition {condition, ..} if condition.label == "600")
    );
    assert!(
        finder_view(&workspace, courses.clone(), request(json!([]), "level:500"))
            .unwrap()
            .suggestions
            .is_empty()
    );
    // Switching form metadata removes unsupported graduate suggestions, without
    // hardcoding an undergraduate/graduate range in the product vocabulary.
    workspace["kyomuSearchOptions"][0]["label"] = json!("100");
    let view = finder_view(&workspace, courses, request(json!([]), "400")).unwrap();
    assert!(!view.suggestions.iter().any(
        |s| matches!(s, Suggestion::Condition{condition,..} if condition.r#type == "official-level")
    ));
    assert!(view.context.recommended.is_empty());
    assert!(finder_view(&workspace, vec![], request(json!(["!100"]), "")).is_err());
}
