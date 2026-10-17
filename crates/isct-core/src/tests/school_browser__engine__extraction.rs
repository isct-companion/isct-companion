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

    let maintenance = serde_json::json!({
        "status":"maintenance","sourceUrl":"https://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx","kind":"unknown",
        "message":"ただいまメンテナンス中です 2031/01/02 10:00〜12:30","data":{}
    });
    let outcome = apply_raw(&store, SchoolSite::Kyomu, &maintenance.to_string()).unwrap();
    assert_eq!(outcome.state, IngestState::Maintenance);
    assert!(outcome.state.is_problem());
    assert_eq!(outcome.changed, 0);
    assert!(outcome.message.contains("2031/01/02"));
    assert_eq!(store.table_rows("grades").unwrap().len(), 1);
    let (_, after, _) = store.workspace_snapshot_parts().unwrap();
    assert_eq!(after["pageAvailability"], records["pageAvailability"]);
    assert_eq!(after["lastRuns"], records["lastRuns"]);
    for source in [
        "http://kyomu0.gakumu.titech.ac.jp/Maintenance.aspx",
        "https://example.org/Maintenance.aspx",
    ] {
        let mut untrusted = maintenance.clone();
        untrusted["sourceUrl"] = source.into();
        assert!(parse_kyomu_payload(&untrusted.to_string()).is_err());
    }

    let invalid = serde_json::json!({
        "status":"ok","sourceUrl":source,"kind":"grades",
        "data":{"complete":true,"shape":"future-layout","empty":true,"groups":[]}
    });
    let outcome = apply_raw(&store, SchoolSite::Kyomu, &invalid.to_string()).unwrap();
    assert_eq!(outcome.state, IngestState::Error);
    assert_eq!(outcome.kind, "grades");
    assert_eq!(store.table_rows("grades").unwrap().len(), 1);

    // A page the extractor rejects is reported against that page with the
    // selector's reason, and stays stale. Before, every such failure read
    // "unknown: kyomu extraction was incomplete", hiding which page and why.
    for (path, kind, reason) in [
        (
            "%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
            "registrations",
            "The registration page shape is unknown.",
        ),
        (
            "%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
            "requirements",
            "An unknown requirement table is present.",
        ),
    ] {
        let source = format!("https://kyomu0.gakumu.titech.ac.jp/Titech/Student/{path}");
        let rejected = serde_json::json!({
            "status":"error","sourceUrl":source,"kind":kind,"message":reason,"data":{}
        });
        let outcome = apply_raw(&store, SchoolSite::Kyomu, &rejected.to_string()).unwrap();
        assert_eq!(outcome.state, IngestState::Error);
        assert_eq!(outcome.kind, kind);
        assert_eq!(
            outcome.message,
            format!("kyomu extraction was incomplete: {reason}")
        );
        let runs = store.last_import_runs().unwrap();
        assert!(runs[format!("kyomu:{kind}")].is_null());

        // An explicit unavailability notice on the same page is a complete
        // observation, so the refresh policy may treat the page as fresh.
        let unavailable = serde_json::json!({
            "status":"unavailable","sourceUrl":source,"kind":kind,
            "message":"このページは利用できません","data":{}
        });
        let outcome = apply_raw(&store, SchoolSite::Kyomu, &unavailable.to_string()).unwrap();
        assert_eq!(outcome.state, IngestState::Unavailable);
        assert!(store.last_import_runs().unwrap()[format!("kyomu:{kind}")].is_string());
    }

    // The kind is taken from the approved URL, never from the payload's claim.
    let foreign = serde_json::json!({
        "status":"error","sourceUrl":"https://example.org/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
        "kind":"requirements","message":"An unknown requirement table is present.","data":{}
    });
    let outcome = apply_raw(&store, SchoolSite::Kyomu, &foreign.to_string()).unwrap();
    assert_eq!(outcome.kind, "unknown");
}

#[test]
fn lms_partial_quarter_reads_commit_available_data_and_preserve_cached_clocks() {
    let path = std::env::temp_dir().join(format!("isct-lms-partial-{}.db", Uuid::new_v4()));
    let store = DataStore::initialize(path).unwrap().0;
    let mut raw = serde_json::json!({
        "status":"ok","sourceUrl":"https://lms.s.isct.ac.jp/2025/my/",
        "coursesComplete":true,"fullScan":true,
        "courses":[{"id":41,"viewurl":"https://lms.s.isct.ac.jp/2025/course/view.php?id=41","fullname":"Original","shortname":"CSC.T101[2026]","coursecategory":1}],
        "timetables":[{"year":2026,"quarter":"1","slots":[{"courseId":41,"day":1,"period":3}],"unscheduled":[]}]
    });
    assert_eq!(
        apply_raw(&store, SchoolSite::ScienceLms, &raw.to_string())
            .unwrap()
            .state,
        IngestState::Ok
    );
    raw["courses"][0]["fullname"] = "Updated".into();
    raw["timetables"] = serde_json::json!([]);
    raw["timetableErrors"] = serde_json::json!(["2026-1Q: timeout"]);
    let partial = apply_raw(&store, SchoolSite::ScienceLms, &raw.to_string()).unwrap();
    assert_eq!(partial.state, IngestState::Error);
    assert!(partial.changed > 0);
    let courses = crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-1Q");
    assert_eq!(courses.len(), 1);
    assert_eq!(courses[0]["meetings"][0]["days"], serde_json::json!([1]));
    raw["timetables"] =
        serde_json::json!([{"year":2026,"quarter":"1","slots":[],"unscheduled":[]}]);
    raw["timetableErrors"] = serde_json::json!([]);
    assert_eq!(
        apply_raw(&store, SchoolSite::ScienceLms, &raw.to_string())
            .unwrap()
            .state,
        IngestState::Ok
    );
    assert!(
        crate::schedule::scheduled_courses(&store.load_workspace().unwrap(), "2026-1Q").is_empty()
    );
}
