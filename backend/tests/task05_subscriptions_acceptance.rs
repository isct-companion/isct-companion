use isct_core::data;
use isct_core::subscriptions;

use rusqlite::Connection;
use serde_json::Value;
use subscriptions::{
    apply_subscription, bundled_selection, validate_academic_calendar, validate_departments,
    BUNDLED_ACADEMIC_CALENDAR_2026, BUNDLED_DEPARTMENTS_2025, BUNDLED_DEPARTMENTS_2026,
    BUNDLED_INDEX,
};
use url::Url;

#[test]
fn official_bundled_files_validate_and_include_the_documented_cs_course() {
    let (calendar, departments) =
        bundled_selection(2026, 2026).expect("official bundled data must validate");
    assert_eq!(calendar.academic_year, 2026);
    assert_eq!(departments.departments.len(), 17);
    let cs = departments
        .departments
        .iter()
        .find(|department| department.id == "cs")
        .expect("Computer Science department");
    assert!(cs
        .recommended
        .iter()
        .any(|course| course.code == "CSC.T241" && course.group == "A"));
}

#[test]
fn calendar_validation_rejects_missing_fields_bad_dates_and_unknown_types() {
    let official: Value =
        serde_json::from_str(BUNDLED_ACADEMIC_CALENDAR_2026).expect("bundled JSON");

    let mut missing = official.clone();
    missing.as_object_mut().unwrap().remove("quarters");
    assert!(validate_academic_calendar(&missing.to_string()).is_err());

    let mut bad_date = official.clone();
    bad_date["quarters"]["1Q"]["start"] = Value::String("2026-13-40".into());
    assert!(validate_academic_calendar(&bad_date.to_string()).is_err());

    let mut unknown_type = official;
    unknown_type["days"][0]["type"] = Value::String("unexpected-closure".into());
    assert!(validate_academic_calendar(&unknown_type.to_string()).is_err());
}

#[test]
fn department_validation_rejects_a_missing_required_field() {
    let mut official: Value = serde_json::from_str(BUNDLED_DEPARTMENTS_2026).expect("bundled JSON");
    official["departments"][0]
        .as_object_mut()
        .unwrap()
        .remove("syllabusScope");
    assert!(validate_departments(&official.to_string()).is_err());
}

#[test]
fn department_guide_must_match_its_declared_admission_cohort() {
    let mut official: Value =
        serde_json::from_str(BUNDLED_DEPARTMENTS_2026).expect("official 2026 guide data");
    assert_eq!(official["admissionYear"], 2026);
    assert!(official["source"][0]["url"]
        .as_str()
        .unwrap()
        .contains("/guide/guide_2026/"));

    // A mismatched cohort label must not silently relabel the real 2026
    // recommendations as 2025 recommendations.
    official["admissionYear"] = 2025.into();
    let error = validate_departments(&official.to_string()).unwrap_err();
    assert!(
        error.to_string().contains("does not match admissionYear"),
        "{error}"
    );
}

#[test]
fn admission_year_2025_selects_only_the_real_guide_2025_dataset() {
    let index: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
    assert_eq!(
        index["admissionYears"]["2025"]["departments"]["source"],
        "data/2025/departments.json"
    );
    assert_ne!(
        index["admissionYears"]["2025"]["departments"]["source"],
        index["admissionYears"]["2026"]["departments"]["source"]
    );

    let (calendar, departments) = bundled_selection(2026, 2025).unwrap();
    assert_eq!(calendar.academic_year, 2026);
    assert_eq!(departments.admission_year, 2025);
    assert_eq!(departments.departments.len(), 17);
    for source in &departments.source {
        let url = Url::parse(&source.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("www.titech.ac.jp"));
        assert!(url.path().starts_with("/guide/guide_2025/gakubu1/"));
        assert!(!url.path().contains("guide_2026"));
    }
    for department in &departments.departments {
        let url = Url::parse(&department.guide_pdf).unwrap();
        assert_eq!(url.host_str(), Some("www.titech.ac.jp"));
        assert!(url.path().starts_with("/guide/guide_2025/gakubu1/pdf/26-"));
        assert!(url.path().ends_with(".pdf"));
        assert!(!url.path().contains("guide_2026"));
    }

    let cs = departments
        .departments
        .iter()
        .find(|department| department.id == "cs")
        .unwrap();
    assert_eq!(
        cs.guide_pdf,
        "https://www.titech.ac.jp/guide/guide_2025/gakubu1/pdf/26-13.pdf"
    );
    assert_eq!(cs.recommended.len(), 39);
    assert!(cs.recommended.iter().any(|course| {
        course.code == "CSC.Z399"
            && course.group == "R"
            && serde_json::to_value(&course.mark).unwrap() == "required"
    }));

    // The bundled bytes themselves must carry the same audited cohort.
    let raw: Value = serde_json::from_str(BUNDLED_DEPARTMENTS_2025).unwrap();
    assert_eq!(raw["admissionYear"], 2025);
}

#[test]
fn stored_recommendations_are_namespaced_and_tagged_by_admission_cohort() {
    let (calendar, departments_2025) = bundled_selection(2026, 2025).unwrap();
    let (_, departments_2026) = bundled_selection(2026, 2026).unwrap();
    let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
    data::migrations::migrate(&mut connection).expect("schema v3");
    apply_subscription(&mut connection, &calendar, &departments_2025).unwrap();
    apply_subscription(&mut connection, &calendar, &departments_2026).unwrap();

    let mut statement = connection
        .prepare(
            "SELECT id,value FROM recommended_courses WHERE code='CSC.T241' AND course_group='A'",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2, "both real cohorts must remain stored");
    for (expected_year, expected_guide) in [(2025, "guide_2025"), (2026, "guide_2026")] {
        let expected_id = format!("cohort:{expected_year}:recommended:cs:CSC.T241:A");
        let (_, raw) = rows.iter().find(|(id, _)| id == &expected_id).unwrap();
        let value: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(value["admissionYear"], expected_year);
        assert_eq!(value["departmentId"], "cs");
        assert!(value["provenance"]["guidePdf"]
            .as_str()
            .unwrap()
            .contains(&format!("/guide/{expected_guide}/gakubu1/")));
    }
}

#[test]
fn subscription_update_is_idempotent_and_changes_only_the_edited_day() {
    let (calendar, departments) = bundled_selection(2026, 2026).expect("official bundled data");
    let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
    data::migrations::migrate(&mut connection).expect("schema v3");

    assert!(apply_subscription(&mut connection, &calendar, &departments).unwrap() > 0);
    assert_eq!(
        apply_subscription(&mut connection, &calendar, &departments).unwrap(),
        0,
        "the same source hash must not rewrite rows"
    );

    let mut changed = calendar.clone();
    changed.days[0].label.en = Some("Updated official label".into());
    assert_eq!(
        apply_subscription(&mut connection, &changed, &departments).unwrap(),
        1,
        "one changed calendar day must produce one changed row"
    );
}

#[test]
fn calendar_v2_preserves_disjoint_teaching_ranges_and_rejects_exam_overlap() {
    let mut raw: Value = serde_json::from_str(BUNDLED_ACADEMIC_CALENDAR_2026).unwrap();
    raw["schemaVersion"] = 2.into();
    raw["teachingRanges"] = serde_json::json!({});
    for q in ["1Q", "2Q", "3Q", "4Q"] {
        raw["teachingRanges"][q] = serde_json::json!([raw["quarters"][q].clone()]);
    }
    raw["teachingRanges"]["1Q"] = serde_json::json!([
        {"start":"2026-04-07","end":"2026-05-27"},
        {"start":"2026-05-30","end":"2026-06-01"},
        {"start":"2026-06-03","end":"2026-06-03"}
    ]);
    let calendar = validate_academic_calendar(&raw.to_string()).unwrap();
    assert_eq!(calendar.teaching_ranges["1Q"].len(), 3);
    raw["teachingRanges"]["1Q"][1]["start"] = "2026-05-27".into();
    assert!(validate_academic_calendar(&raw.to_string()).is_err());
}

#[test]
fn independent_calendar_refresh_does_not_remove_guide_recommendations() {
    let (calendar, departments) = bundled_selection(2026, 2026).unwrap();
    let mut connection = Connection::open_in_memory().unwrap();
    data::migrations::migrate(&mut connection).unwrap();
    apply_subscription(&mut connection, &calendar, &departments).unwrap();
    let count = |db: &Connection| {
        db.query_row(
            "SELECT COUNT(*) FROM recommended_courses WHERE deleted_at IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = count(&connection);
    assert!(before > 0);
    subscriptions::apply_reference(&mut connection, Some(&calendar), None).unwrap();
    assert_eq!(count(&connection), before);
    assert_eq!(
        subscriptions::apply_reference(&mut connection, None, Some(&departments)).unwrap(),
        0
    );
}

#[test]
fn curriculum_map_courses_do_not_require_invented_groups_or_credits() {
    let mut raw: Value = serde_json::from_str(BUNDLED_DEPARTMENTS_2026).unwrap();
    raw["departments"][0]["recommended"] = serde_json::json!([{
        "category":{"ja":"科目体系図"}, "level":200, "code":"EXAMPLE.C201",
        "title":{"ja":"体系図掲載科目"}, "sourcePages":[3], "sourceKind":"curriculum-map"
    }]);
    let parsed = validate_departments(&raw.to_string()).unwrap();
    let row = &parsed.departments[0].recommended[0];
    assert!(row.group.is_empty());
    assert!(row.credits.is_none());
    assert!(row.mark.is_none());
    assert_eq!(row.source_pages, [3]);
    assert_eq!(row.source_kind.as_deref(), Some("curriculum-map"));
    raw["departments"][0]["recommended"][0]["sourcePages"] = serde_json::json!([0]);
    assert!(validate_departments(&raw.to_string()).is_err());
}
