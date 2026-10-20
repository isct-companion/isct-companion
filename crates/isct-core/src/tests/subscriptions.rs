use super::*;
use crate::data::{commit_on, migrations};
use rusqlite::Connection;

const FIXTURE_INDEX: &str = include_str!("../../tests/fixtures/subscriptions/index.json");
const FIXTURE_INDEX_HASH: &str = include_str!("../../tests/fixtures/subscriptions/index.sha256");
const FIXTURE_ACADEMIC_CALENDAR_2026: &str =
    include_str!("../../tests/fixtures/subscriptions/data/2026/academic-calendar.json");
const FIXTURE_DEPARTMENTS_2025: &str =
    include_str!("../../tests/fixtures/subscriptions/data/2025/departments.json");
const FIXTURE_DEPARTMENTS_2026: &str =
    include_str!("../../tests/fixtures/subscriptions/data/2026/departments.json");

fn source() -> Value {
    json!([{
        "id": "official",
        "title": "Official notice",
        "url": "https://example.com/exam.pdf"
    }])
}

#[test]
fn frozen_fixture_manifest_hashes_match_every_source() {
    assert_eq!(
        format!("{:x}", Sha256::digest(FIXTURE_INDEX.as_bytes())),
        FIXTURE_INDEX_HASH.trim()
    );
    let index: SubscriptionIndex = serde_json::from_str(FIXTURE_INDEX).unwrap();
    let year = index.academic_years.get("2026").unwrap();
    let cohort_2025 = index.admission_years.get("2025").unwrap();
    let cohort_2026 = index.admission_years.get("2026").unwrap();
    let actual = [
        content_hash(&serde_json::from_str(FIXTURE_ACADEMIC_CALENDAR_2026).unwrap()).unwrap(),
        content_hash(&serde_json::from_str(FIXTURE_DEPARTMENTS_2025).unwrap()).unwrap(),
        content_hash(&serde_json::from_str(FIXTURE_DEPARTMENTS_2026).unwrap()).unwrap(),
    ];
    let expected = [
        year.academic_calendar.hash.as_str(),
        cohort_2025.departments.hash.as_str(),
        cohort_2026.departments.hash.as_str(),
    ];
    assert_eq!(
        actual.iter().map(String::as_str).collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn exam_and_exam_week_class_feeds_validate_and_store_apart() {
    let feed_json = |kind: &str, timing: Value| {
        let mut entry = json!({
            "id": "row-one",
            "kind": kind,
            "status": "scheduled",
            "quarter": "2Q",
            "courseCode": "CSC.T241",
            "date": "2026-07-29",
            "room": {"ja": "W9-101"},
            "label": {"ja": "試験"},
            "sourceIds": ["official"],
            "sourcePages": [1]
        });
        entry
            .as_object_mut()
            .unwrap()
            .extend(timing.as_object().unwrap().clone());
        json!({
            "schemaVersion": 1,
            "academicYear": 2026,
            "timezone": "Asia/Tokyo",
            "source": source(),
            "entries": [entry]
        })
        .to_string()
    };
    let periods = json!({"periods": [3, 4]});
    let exact = json!({"startTime": "10:45", "endTime": "12:25"});
    let exams = ExamFeedKind::Exams;
    let classes = ExamFeedKind::ExamWeekClasses;
    // Each dataset accepts only its own row kind; a class row needs periods.
    assert!(validate_exam_feed(&feed_json("class", periods.clone()), 2026, "2q", exams).is_err());
    assert!(validate_exam_feed(&feed_json("exam", periods.clone()), 2026, "2q", classes).is_err());
    assert!(validate_exam_feed(&feed_json("class", exact.clone()), 2026, "2q", classes).is_err());
    assert!(validate_exam_feed(&feed_json("exam", exact), 2026, "2q", exams).is_ok());
    assert_eq!(
        exam_scope("exam-week-classes/graduate/2026-2q.json", 2026, classes),
        Some(("graduate".into(), "2q".into()))
    );
    assert_eq!(
        exam_scope("exam-week-classes/graduate/2026-2q.json", 2026, exams),
        None
    );
    assert_eq!(
        exam_scope("exam/graduate/2026-2q.json", 2026, classes),
        None
    );

    let published = |kind, raw: String| PublishedExamFeed {
        kind,
        level: "undergraduate".into(),
        quarter: "2q".into(),
        url: "https://example.com/feed.json".into(),
        feed: validate_exam_feed(&raw, 2026, "2q", kind).unwrap(),
    };
    let exam = published(exams, feed_json("exam", periods.clone()));
    let class = published(classes, feed_json("class", periods));
    let mut connection = migrations::open_in_memory().unwrap();
    assert_eq!(
        commit_on(&mut connection, |tx| apply_exam_feed(tx, &exam))
            .map(|(changed, _)| changed)
            .unwrap(),
        1
    );
    assert_eq!(
        commit_on(&mut connection, |tx| apply_exam_feed(tx, &class))
            .map(|(changed, _)| changed)
            .unwrap(),
        1
    );
    // Re-applying one dataset must not mark the other's rows missing, even
    // when both use the same entry id.
    commit_on(&mut connection, |tx| apply_exam_feed(tx, &exam))
        .map(|(changed, _)| changed)
        .unwrap();
    let rows: Vec<Value> = connection
        .prepare("SELECT value FROM exams WHERE missing_since IS NULL ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|raw| serde_json::from_str(&raw.unwrap()).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["quarter"], "2Q");
    assert_eq!(
        rows.iter()
            .map(|row| (
                row["kind"].as_str().unwrap(),
                row["sourceKey"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            (
                "class",
                "reference:exam-week-classes:2026:undergraduate:2q:row-one"
            ),
            ("exam", "reference:exams:2026:undergraduate:2q:row-one"),
        ]
    );
}

#[test]
fn validates_important_date_ranges_and_sources() {
    let entry = json!({
        "id": "festival",
        "startDate": "2026-11-01",
        "endDate": "2026-11-02",
        "label": {"ja": "工大祭"},
        "category": "campus-festival",
        "sourceIds": ["official"]
    });
    let feed = |entries: Value| {
        json!({
            "schemaVersion": 1,
            "academicYear": 2026,
            "timezone": "Asia/Tokyo",
            "source": source(),
            "entries": entries
        })
        .to_string()
    };
    let value = validate_important_dates(&feed(json!([entry])), 2026).unwrap();
    assert_eq!(value.entries.len(), 1);
    assert!(validate_important_dates(&feed(json!([entry])), 2025).is_err());

    let with = |field: &str, replacement: Value| {
        let mut changed = entry.clone();
        changed[field] = replacement;
        feed(json!([changed]))
    };
    for (case, raw) in [
        ("reversed range", with("startDate", json!("2026-11-03"))),
        (
            "outside the academic year",
            with("endDate", json!("2027-04-01")),
        ),
        ("unknown source", with("sourceIds", json!(["unlisted"]))),
        ("no source", with("sourceIds", json!([]))),
        ("unknown category", with("category", json!("holiday"))),
        ("blank label", with("label", json!({"ja": " "}))),
        ("duplicate id", feed(json!([entry, entry]))),
    ] {
        assert!(validate_important_dates(&raw, 2026).is_err(), "{case}");
    }
}

fn fixture_selection(
    academic_year: i32,
    admission_year: i32,
) -> Result<(AcademicCalendar, Departments), SubscriptionError> {
    if academic_year != 2026 {
        return Err(format!("no fixture academic calendar for {academic_year}").into());
    }
    let calendar = validate_academic_calendar(FIXTURE_ACADEMIC_CALENDAR_2026)?;
    let departments = match admission_year {
        2025 => validate_departments(FIXTURE_DEPARTMENTS_2025)?,
        2026 => validate_departments(FIXTURE_DEPARTMENTS_2026)?,
        _ => return Err(format!("no fixture guide for admission year {admission_year}").into()),
    };
    Ok((calendar, departments))
}

#[test]
fn frozen_subscription_fixtures_validate_and_include_the_documented_cs_course() {
    let (calendar, departments) =
        fixture_selection(2026, 2026).expect("frozen official data must validate");
    assert_eq!(calendar.academic_year, 2026);
    assert_eq!(departments.departments.len(), 17);
    let cs = departments
        .departments
        .iter()
        .find(|department| department.id == "cs")
        .expect("Computer Science department");
    assert!(cs.recommended.iter().any(|course| course.code == "CSC.T241"
        && course.group == "A"
        && matches!(course.mark, Some(RecommendationMark::Recommended))));
}

#[test]
fn calendar_validation_rejects_missing_fields_bad_dates_and_unknown_types() {
    let official: Value =
        serde_json::from_str(FIXTURE_ACADEMIC_CALENDAR_2026).expect("fixture JSON");

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
    let mut official: Value = serde_json::from_str(FIXTURE_DEPARTMENTS_2026).expect("fixture JSON");
    official["departments"][0]
        .as_object_mut()
        .unwrap()
        .remove("syllabusScope");
    assert!(validate_departments(&official.to_string()).is_err());
}

#[test]
fn department_guide_must_match_its_declared_admission_cohort() {
    let mut official: Value =
        serde_json::from_str(FIXTURE_DEPARTMENTS_2026).expect("official 2026 guide data");
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
    let index: Value = serde_json::from_str(FIXTURE_INDEX).unwrap();
    assert_eq!(
        index["admissionYears"]["2025"]["departments"]["source"],
        "data/2025/departments.json"
    );
    assert_ne!(
        index["admissionYears"]["2025"]["departments"]["source"],
        index["admissionYears"]["2026"]["departments"]["source"]
    );

    let (calendar, departments) = fixture_selection(2026, 2025).unwrap();
    assert_eq!(calendar.academic_year, 2026);
    assert_eq!(departments.admission_year, 2025);
    assert_eq!(departments.departments.len(), 17);
    for source in &departments.source {
        let url = url::Url::parse(&source.url).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("www.titech.ac.jp"));
        assert!(url.path().starts_with("/guide/guide_2025/gakubu1/"));
        assert!(!url.path().contains("guide_2026"));
    }
    for department in &departments.departments {
        let url = url::Url::parse(&department.guide_pdf).unwrap();
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

    // The frozen fixture bytes themselves must carry the same audited cohort.
    let raw: Value = serde_json::from_str(FIXTURE_DEPARTMENTS_2025).unwrap();
    assert_eq!(raw["admissionYear"], 2025);
}

#[test]
fn stored_recommendations_are_namespaced_and_tagged_by_admission_cohort() {
    let (calendar, departments_2025) = fixture_selection(2026, 2025).unwrap();
    let (_, departments_2026) = fixture_selection(2026, 2026).unwrap();
    let mut connection = migrations::open_in_memory().expect("both schemas");
    commit_on(&mut connection, |tx| {
        apply_reference(tx, Some(&calendar), Some(&departments_2025))
    })
    .map(|(changed, _)| changed)
    .unwrap();
    commit_on(&mut connection, |tx| {
        apply_reference(tx, Some(&calendar), Some(&departments_2026))
    })
    .map(|(changed, _)| changed)
    .unwrap();

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
    let (calendar, departments) = fixture_selection(2026, 2026).expect("official fixture data");
    let mut connection = migrations::open_in_memory().expect("both schemas");

    assert!(
        commit_on(&mut connection, |tx| apply_reference(
            tx,
            Some(&calendar),
            Some(&departments)
        ))
        .map(|(changed, _)| changed)
        .unwrap()
            > 0
    );
    assert_eq!(
        commit_on(&mut connection, |tx| apply_reference(
            tx,
            Some(&calendar),
            Some(&departments)
        ))
        .map(|(changed, _)| changed)
        .unwrap(),
        0,
        "the same source hash must not rewrite rows"
    );

    let mut changed = calendar.clone();
    changed.days[0].label.en = Some("Updated official label".into());
    assert_eq!(
        commit_on(&mut connection, |tx| apply_reference(
            tx,
            Some(&changed),
            Some(&departments)
        ))
        .map(|(changed, _)| changed)
        .unwrap(),
        1,
        "one changed calendar day must produce one changed row"
    );
}

#[test]
fn calendar_v2_preserves_disjoint_teaching_ranges_and_rejects_exam_overlap() {
    let mut raw: Value = serde_json::from_str(FIXTURE_ACADEMIC_CALENDAR_2026).unwrap();
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
    let (calendar, departments) = fixture_selection(2026, 2026).unwrap();
    let mut connection = migrations::open_in_memory().unwrap();
    commit_on(&mut connection, |tx| {
        apply_reference(tx, Some(&calendar), Some(&departments))
    })
    .map(|(changed, _)| changed)
    .unwrap();
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
    commit_on(&mut connection, |tx| {
        apply_reference(tx, Some(&calendar), None)
    })
    .map(|(changed, _)| changed)
    .unwrap();
    assert_eq!(count(&connection), before);
    assert_eq!(
        commit_on(&mut connection, |tx| apply_reference(
            tx,
            None,
            Some(&departments)
        ))
        .map(|(changed, _)| changed)
        .unwrap(),
        0
    );
}

#[test]
fn curriculum_map_courses_do_not_require_invented_groups_or_credits() {
    let mut raw: Value = serde_json::from_str(FIXTURE_DEPARTMENTS_2026).unwrap();
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
