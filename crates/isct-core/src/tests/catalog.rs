use super::*;
use serde_json::json;
#[test]
fn english_names_from_the_english_page_survive_japanese_rereads() {
    let japanese = json!({"id":"202602431","year":2026,"quarters":["3Q"],"code":"CSC.T273",
        "title":{"ja":"オブジェクト指向プログラミング"},"instructors":[{"ja":"山田 太郎"},{"ja":"鈴木 花子"}],
        "instructionLanguage":"ja","format":{"kinds":["lecture"]},"media":{},
        "credits":{"lecture":1,"exercise":1,"lab":0},
        "schedule":[{"kind":"weekly","day":1,"periods":[3,4],"room":{"ja":"M-B07"}}],
        "academicUnit":{},"intensive":false,
        "syllabusUrl":crate::catalog::syllabus_url("202602431", "ja"),"fetchedAt":{},
        "kyomu":{"department":"情報工学系","group":"C","mark":"recommended","fetchedAt":"2026-10-16T00:00:00Z"}});
    let english = json!({"id":"202602431","year":2026,"title":{"en":"Object-Oriented Programming"},
        "instructors":[{"en":"Yamada Taro"},{"en":"Suzuki Hanako"}]});
    let unknown = json!({"id":"202699999","year":2026,"title":{"en":"Not stored"},"instructors":[]});
    let root = std::env::temp_dir().join(format!("isct-source-{}", uuid::Uuid::new_v4()));
    let store = data::DataStore::initialize_dir(root.clone()).unwrap().0;
    let merge_english = |rows: &[serde_json::Value]| {
        store
            .write(None, |transaction| {
                data::DataStore::catalog_merge_english_in(transaction, rows)
            })
            .unwrap()
    };
    // English before the Japanese row exists adds nothing.
    assert_eq!(merge_english(std::slice::from_ref(&english)), 0);
    store.catalog_upsert_offerings(std::slice::from_ref(&japanese)).unwrap();
    assert_eq!(merge_english(&[english.clone(), unknown]), 1);
    // The Japanese page read again keeps the English names, and a changed
    // instructor list drops the name it no longer matches.
    let mut reread = japanese.clone();
    reread["instructors"] = json!([{"ja":"山田 太郎"},{"ja":"田中 一郎"}]);
    store.catalog_upsert_offerings(&[reread]).unwrap();
    let merged = store.catalog_offering("202602431").unwrap().unwrap();
    assert_eq!(merged["schedule"], japanese["schedule"]);
    assert_eq!(merged["title"]["ja"], japanese["title"]["ja"]);
    assert_eq!(merged["title"]["en"], "Object-Oriented Programming");
    assert_eq!(
        merged["instructors"],
        json!([{"ja":"山田 太郎","en":"Yamada Taro"},{"ja":"田中 一郎"}])
    );
    assert_eq!(merged["unitCode"], "342300");
    assert_eq!(offerings(&store, 2026).unwrap().len(), 1);
    assert!(offerings(&store, 2025).unwrap().is_empty());
    let planned = crate::workspace::planned_catalog_course(
        &merged,
        "planned",
        "2026-3Q",
        "en",
        &crate::schedule::default_teaching_periods(),
    )
    .unwrap();
    assert_eq!(planned["source"]["provider"], "kyomu");
    assert_eq!(planned["title"], "Object-Oriented Programming");
    assert_eq!(planned["meetings"][0]["room"], "M-B07");
    std::fs::remove_dir_all(root).unwrap();
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
