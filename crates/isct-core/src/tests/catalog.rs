use super::*;
use serde_json::json;
#[tokio::test]
async fn cached_bilingual_details_preserve_wire_and_do_not_fetch() {
    let root = std::env::temp_dir().join(format!("isct-catalog-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = data::DataStore::initialize_dir(root.clone()).unwrap().0;
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
        let root = std::env::temp_dir().join(format!("isct-source-{}", uuid::Uuid::new_v4()));
        let store = data::DataStore::initialize_dir(root).unwrap().0;
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
fn official_form_choices_are_read_from_each_stored_row() {
    let row = json!({"id":"202602431","code":"CSC.T273","quarters":["3Q","4Q"],
        "instructionLanguage":"en",
        "schedule":[{"kind":"weekly","day":1,"periods":[3,4]},{"kind":"intensive"}],
        "kyomu":{"group":"C","mark":"recommended","category":""}});
    let option = |family: &str, label: &str| OfficialFilterOption {
        id: "control".into(),
        value: "on".into(),
        label: label.into(),
        family: family.into(),
    };
    let pass = |family: &str, label: &str| official_control_match(&row, &option(family, label));
    for (family, label) in [
        ("delivery", "月"),
        ("delivery", "集中講義"),
        ("start", "3限"),
        ("language", "英語"),
        ("level", "200番台"),
        ("quarter", "3~4Q"),
        ("requirement", "C"),
        ("recommended", "推奨"),
    ] {
        assert!(pass(family, label), "{family} {label}");
    }
    for (family, label) in [
        ("delivery", "金"),
        ("start", "1限"),
        ("language", "日本語"),
        ("level", "100番台"),
        ("quarter", "1Q"),
        ("requirement", "A"),
    ] {
        assert!(!pass(family, label), "{family} {label}");
    }
    // A choice whose meaning the rows cannot show keeps them.
    assert!(pass("other", "何か"));
    assert!(pass("category", "研究関連科目"));
}
