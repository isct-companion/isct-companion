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
