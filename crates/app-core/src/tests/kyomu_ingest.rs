use super::*;
use uuid::Uuid;

/// The internal id of the workspace course whose `field` is `value`.
fn course_with(workspace: &Value, field: &str, value: Value) -> String {
    workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|course| course[field] == value)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn page_state(store: &crate::data::DataStore, kind: &str) -> Option<sync_state::KeyState> {
    let connection = store.test_connection();
    sync_state::get(&connection, &Key::new(Dataset::KyomuPage, kind)).unwrap()
}

#[test]
fn graduate_grade_identity_survives_reordering_and_display_value_changes() {
    let original = json!({
        "title":" 先端システム論 ","instructor":"架空 教員",
        "result":"B","completed":"2026年度 1Q"
    });
    let changed = json!({
        "title":"先端システム論","instructor":"別の 教員",
        "result":"A","completed":"2026年度 1Q"
    });
    assert_eq!(
        grade_row_id(true, "graduate", &original, 7).unwrap(),
        grade_row_id(true, "graduate", &changed, 0).unwrap()
    );
    assert_ne!(
        grade_row_id(true, "graduate", &changed, 0).unwrap(),
        grade_row_id(
            true,
            "graduate",
            &json!({"title":"別科目","completed":"2026年度 1Q"}),
            0
        )
        .unwrap()
    );
}

#[test]
fn duplicate_graduate_grade_identity_rolls_back_the_page() {
    let path = std::env::temp_dir().join(format!("isct-grade-id-{}", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize_dir(path).unwrap().0;
    let row = json!({"title":"先端システム論","completed":"2026年度 1Q","result":"A"});
    let payload = KyomuPayload {
            status: "ok".into(),
            source_url: "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx".into(),
            kind: "grades".into(),
            data: json!({
                "complete":true,"shape":"graduate-flat","empty":false,
                "groups":[{"section":"graduate","category":"大学院科目","rows":[row.clone(),row]}]
            }),
            message: String::new(),
        };
    assert!(store
        .write(None, |tx| apply_kyomu_payload(tx, &payload))
        .is_err());
    assert!(store.table_rows("grades").unwrap().is_empty());
    // The rolled-back page is not marked fresh either.
    assert_eq!(page_state(&store, "grades"), None);
}

#[test]
fn gpa_summary_maps_the_totals_table_by_column_labels() {
    let data = json!({"overall": [[
        ["全ての科目", "教養科目群", "専門科目群"],
        ["GPA", "GPT", "修得済単位", "合格率%", "GPA", "修得済単位", "合格率%", "GPA", "修得済単位", "合格率%"],
        ["3.12", "312.5", "64.0", "98.0", "3.40", "30.0", "100.0", "2.90", "34.0", "96.0"]
    ]]});
    let rows = gpa_summaries(&data);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["scope"], "all");
    assert_eq!(rows[0]["gpa"], 3.12);
    assert_eq!(rows[0]["gpt"], 312.5);
    assert_eq!(rows[0]["earnedCredits"], 64.0);
    assert_eq!(rows[1]["scope"], "liberal-arts");
    assert_eq!(rows[1]["earnedCredits"], 30.0);
    assert_eq!(rows[2]["scope"], "major");
    assert!(rows[2].get("gpt").is_none());
    assert!(gpa_summaries(&json!({"overall": []})).is_empty());
}

#[test]
fn declared_credits_follow_each_category_total() {
    let data = json!({"courses": [
        {"offeringId": "202600702", "unit": "kamokuUnit_110700", "unitTotal": 1.0, "quarters": [3],
         "credits": {"lecture": 0, "exercise": 0, "lab": 1}},
        {"offeringId": "202600722", "unit": "kamokuUnit_110700", "unitTotal": 1.0, "quarters": [4],
         "credits": {"lecture": 0, "exercise": 0, "lab": 1}},
        {"offeringId": "202602431", "unit": "kamokuUnit_120900", "unitTotal": 2.0, "quarters": [3],
         "credits": {"lecture": 1, "exercise": 1, "lab": 0}},
        {"offeringId": "202637648", "unit": "kamokuUnit_110610", "quarters": [3, 4],
         "credits": {"lecture": 0, "exercise": 2, "lab": 0}}
    ]});
    let rows = declared_credits(&data);
    let credit = |id: &str| {
        rows.iter().find(|r| r["offeringId"] == id).unwrap()["effectiveCredits"]
            .as_f64()
            .unwrap()
    };
    // Two 0-0-1 PE rows in a category that totals 1.0 count half each.
    assert_eq!(credit("202600702"), 0.5);
    assert_eq!(credit("202600722"), 0.5);
    assert_eq!(credit("202602431"), 2.0);
    // Without a stated total the row's own split stands.
    assert_eq!(credit("202637648"), 2.0);
    assert_eq!(rows[0]["id"], "kyomu-declared:202600702");
}

/// LMS clocks and membership survive 教務 changes; cached user records,
/// course overlays, content joins and personal plans keep their identities.
#[test]
fn lms_owns_the_calendar_while_official_records_and_personal_edits_survive() {
    let path = std::env::temp_dir().join(format!("isct-lms-calendar-{}", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize_dir(path).unwrap().0;
    let original = json!({"id":"2026-202602431","kind":"course","offeringId":"202602431",
        "termId":"2026-3Q","code":"CSC.T273","title":"Personal","instructor":"",
        "credits":2,"creditSplit":{"lecture":2,"exercise":0,"lab":0},
        "instructionLanguage":"en","color":"#112233",
        "meetings":[{"days":[5],"periods":[5,6],"startTime":"13:30","endTime":"15:10","room":"Personal room"}],
        "source":{"provider":"lms"}});
    store.put_user_record("manual_courses", original).unwrap();
    store
        .put_user_record(
            "plan_items",
            json!({"id":"personal-plan","kind":"plan-item",
        "courseId":"2026-202602431","termId":"2026-3Q","state":"planned"}),
        )
        .unwrap();
    let mut payload: adpt_isct::lms::LmsEnrolment = serde_json::from_value(json!({
        "language":"en",
        "courses":[{"id":99,"idnumber":"202602431","shortname":"CSC.T273[2026]","fullname":"LMS course","coursecategory":3},
            {"id":100,"idnumber":"202600001","shortname":"CSC.T101[2026]","fullname":"Earlier course","coursecategory":1}],
        "timetables":[{"term":"2026-1Q","meetings":{"100":[{"day":1,"periods":[1,2]}]},"unscheduled":[]}]
    })).unwrap();
    store
        .write(None, |tx| {
            crate::lms_ingest::apply_lms_enrolment(tx, &payload, crate::clock::now())
        })
        .unwrap();
    let service = crate::workspace::WorkspaceService::new(store.clone());
    let earlier = course_with(
        &service.read_back().unwrap().workspace,
        "offeringId",
        json!("202600001"),
    );
    let saved = service
        .apply(crate::workspace::WorkspaceMutation {
            expected_revision: service.read_back().unwrap().revision,
            intent: crate::workspace::WorkspaceIntent::SaveCourse {
                draft: json!({"title":"My course name","color":"#abcdef","onTimetable":true}),
                term_id: "2026-1Q".into(),
                existing_id: Some(earlier.clone()),
            },
        })
        .unwrap();
    assert_eq!(
        saved.workspace["courses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == earlier.as_str())
            .unwrap()["overrides"]["title"],
        "My course name"
    );
    let registration = |courses| KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "registrations".into(),
        data: json!({"state":{"state":"registered"},"quarters":["3Q","4Q"],"courses":courses}),
        message: String::new(),
    };
    store.write(None, |tx| apply_kyomu_payload(tx, &registration(json!([{
        "offeringId":"202602431","code":"CSC.T273","title":"Official course","instructor":"Teacher",
        "schedule":[{"quarter":"3Q","day":"月","periods":[3,4],"room":"Official room"}]
    }])))).unwrap();
    let workspace = store.load_workspace().unwrap();
    let current = crate::schedule::scheduled_courses(&workspace, "2026-3Q");
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["lmsCourseId"], 99);
    assert_eq!(current[0]["color"], "#112233");
    assert_eq!(current[0]["credits"], 2);
    assert_eq!(current[0]["meetings"][0]["room"], "Personal room");
    assert_eq!(current[0]["meetings"][0]["days"], json!([5]));
    assert!(crate::schedule::scheduled_courses(&workspace, "2026-4Q").is_empty());
    store
        .write(None, |tx| apply_kyomu_payload(tx, &registration(json!([]))))
        .unwrap();
    assert_eq!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-3Q").len(),
        1
    );
    // A completed empty LMS quarter, unlike an empty 教務 page, removes only
    // the inferred placement; personal records remain stored.
    payload.timetables = vec![adpt_isct::lms::LmsTimetable {
        term: "2026-1Q".into(),
        meetings: std::collections::HashMap::new(),
        unscheduled: vec![],
    }];
    store
        .write(None, |tx| {
            crate::lms_ingest::apply_lms_enrolment(tx, &payload, crate::clock::now())
        })
        .unwrap();
    assert!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-1Q").is_empty()
    );
    assert_eq!(store.user_rows("manual_courses").unwrap().len(), 1);
    assert_eq!(store.user_rows("plan_items").unwrap().len(), 1);
}

#[test]
fn official_academic_identity_fills_a_fresh_profile_without_overwriting_saved_choices() {
    let path = std::env::temp_dir().join(format!("isct-academic-id-{}", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize_dir(path).unwrap().0;
    let payload = KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "profile".into(),
        data: json!({"department":"情報工学系","admittedOn":"2025-04-01","programme":"学士課程","entryCategory":"前期日程"}),
        message: String::new(),
    };
    store
        .write(None, |tx| apply_kyomu_payload(tx, &payload))
        .unwrap();
    // The page's rows and its freshness key commit together.
    let state = page_state(&store, "profile").unwrap();
    assert_eq!(state.outcome, Some(Outcome::Complete));
    assert!(state.fetched_at.is_some());
    let initial = store.load_workspace().unwrap();
    assert_eq!(initial["settings"]["departmentId"], "cs");
    assert_eq!(initial["academicProfile"]["admissionYear"], 2025);
    assert_eq!(
        initial["academicProfile"]["studyEnrollments"][0],
        json!({"stage":"undergraduate","entryKind":"regular","entryGrade":1,"admittedTerm":"2025-1Q"})
    );
    store
        .put_workspace_setting("userPreferences", json!({"departmentId":"mcs"}))
        .unwrap();
    let mut profile = initial["academicProfile"].clone();
    profile["admissionYear"] = json!(2024);
    store
        .put_workspace_setting("academicProfile", profile)
        .unwrap();
    store
        .write(None, |tx| apply_kyomu_payload(tx, &payload))
        .unwrap();
    let updated = store.load_workspace().unwrap();
    assert_eq!(updated["settings"]["departmentId"], "mcs");
    assert_eq!(updated["academicProfile"]["admissionYear"], 2024);
}

/// Past quarters absent from the current registration page still have LMS
/// enrollment. Projection must join clocks/content and retain local overlays.
#[test]
fn lms_quarter_grids_keep_clocks_overlays_and_removal_tombstones_across_official_refreshes() {
    let path = std::env::temp_dir().join(format!("isct-lms-fallback-{}", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize_dir(path).unwrap().0;
    let ingest = |enrolment: &adpt_isct::lms::LmsEnrolment| {
        store
            .write(None, |tx| {
                crate::lms_ingest::apply_lms_enrolment(tx, enrolment, crate::clock::now())
            })
            .unwrap();
    };
    let registration = |quarters: Value, courses: Value| KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "registrations".into(),
        data: json!({"state":{"state":"registered"},"quarters":quarters,"courses":courses}),
        message: String::new(),
    };
    store
        .write(None, |tx| {
            apply_kyomu_payload(tx, &registration(json!(["3Q", "4Q"]), json!([])))
        })
        .unwrap();
    let raw = json!({
        "language":"en",
        "courses":[
            {"id":41,"viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=41","idnumber":"202600041","shortname":"CSC.T101[2026]","fullname":"前期 / Earlier","coursecategory":1},
            {"id":42,"viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=42","shortname":"CSC.T102[2026]","fullname":"後半 / Second","coursecategory":2},
            {"id":43,"viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=43","shortname":"CSC.T103[2026]","fullname":"Optional","coursecategory":3},
            {"id":44,"viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=44","shortname":"Staff[2026]","fullname":"Unplaced","coursecategory":0}
        ],"timetables":[
            {"year":2026,"quarter":"1","slots":[{"courseId":41,"day":1,"period":3},{"courseId":41,"day":1,"period":4}],"unscheduled":[]},
            {"year":2026,"quarter":"2","slots":[{"courseId":42,"day":2,"period":5},{"courseId":42,"day":2,"period":6},{"courseId":42,"day":2,"period":7},{"courseId":42,"day":2,"period":8}],"unscheduled":[]}
        ]
    });
    let mut lms = adpt_isct::lms::validate_enrolment(raw).unwrap();
    assert_eq!(lms.timetables.len(), 2);
    let first_grid = lms.timetables[0].clone();
    ingest(&lms);
    lms.timetables = vec![adpt_isct::lms::LmsTimetable {
        term: "2026-2Q".into(),
        meetings: std::collections::HashMap::from([(42, vec![json!({"day":2,"periods":[5,8]})])]),
        unscheduled: vec![],
    }];
    ingest(&lms);
    let workspace = store.load_workspace().unwrap();
    let first = crate::schedule::scheduled_courses(&workspace, "2026-1Q");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["offeringId"], "202600041");
    assert_eq!(first[0]["lmsCourseId"], 41);
    assert_eq!(first[0]["meetings"][0]["startTime"], "10:45");
    assert_eq!(first[0]["meetings"][0]["termIds"], json!(["2026-1Q"]));
    let second = crate::schedule::scheduled_courses(&workspace, "2026-2Q");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0]["meetings"][0]["periods"], json!([5, 6, 7, 8]));
    assert_eq!(
        crate::schedule::scheduled_courses(&workspace, "2026-3Q").len(),
        1
    );
    assert!(!workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["lmsCourseId"] == 44));
    // A source projection is ephemeral, never a write to the user's tables.
    assert!(store.user_rows("manual_courses").unwrap().is_empty());
    assert!(store.user_rows("manual_memberships").unwrap().is_empty());
    let service = crate::workspace::WorkspaceService::new(store.clone());
    let snapshot = service.read_back().unwrap();
    service
        .apply(crate::workspace::WorkspaceMutation {
            expected_revision: snapshot.revision,
            intent: crate::workspace::WorkspaceIntent::SaveCourse {
                draft: json!({"title":"My earlier course","color":"#abcdef","onTimetable":true}),
                term_id: "2026-1Q".into(),
                existing_id: Some(course_with(&workspace, "lmsCourseId", json!(41))),
            },
        })
        .unwrap();
    let set_lms_timetable = |course_id: &str, on_timetable| {
        service
            .apply(crate::workspace::WorkspaceMutation {
                expected_revision: service.read_back().unwrap().revision,
                intent: crate::workspace::WorkspaceIntent::SetCourseOnTimetable {
                    course_id: course_id.to_owned(),
                    on_timetable,
                },
            })
            .unwrap()
    };
    let second_course = course_with(&workspace, "lmsCourseId", json!(42));
    set_lms_timetable(&second_course, false);
    ingest(&lms);
    assert!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-2Q").is_empty()
    );
    // The course's own lecture remains available after hiding the timetable.
    use crate::schedule::{query, WorkspaceQuery, WorkspaceQueryResult};
    let snapshot = service.read_back().unwrap();
    let WorkspaceQueryResult::LectureList(lectures) = query(
        &snapshot.workspace,
        &snapshot.school,
        WorkspaceQuery::LectureList {
            term: "2026-2Q".into(),
            now: "2026-07-27T08:00:00+09:00".into(),
        },
    )
    .unwrap() else {
        panic!("lecture list expected")
    };
    assert!(lectures
        .courses
        .iter()
        .any(|course| course.id == second_course));
    let timetable_control = |workspace: &Value, school: &crate::schedule::SchoolReads| {
        let WorkspaceQueryResult::WorkspaceOverview(overview) = query(
            workspace,
            school,
            WorkspaceQuery::WorkspaceOverview {
                term: "2026-2Q".into(),
            },
        )
        .unwrap() else {
            panic!("workspace overview expected")
        };
        overview.placements[&second_course].timetable.clone()
    };
    use crate::schedule::CourseTimetableControl::Lms;
    assert!(matches!(
        timetable_control(&snapshot.workspace, &snapshot.school),
        Some(Lms {
            visible: false,
            can_restore: true
        })
    ));

    // A later grid can add another visible quarter while this one stays hidden.
    let mut later = lms.clone();
    later.timetables[0].term = "2026-4Q".into();
    ingest(&later);
    let partially_hidden = service.read_back().unwrap();
    assert!(matches!(
        timetable_control(&partially_hidden.workspace, &partially_hidden.school),
        Some(Lms {
            visible: true,
            can_restore: true
        })
    ));
    assert!(crate::schedule::scheduled_courses(&partially_hidden.workspace, "2026-2Q").is_empty());
    let restored = set_lms_timetable(&second_course, true);
    assert_eq!(
        crate::schedule::scheduled_courses(&restored.workspace, "2026-2Q")[0]["id"],
        second_course
    );
    assert!(matches!(
        timetable_control(&restored.workspace, &restored.school),
        Some(Lms {
            visible: true,
            can_restore: false
        })
    ));
    assert!(store.user_rows("plan_items").unwrap().is_empty());
    set_lms_timetable(&second_course, false);

    // A saved local declaration after a hide was how older versions resumed
    // the same LMS placement. It still appears and can be hidden again.
    store
        .put_user_record(
            "manual_memberships",
            json!({
                "kind":"membership", "courseId":second_course, "termId":"2026-2Q",
                "status":"declared", "evidence":{"provider":"manual"}
            }),
        )
        .unwrap();
    let resumed = service.read_back().unwrap();
    let courses = crate::schedule::scheduled_courses(&resumed.workspace, "2026-2Q");
    assert_eq!(courses[0]["id"], second_course);
    let mut absent = lms.clone();
    absent.timetables[0].meetings.clear();
    ingest(&absent);
    assert!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-2Q").is_empty()
    );
    ingest(&lms);
    set_lms_timetable(&second_course, false);
    // Official evidence cannot replace LMS clocks or the user's overlay.
    store.write(None, |tx| apply_kyomu_payload(tx, &registration(json!(["1Q","2Q"]),json!([{
        "offeringId":"202600041","code":"CSC.T101","title":"Official earlier","instructor":"Teacher",
        "schedule":[{"quarter":"1Q","day":"木","periods":[1,2],"room":"Official room"}]
    }])))).unwrap();
    let upgraded = store.load_workspace().unwrap();
    let first = crate::schedule::scheduled_courses(&upgraded, "2026-1Q");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["source"]["provider"], "lms");
    assert_eq!(first[0]["lmsCourseId"], 41);
    assert_eq!(first[0]["overrides"]["title"], "My earlier course");
    assert_eq!(first[0]["overrides"]["color"], "#abcdef");
    assert_eq!(first[0]["meetings"][0]["room"], "");
    assert_eq!(first[0]["meetings"][0]["days"], json!([1]));
    assert!(crate::schedule::scheduled_courses(&upgraded, "2026-2Q").is_empty());
    // Without a syllabus room, the official room of the same slot fills it.
    store.write(None, |tx| apply_kyomu_payload(tx, &registration(json!(["1Q","2Q"]),json!([{
        "offeringId":"202600041","code":"CSC.T101","title":"Official earlier","instructor":"Teacher",
        "schedule":[{"quarter":"1Q","day":"月","periods":[3,4],"room":"Official room"}]
    }])))).unwrap();
    let first = crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-1Q");
    assert_eq!(first[0]["meetings"][0]["room"], "Official room");
    assert_eq!(first[0]["meetings"][0]["startTime"], "10:45");
    // The user's offline syllabus description supplies a draft clock only for
    // a quarter outside that offering's independent LMS timetable.
    let first_course = course_with(&workspace, "lmsCourseId", json!(41));
    let mut description = first[0].clone();
    description["id"] = "catalog-description:2026:202600041".into();
    description["source"] = json!({"provider":"kyomu", "externalId":"202600041"});
    description["meetings"][0]["days"] = json!([4]);
    store
        .put_user_record("manual_courses", description)
        .unwrap();
    store
        .put_user_record(
            "plan_items",
            json!({
                "kind":"plan-item", "courseId":first_course, "offeringId":"202600041",
                "termId":"2026-1Q", "state":"planned"
            }),
        )
        .unwrap();
    let hidden = set_lms_timetable(&first_course, false);
    let scheduled = |workspace: &Value| crate::schedule::scheduled_courses(workspace, "2026-1Q");
    assert_eq!(
        scheduled(&hidden.workspace)[0]["meetings"][0]["days"],
        json!([1])
    );
    let failed_edit = service.apply(crate::workspace::WorkspaceMutation {
        expected_revision: hidden.revision,
        intent: crate::workspace::WorkspaceIntent::EditCourseBatch {
            course_ids: vec![first_course.clone()],
            changes: crate::workspace::CourseBatchChanges {
                credits: Some(4.0),
                ..Default::default()
            },
        },
    });
    assert!(matches!(
        failed_edit,
        Err(crate::data::DataError::InvalidInput(_))
    ));
    let mut current_grid = lms.clone();
    current_grid.timetables = vec![first_grid.clone()];
    let grid = &mut current_grid.timetables[0];
    grid.meetings.remove(&41);
    grid.unscheduled.push(41);
    ingest(&current_grid);
    assert_eq!(
        scheduled(&store.load_workspace().unwrap())[0]["meetings"],
        json!([])
    );
    current_grid.timetables[0].unscheduled.clear();
    ingest(&current_grid);
    assert_eq!(
        scheduled(&store.load_workspace().unwrap())[0]["meetings"][0]["days"],
        json!([4])
    );
    current_grid.timetables = vec![first_grid];
    ingest(&current_grid);
    assert_eq!(
        scheduled(&store.load_workspace().unwrap())[0]["meetings"][0]["days"],
        json!([1])
    );
    // Maintenance/unavailable extraction does not manufacture authoritative emptiness.
    let mut unavailable = registration(json!(["1Q", "2Q"]), json!([]));
    unavailable.status = "unavailable".into();
    unavailable.message = "Registration is closed".into();
    assert!(store
        .write(None, |tx| apply_kyomu_payload(tx, &unavailable))
        .is_err());
    // Recorded instead, the page is current but unavailable and its rows
    // stay; the views show both under the keys the shells read.
    store
        .write(None, |tx| record_kyomu_unavailable(tx, &unavailable))
        .unwrap();
    let records = store
        .view(&["freshness", "pageAvailability"].into_iter().collect())
        .unwrap()
        .0;
    let availability = &records["pageAvailability"]["kyomu:registrations"];
    assert_eq!(availability["state"], "unavailable");
    assert_eq!(availability["message"], "Registration is closed");
    let freshness = &records["freshness"]["kyomu.page/registrations"];
    assert_eq!(freshness["outcome"], "unavailable");
    assert_eq!(freshness["fetchedAt"], availability["checkedAt"]);
    assert_eq!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-1Q").len(),
        1
    );
}
