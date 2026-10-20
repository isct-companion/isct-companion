use super::*;
use serde_json::json;
#[tokio::test]
async fn cached_bilingual_details_preserve_wire_and_do_not_fetch() {
    let root = std::env::temp_dir().join(format!("isct-catalog-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = data::DataStore::initialize(root.join("school.db"))
        .unwrap()
        .0;
    let now = chrono::Utc::now().to_rfc3339();
    let value = json!({
        "id":"202602431", "year":2026, "quarters":["3Q"], "code":"TEST.1",
        "title":{"en":"Test"}, "instructors":[], "instructionLanguage":"en",
        "format":{"kinds":[]}, "media":{}, "credits":{"lecture":2,"exercise":0,"lab":0},
        "schedule":[], "academicUnit":{}, "intensive":false,
        "syllabusUrl":"https://example.invalid/202602431", "fetchedAt":{"ja":now,"en":now}
    });
    let content = ["ja","en"].map(|lang| json!({"offeringId":"202602431","lang":lang,"content":{
            "schemaVersion":1,"sourceUrl":"https://example.invalid/202602431","sourceLanguage":lang,
            "fetchedAt":now,"sourceUpdated":null,"metadata":[],"sections":[],"lecturePlan":[],"competencies":[]
        }}));
    let key = detail_key("202602431");
    let read_at = Utc::now();
    store
        .catalog_store_detail(
            &value,
            &content,
            &RunRecord {
                run: "seed",
                key: &key,
                account: "",
                started_at: read_at,
                finished_at: read_at,
                outcome: RunOutcome::Complete,
                changed: 3,
                requests: None,
                bytes: None,
                source: "example.invalid/202602431",
                error_kind: None,
            },
        )
        .unwrap();
    let result = catalog_sync_offering(&store, "https://example.invalid/202602431".into())
        .await
        .unwrap();
    assert_eq!(result.offering.id, "202602431");
    assert_eq!(result.content.len(), 2);
    assert!(result.warnings.is_empty());
    assert_eq!(offerings(&store, 2026).unwrap().len(), 1);
    assert!(offerings(&store, 2025).unwrap().is_empty());
    let warnings = catalog_sync_offerings(&store, vec!["202602431".into(), "missing".into()])
        .await
        .unwrap();
    assert_eq!(warnings, vec!["missing: not in the catalog"]);
    assert!(matches!(
        catalog_sync_offerings(&store, vec![String::new(); 21]).await,
        Err(CatalogError::InvalidInput(_))
    ));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn official_delivery_facts_survive_syllabus_enrichment_in_both_arrival_orders() {
    let syllabus = json!({"id":"202602431","year":2026,"quarters":["3Q"],"code":"CSC.T273",
        "title":{"ja":"Old name","en":"Object-Oriented Programming"},"instructors":[{"en":"Teacher"}],
        "instructionLanguage":"en","format":{"kinds":["lecture"]},"media":{},
        "credits":{"lecture":2,"exercise":0,"lab":0},"schedule":[{"kind":"weekly","day":5,"periods":[5,6],"room":{"en":"Old room"}}],
        "academicUnit":{},"intensive":false,"syllabusUrl":"https://syllabus.s.isct.ac.jp/courses?jwc=202602431","fetchedAt":{}});
    let mut official = syllabus.clone();
    official["title"] = json!({"ja":"オブジェクト指向プログラミング"});
    official["credits"] = json!({"lecture":1,"exercise":1,"lab":0});
    official["schedule"] = json!([{"kind":"weekly","day":1,"periods":[3,4],"room":{"ja":"M-B07"}}]);
    official["kyomu"] = json!({"department":"情報工学系","group":"C","mark":"recommended","fetchedAt":"2026-10-16T00:00:00Z"});
    for first_official in [false, true] {
        let root = std::env::temp_dir().join(format!("isct-source-{}.db", uuid::Uuid::new_v4()));
        let store = data::DataStore::initialize(root).unwrap().0;
        let rows = if first_official {
            vec![official.clone(), syllabus.clone()]
        } else {
            vec![syllabus.clone(), official.clone()]
        };
        for row in rows {
            store.catalog_upsert_offerings(&[row]).unwrap();
        }
        store
            .catalog_upsert_offerings(std::slice::from_ref(&syllabus))
            .unwrap();
        let merged = store.catalog_offering("202602431").unwrap().unwrap();
        assert_eq!(merged["schedule"], official["schedule"]);
        assert_eq!(merged["credits"], official["credits"]);
        assert_eq!(merged["title"]["ja"], official["title"]["ja"]);
        assert_eq!(merged["title"]["en"], syllabus["title"]["en"]);
        assert_eq!(merged["unitCode"], "342300");
        let planned = crate::workspace::planned_catalog_course(
            &merged,
            "planned",
            "2026-3Q",
            "ja",
            &crate::schedule::default_teaching_periods(),
        )
        .unwrap();
        assert_eq!(planned["source"]["provider"], "kyomu");
        assert_eq!(planned["meetings"][0]["room"], "M-B07");
    }
}

#[test]
fn official_filters_are_applied_to_the_returned_department_rows() {
    let row = json!({"id":"202602431","code":"CSC.T273","quarters":["3Q","4Q"],
        "instructionLanguage":"en",
        "schedule":[{"kind":"weekly","day":1,"periods":[3,4]},{"kind":"intensive"}],
        "kyomu":{"group":"C","mark":"recommended","category":""}});
    let pass = |filters: serde_json::Value, controls: &[&OfficialFilterOption]| {
        official_match(&row, &serde_json::from_value(filters).unwrap(), controls)
    };
    assert!(pass(json!({}), &[]));
    assert!(pass(
        json!({"groups":["200"],"quarters":[4],"days":[1],"periodStarts":[3],
            "language":"en","requirementGroups":["C"],"recommended":true}),
        &[]
    ));
    for filters in [
        json!({"groups":["100"]}),
        json!({"quarters":[1]}),
        json!({"days":[2]}),
        json!({"periodStarts":[1]}),
        json!({"language":"ja"}),
        json!({"requirementGroups":["A"]}),
    ] {
        assert!(!pass(filters.clone(), &[]), "{filters}");
    }
    let option = |family: &str, label: &str| OfficialFilterOption {
        id: "control".into(),
        value: "on".into(),
        label: label.into(),
        family: family.into(),
    };
    let (monday, friday) = (option("delivery", "月"), option("delivery", "金"));
    let (first_period, english) = (option("start", "1限"), option("language", "英語"));
    // Choices of one family are alternatives; every family must match.
    assert!(pass(json!({}), &[&friday, &monday, &english]));
    assert!(!pass(json!({}), &[&friday]));
    assert!(!pass(json!({}), &[&monday, &first_period]));
    // A choice whose meaning the rows cannot show keeps them.
    assert!(pass(json!({}), &[&option("other", "何か")]));
}
