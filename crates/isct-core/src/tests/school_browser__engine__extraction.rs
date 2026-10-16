use super::*;
use uuid::Uuid;

#[test]
fn kyomu_unavailable_and_invalid_payloads_preserve_existing_page_rows() {
    let path = std::env::temp_dir().join(format!("isct-kyomu-preserve-{}.db", Uuid::new_v4()));
    let store = DataStore::initialize(path).unwrap().0;
    let source = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx";
    let ready = serde_json::json!({
        "status":"ok","sourceUrl":source,"kind":"grades",
        "data":{
            "complete":true,"shape":"undergraduate-grouped","empty":false,
            "groups":[{"category":"文系教養科目","rows":[{
                "title":"社会と技術","code":"LAH.H101","result":"90","completed":"2026 1Q"
            }]}]
        }
    });
    let outcome = apply_raw(&store, SchoolSite::Kyomu, &ready.to_string()).unwrap();
    assert_eq!(outcome.state, IngestState::Ok);
    assert_eq!(store.table_rows("grades").unwrap().len(), 1);

    let unavailable = serde_json::json!({
        "status":"unavailable","sourceUrl":source,"kind":"grades",
        "message":"このページは利用できません","data":{}
    });
    let outcome = apply_raw(&store, SchoolSite::Kyomu, &unavailable.to_string()).unwrap();
    assert_eq!(outcome.state, IngestState::Unavailable);
    assert_eq!(store.table_rows("grades").unwrap().len(), 1);
    let (_, records, _) = store.workspace_snapshot_parts().unwrap();
    assert_eq!(
        records["pageAvailability"]["kyomu:grades"]["state"],
        "unavailable"
    );
    assert!(records["lastRuns"]["kyomu:grades"].is_string());

    let invalid = serde_json::json!({
        "status":"ok","sourceUrl":source,"kind":"grades",
        "data":{"complete":true,"shape":"future-layout","empty":true,"groups":[]}
    });
    let outcome = apply_raw(&store, SchoolSite::Kyomu, &invalid.to_string()).unwrap();
    assert_eq!(outcome.state, IngestState::Error);
    assert_eq!(store.table_rows("grades").unwrap().len(), 1);
}
