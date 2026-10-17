use super::*;

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
    let path = std::env::temp_dir().join(format!("isct-grade-id-{}.db", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize(path).unwrap().0;
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
    let mut connection = Connection::open(store.path()).unwrap();
    assert!(apply_kyomu_payload(&mut connection, &payload).is_err());
    assert!(store.table_rows("grades").unwrap().is_empty());
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

/// Protects official timing, quarter boundaries, LMS content linking and user
/// plans together: these are the joins that a source switch can otherwise drop.
#[test]
fn official_enrollment_owns_the_calendar_while_lms_and_personal_plans_survive() {
    let path = std::env::temp_dir().join(format!("isct-official-{}.db", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize(path).unwrap().0;
    store.put_workspace_setting("academicProfile",json!({"name":"Test","gradePath":[],"studyEnrollments":[],"years":[{
        "academicYear":2026,"quarterStarts":{"1Q":"2026-04-01","2Q":"2026-06-01","3Q":"2026-10-01","4Q":"2026-12-01"}
    }]}),"test").unwrap();
    let manual = |id: &str, code: &str| {
        json!({"id":id,"kind":"course","termId":"2026-3Q","code":code,"title":"Personal",
        "instructor":"","credits":2,"creditSplit":{"lecture":2,"exercise":0,"lab":0},"instructionLanguage":"en","color":"#112233",
        "meetings":[{"days":[5],"periods":[5,6],"startTime":"13:30","endTime":"15:10","room":"Old LMS room"}],"source":{"provider":"lms"}})
    };
    let mut original = manual("2026-202602431", "CSC.T273");
    original["offeringId"] = json!("202602431");
    store
        .put_user_record("manual_courses", original, "test")
        .unwrap();
    store
        .put_user_record("manual_courses", manual("personal", "MTH.B301"), "test")
        .unwrap();
    store.put_user_record("plan_items",json!({"id":"personal-plan","kind":"plan-item","courseId":"personal","termId":"2026-3Q","state":"planned"}),"test").unwrap();
    store.put_user_record("manual_memberships",json!({"id":"old-lms","kind":"membership","courseId":"2026-202602431","termId":"2026-3Q","status":"lms-member","evidence":{"provider":"lms"}}),"test").unwrap();
    let mut connection = Connection::open(store.path()).unwrap();
    let registration = |courses: Value, quarters: Value| KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "registrations".into(),
        data: json!({"state":{"state":"registered","raw":"本登録済"},"quarters":quarters,"courses":courses}),
        message: String::new(),
    };
    let row = |offering: &str, code: &str, schedule: Value| json!({"offeringId":offering,"title":"Official course","code":code,"instructor":"Official teacher","schedule":schedule});
    // A previous-semester course must survive the current-semester refresh.
    apply_kyomu_payload(
        &mut connection,
        &registration(
            json!([row(
                "202600001",
                "CSC.T101",
                json!([{"quarter":"1Q","day":"月","periods":[1,2],"room":"A"}])
            )]),
            json!(["1Q", "2Q"]),
        ),
    )
    .unwrap();
    let official = row(
        "202602431",
        "CSC.T273",
        json!([
            {"quarter":"3Q","day":"月","periods":[3,4],"room":"M-B07"},
            {"quarter":"4Q","day":"木","periods":[5,6],"room":"W9-324"}
        ]),
    );
    apply_kyomu_payload(
        &mut connection,
        &registration(json!([official]), json!(["3Q", "4Q"])),
    )
    .unwrap();
    let declaration = KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "declared".into(),
        data: json!({"courses":[
            {"offeringId":"202602431","unit":"group","unitTotal":0.5,"quarters":[3,4],"credits":{"lecture":1,"exercise":0,"lab":0}}
        ]}),
        message: String::new(),
    };
    apply_kyomu_payload(&mut connection, &declaration).unwrap();
    // No jwc is needed for an unambiguous code/year/quarter content join.
    let lms = json!({"id":"lms-course:99","lmsId":99,"shortname":"CSC.T273[2026]","year":2026,"quarters":["3Q"],"title":{"en":"LMS content"}});
    connection.execute("INSERT INTO lms_courses(id,value,hash,updated_at,updated_by,rev) VALUES(?1,?2,'h','now','test',1)",params!["lms-course:99",lms.to_string()]).unwrap();
    let assignment = json!({"id":"lms-assignment:1","kind":"assignment","courseId":"2026-lms-99","lmsCourseId":99,"title":"Task","details":"","dueAt":null,"source":{"provider":"lms"},"rev":1,"updatedAt":"2026-10-01T00:00:00Z","updatedBy":"lms"});
    connection.execute("INSERT INTO assignments(id,value,hash,updated_at,updated_by,rev) VALUES(?1,?2,'h','now','test',1)",params!["lms-assignment:1",assignment.to_string()]).unwrap();
    // A newly projected course must support the same personal overlay as
    // pre-existing courses, while official enrollment remains source-owned.
    let service = crate::workspace::WorkspaceService::new(store.clone());
    let snapshot = service.snapshot().unwrap();
    let saved = service.mutate(crate::workspace::WorkspaceMutation {
        expected_revision: snapshot.revision,
        intent: crate::workspace::WorkspaceIntent::SaveCourse {
            draft: json!({"title":"My course name","instructor":"","color":"#abcdef","onTimetable":true}),
            term_id: "2026-1Q".into(),
            existing_id: Some("2026-202600001".into()),
        },
    }).unwrap();
    let projected = saved.workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "2026-202600001")
        .unwrap();
    assert_eq!(projected["overrides"]["title"], "My course name");
    assert_eq!(projected["overrides"]["color"], "#abcdef");
    let remove = service.mutate(crate::workspace::WorkspaceMutation {
        expected_revision: saved.revision,
        intent: crate::workspace::WorkspaceIntent::SetCourseOnTimetable {
            course_id: "2026-202600001".into(),
            on_timetable: false,
        },
    });
    assert!(matches!(
        remove,
        Err(crate::workspace::WorkspaceError::InvalidIntent(_))
    ));
    let workspace = store.load_workspace().unwrap();
    let courses = crate::schedule::scheduled_courses(&workspace, "2026-3Q");
    assert_eq!(courses.len(), 2);
    let official = courses
        .iter()
        .find(|c| c["id"] == "2026-202602431")
        .unwrap();
    assert_eq!(official["lmsCourseId"], 99);
    assert_eq!(official["color"], "#112233");
    assert_eq!(official["credits"], 0.5);
    assert_eq!(official["meetings"][0]["room"], "M-B07");
    assert_eq!(official["meetings"].as_array().unwrap().len(), 1);
    assert_eq!(workspace["assignments"][0]["courseId"], "2026-202602431");
    assert_eq!(
        crate::schedule::scheduled_courses(&workspace, "2026-1Q").len(),
        1
    );
    let later = crate::schedule::scheduled_courses(&workspace, "2026-4Q");
    assert_eq!(later[0]["meetings"][0]["days"], json!([4]));
    assert_eq!(later[0]["meetings"][0]["room"], "W9-324");
    let calendar =
        crate::schedule::calendar_between(&workspace, "2026-10-05", "2026-10-05").unwrap();
    assert_eq!(calendar[0].occurrences[0].room, "M-B07");
    // Removing the official course cannot resurrect its old LMS membership.
    apply_kyomu_payload(
        &mut connection,
        &registration(json!([]), json!(["3Q", "4Q"])),
    )
    .unwrap();
    let after = store.load_workspace().unwrap();
    assert_eq!(
        crate::schedule::scheduled_courses(&after, "2026-3Q").len(),
        1
    );
    assert_eq!(
        crate::schedule::scheduled_courses(&after, "2026-1Q").len(),
        1
    );
    assert_eq!(store.user_rows("plan_items").unwrap().len(), 1);
    assert_eq!(
        store.user_rows("manual_memberships").unwrap().len(),
        1,
        "source rows remain reversible"
    );
}

#[test]
fn official_academic_identity_fills_a_fresh_profile_without_overwriting_saved_choices() {
    let path = std::env::temp_dir().join(format!("isct-academic-id-{}.db", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize(path).unwrap().0;
    let mut connection = Connection::open(store.path()).unwrap();
    let payload = KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "profile".into(),
        data: json!({"department":"情報工学系","admittedOn":"2025-04-01","programme":"学士課程","entryCategory":"前期日程"}),
        message: String::new(),
    };
    apply_kyomu_payload(&mut connection, &payload).unwrap();
    let initial = store.load_workspace().unwrap();
    assert_eq!(initial["settings"]["departmentId"], "cs");
    assert_eq!(initial["academicProfile"]["admissionYear"], 2025);
    assert_eq!(
        initial["academicProfile"]["studyEnrollments"][0],
        json!({"stage":"undergraduate","entryKind":"regular","entryGrade":1,"admittedTerm":"2025-1Q"})
    );
    store
        .put_workspace_setting("userPreferences", json!({"departmentId":"mcs"}), "test")
        .unwrap();
    let mut profile = initial["academicProfile"].clone();
    profile["admissionYear"] = json!(2024);
    store
        .put_workspace_setting("academicProfile", profile, "test")
        .unwrap();
    apply_kyomu_payload(&mut connection, &payload).unwrap();
    let updated = store.load_workspace().unwrap();
    assert_eq!(updated["settings"]["departmentId"], "mcs");
    assert_eq!(updated["academicProfile"]["admissionYear"], 2024);
}

/// Past quarters absent from the current registration page still have LMS
/// enrollment. Projection must join clocks/content and retain local overlays.
#[test]
fn uncovered_quarters_use_lms_until_official_enrollment_becomes_available() {
    let path = std::env::temp_dir().join(format!("isct-lms-fallback-{}.db", Uuid::new_v4()));
    let store = crate::data::DataStore::initialize(path).unwrap().0;
    let mut connection = Connection::open(store.path()).unwrap();
    let registration = |quarters: Value, courses: Value| KyomuPayload {
        status: "ok".into(),
        source_url: String::new(),
        kind: "registrations".into(),
        data: json!({"state":{"state":"registered"},"quarters":quarters,"courses":courses}),
        message: String::new(),
    };
    apply_kyomu_payload(
        &mut connection,
        &registration(json!(["3Q", "4Q"]), json!([])),
    )
    .unwrap();
    let mut lms: crate::school_parser::lms::LmsPayload = serde_json::from_value(json!({
        "status":"ok","sourceUrl":"https://lms.s.isct.ac.jp/2025/my/","language":"en",
        "courses":[
            {"id":41,"idnumber":"202600041","shortname":"CSC.T101[2026]","fullname":"前期 / Earlier","coursecategory":1},
            {"id":42,"shortname":"CSC.T102[2026]","fullname":"後半 / Second","coursecategory":2},
            {"id":43,"shortname":"CSC.T103[2026]","fullname":"Optional","coursecategory":3},
            {"id":44,"shortname":"Staff[2026]","fullname":"Unplaced","coursecategory":0}
        ],"timetable":{"term":"2026-1Q","meetings":{"41":[{"day":1,"periods":[3,4]}]},"unscheduled":[]}
    })).unwrap();
    crate::lms_ingest::apply_lms_payload(&mut connection, &lms).unwrap();
    lms.timetable = Some(crate::school_parser::lms::LmsTimetable {
        term: "2026-2Q".into(),
        meetings: std::collections::HashMap::from([(42, vec![json!({"day":2,"periods":[5,8]})])]),
        unscheduled: vec![],
    });
    crate::lms_ingest::apply_lms_payload(&mut connection, &lms).unwrap();
    let workspace = store.load_workspace().unwrap();
    let first = crate::schedule::scheduled_courses(&workspace, "2026-1Q");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["id"], "2026-202600041");
    assert_eq!(first[0]["lmsCourseId"], 41);
    assert_eq!(first[0]["meetings"][0]["startTime"], "10:45");
    assert_eq!(first[0]["meetings"][0]["termIds"], json!(["2026-1Q"]));
    let second = crate::schedule::scheduled_courses(&workspace, "2026-2Q");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0]["meetings"][0]["periods"], json!([5, 6, 7, 8]));
    assert!(crate::schedule::scheduled_courses(&workspace, "2026-3Q").is_empty());
    assert!(!workspace["courses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["lmsCourseId"] == 44));
    // A source projection is ephemeral, never a write to the user's tables.
    assert!(store.user_rows("manual_courses").unwrap().is_empty());
    assert!(store.user_rows("manual_memberships").unwrap().is_empty());
    let service = crate::workspace::WorkspaceService::new(store.clone());
    let snapshot = service.snapshot().unwrap();
    service
        .mutate(crate::workspace::WorkspaceMutation {
            expected_revision: snapshot.revision,
            intent: crate::workspace::WorkspaceIntent::SaveCourse {
                draft: json!({"title":"My earlier course","color":"#abcdef","onTimetable":true}),
                term_id: "2026-1Q".into(),
                existing_id: Some("2026-202600041".into()),
            },
        })
        .unwrap();
    let snapshot = service.snapshot().unwrap();
    service
        .mutate(crate::workspace::WorkspaceMutation {
            expected_revision: snapshot.revision,
            intent: crate::workspace::WorkspaceIntent::SetCourseOnTimetable {
                course_id: "2026-lms-42".into(),
                on_timetable: false,
            },
        })
        .unwrap();
    crate::lms_ingest::apply_lms_payload(&mut connection, &lms).unwrap();
    assert!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-2Q").is_empty()
    );
    // Later official evidence replaces LMS clocks, never the user's overlay.
    apply_kyomu_payload(&mut connection,&registration(json!(["1Q","2Q"]),json!([{
        "offeringId":"202600041","code":"CSC.T101","title":"Official earlier","instructor":"Teacher",
        "schedule":[{"quarter":"1Q","day":"木","periods":[1,2],"room":"Official room"}]
    }]))).unwrap();
    let upgraded = store.load_workspace().unwrap();
    let first = crate::schedule::scheduled_courses(&upgraded, "2026-1Q");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["source"]["provider"], "kyomu");
    assert_eq!(first[0]["lmsCourseId"], 41);
    assert_eq!(first[0]["overrides"]["title"], "My earlier course");
    assert_eq!(first[0]["overrides"]["color"], "#abcdef");
    assert_eq!(first[0]["meetings"][0]["room"], "Official room");
    assert_eq!(first[0]["meetings"][0]["days"], json!([4]));
    assert!(crate::schedule::scheduled_courses(&upgraded, "2026-2Q").is_empty());
    // Maintenance/unavailable extraction does not manufacture authoritative emptiness.
    let mut unavailable = registration(json!(["1Q", "2Q"]), json!([]));
    unavailable.status = "unavailable".into();
    assert!(apply_kyomu_payload(&mut connection, &unavailable).is_err());
    assert_eq!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-1Q").len(),
        1
    );
}
