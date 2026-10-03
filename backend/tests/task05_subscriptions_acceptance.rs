#[path = "../src/data/mod.rs"]
mod data;
#[path = "../src/subscriptions.rs"]
mod subscriptions;

use rusqlite::Connection;
use serde_json::Value;
use subscriptions::{
    apply_subscription, bundled_2026, bundled_selection, validate_academic_calendar,
    validate_departments, BUNDLED_ACADEMIC_CALENDAR_2026, BUNDLED_DEPARTMENTS_2025,
    BUNDLED_DEPARTMENTS_2026, BUNDLED_INDEX,
};
use url::Url;

#[test]
fn official_bundled_files_validate_and_include_the_documented_cs_course() {
    let (calendar, departments) = bundled_2026().expect("official bundled data must validate");
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
    assert!(error.contains("does not match admissionYear"), "{error}");
}

#[test]
fn admission_year_2025_selects_only_the_real_guide_2025_dataset() {
    let index: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
    assert_eq!(
        index["admissionYears"]["2025"]["departments"],
        "data/2025/departments.json"
    );
    assert_ne!(
        index["admissionYears"]["2025"]["departments"],
        index["admissionYears"]["2026"]["departments"]
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
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
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
    let (calendar, departments) = bundled_2026().expect("official bundled data");
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
