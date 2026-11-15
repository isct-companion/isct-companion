use super::*;
use serde_json::json;

const GRADES: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx";
const REQUIREMENTS: &str = "https://kyomu0.gakumu.titech.ac.jp/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx";

fn parse(value: Value) -> Result<(KyomuStage, Option<KyomuPayload>), SchoolParseError> {
    parse_payload(&value.to_string())
}

#[test]
fn accepts_flat_graduate_groups_and_grouped_undergraduate_grades() {
    for (shape, groups) in [
        (
            "graduate-flat",
            json!([
                {"section":"graduate","category":"大学院科目","rows":[{"title":"先端システム論","credits":"2","result":"A","completed":"2026年度 1Q"}]},
                {"section":"undergraduate","category":"学部科目","rows":[{"title":"基礎設計演習","credits":"1","result":"合格","completed":"2025年度 4Q"}]}
            ]),
        ),
        (
            "undergraduate-grouped",
            json!([{"category":"文系教養科目","rows":[{"title":"社会と技術","code":"LAH.H101","result":"90"}]}]),
        ),
    ] {
        let (stage, payload) = parse(json!({
            "status":"ok","sourceUrl":GRADES,"kind":"grades",
            "data":{"complete":true,"shape":shape,"empty":false,"groups":groups}
        }))
        .unwrap();
        assert_eq!(stage, KyomuStage::Ready);
        assert!(payload.is_some());
    }
}

#[test]
fn accepts_explicit_empty_and_more_than_two_requirement_tables() {
    let empty = parse(json!({
        "status":"ok","sourceUrl":GRADES,"kind":"grades",
        "data":{"complete":true,"shape":"graduate-flat","empty":true,"groups":[]}
    }));
    assert!(matches!(empty, Ok((KyomuStage::Ready, Some(_)))));

    let tables = (0..3)
        .map(|index| {
            json!({
                "headers":["区分",format!("列{index}")],
                "rows":[{"label":"修得済単位","values":["1"]}]
            })
        })
        .collect::<Vec<_>>();
    let requirements = parse(json!({
        "status":"ok","sourceUrl":REQUIREMENTS,"kind":"requirements",
        "data":{"complete":true,"shape":"requirements-tables","empty":false,"tables":tables}
    }));
    assert!(matches!(requirements, Ok((KyomuStage::Ready, Some(_)))));
}

#[test]
fn rejects_missing_completeness_collections_and_unknown_shapes() {
    for data in [
        json!({"shape":"graduate-flat","groups":[]}),
        json!({"complete":true,"shape":"graduate-flat","empty":true}),
        json!({"complete":true,"shape":"future-layout","empty":true,"groups":[]}),
        json!({"complete":true,"shape":"graduate-flat","empty":false,"groups":[]}),
        json!({"complete":true,"shape":"graduate-flat","empty":false,"groups":[
            {"section":"graduate","category":"大学院科目","rows":[{"title":"科目"}]}
        ]}),
    ] {
        assert!(parse(json!({
            "status":"ok","sourceUrl":GRADES,"kind":"grades","data":data
        }))
        .is_err());
    }
}

#[test]
fn unavailable_requires_an_approved_page_and_a_reason() {
    let (stage, payload) = parse(json!({
        "status":"unavailable","sourceUrl":REQUIREMENTS,"kind":"requirements",
        "message":"このページは利用できません","data":{}
    }))
    .unwrap();
    assert_eq!(stage, KyomuStage::Unavailable);
    assert_eq!(payload.unwrap().kind, "requirements");
    assert!(parse(json!({
        "status":"unavailable","sourceUrl":REQUIREMENTS,"kind":"requirements","data":{}
    }))
    .is_err());
    assert!(parse(json!({
        "status":"unavailable","sourceUrl":"https://example.invalid/page",
        "kind":"requirements","message":"not offered","data":{}
    }))
    .is_err());
}

/// The requirement tables of one page as `extract_page` reads them.
fn requirement_tables(tables: &str) -> Vec<Value> {
    let html = format!(
        r#"<html><body><div id="ctl00_ContentPlaceHolder1"><div class="tableSet02Tbl">{tables}</div></div></body></html>"#
    );
    let payload = extract_page(REQUIREMENTS, &html);
    assert_eq!(payload.status, "ok", "{}", payload.message);
    // What the page gave passes the same trust boundary as every payload.
    let (stage, _) = parse_payload(&serde_json::to_string(&payload).unwrap()).unwrap();
    assert_eq!(stage, KyomuStage::Ready);
    payload.data["tables"].as_array().unwrap().clone()
}

fn labels(table: &Value) -> Vec<&str> {
    table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["label"].as_str().unwrap())
        .collect()
}

/// The master's 修了要件 page, in the structure of a page saved from 教務Web
/// (1.3.1 failed it with "A requirement table has no recognized rows"): the
/// measures head the columns, each row is a category under a spanning group
/// cell, and the GA earned sit in a headerless table under their own heading.
/// Every row keeps a label of its own, and the undergraduate layout reads as
/// before. The numbers are made up.
#[test]
fn reads_requirement_tables_by_measure_row_and_by_category_row() {
    let tables = requirement_tables(
        r#"<h2>修了要件 ＜電気電子コース＞</h2>
        <table class="tblYouken"><tbody>
        <tr><td colspan="2"></td><th>修了要件</th><th>修得済単位</th><th>修得済単位のうち文系教養科目対応科目単位数（400番台）</th></tr>
        <tr><th rowspan="5">教養科目群</th><th>文系教養科目（400番台）</th><td>2</td><td>1</td><td></td></tr>
        <tr><th>文系教養科目（500番台）</th><td>1</td><td>0</td><td></td></tr>
        <tr><th>アントレプレナーシップ科目（GA付）</th><td>2</td><td>2</td><td></td></tr>
        <tr><th>アントレプレナーシップ科目（GAなし） (注)</th><td></td><td>0</td><td></td></tr>
        <tr><th>教養科目群（小計）＜A＞</th><td>5</td><td>3</td><td>0</td></tr>
        <tr><th rowspan="4">専門科目群</th><th>講究科目</th><td>8</td><td>4</td><td></td></tr>
        <tr><th>コース標準学修課程 専門科目（L）</th><td>13</td><td>6</td><td></td></tr>
        <tr><th>コース標準学修課程の専門科目群 （小計）＜B＞</th><td></td><td>10</td><td>0</td></tr>
        <tr><th>コース標準学修課程外の専門科目＜C＞</th><td></td><td>0</td><td>0</td></tr>
        <tr><th colspan="2">合計＜A+B+C＞</th><td>30</td><td>13</td><td>0</td></tr>
        </tbody></table>
        <h2>修得済GA</h2>
        <table class="tblYouken"><tbody><tr><td>GA0M</td></tr></tbody></table>
        <h2>修了要件に含まれない科目</h2>
        <table class="tblYouken"><tbody>
        <tr><td></td><th>修得済単位</th></tr>
        <tr><th>特定教育課程専用教養科目</th><td>0</td></tr>
        <tr><th>100～300番台</th><td>0</td></tr>
        <tr><th>600番台</th><td>0</td></tr>
        </tbody></table>
        <h2>卒業要件</h2>
        <table class="tblYouken"><tbody>
        <tr><th rowspan="2">区分</th><th colspan="2">教養</th></tr>
        <tr><th>文系</th><th>英語</th></tr>
        <tr><th>卒業要件</th><td>2</td><td>4</td></tr>
        <tr><th>修得済単位</th><td>2</td><td>2</td></tr>
        </tbody></table>"#,
    );
    assert_eq!(tables.len(), 4);

    let course = &tables[0];
    let headers = course["headers"].as_array().unwrap();
    assert_eq!(headers.len(), 4, "the corner, then one header per value");
    assert_eq!(
        headers[..3],
        [json!(""), json!("修了要件"), json!("修得済単位")]
    );
    let course_labels = labels(course);
    assert_eq!(course_labels.len(), 10);
    assert!(
        course_labels[0].starts_with("教養科目群 / 文系教養科目"),
        "{course_labels:?}"
    );
    assert!(
        course_labels[4].starts_with("教養科目群 / 教養科目群"),
        "{course_labels:?}"
    );
    assert!(
        course_labels[5].starts_with("専門科目群 / 講究科目"),
        "{course_labels:?}"
    );
    assert!(course_labels[9].starts_with("合計"), "{course_labels:?}");
    let distinct: std::collections::HashSet<_> = course_labels.iter().collect();
    assert_eq!(
        distinct.len(),
        course_labels.len(),
        "labels name one row each"
    );
    assert_eq!(course["rows"][0]["values"], json!(["2", "1", ""]));
    // A measure the page leaves blank stays blank, not shifted.
    assert_eq!(course["rows"][3]["values"], json!(["", "0", ""]));
    assert_eq!(course["rows"][9]["values"], json!(["30", "13", "0"]));

    assert_eq!(tables[1]["headers"], json!(["", "修得済GA"]));
    assert_eq!(
        tables[1]["rows"],
        json!([{ "label": "修得済GA", "values": ["GA0M"] }])
    );

    let outside = &tables[2];
    assert_eq!(outside["headers"], json!(["", "修得済単位"]));
    assert_eq!(labels(outside).len(), 3);
    assert_eq!(outside["rows"][2]["values"], json!(["0"]));

    let undergraduate = &tables[3];
    assert_eq!(
        undergraduate["headers"],
        json!(["区分", "教養 / 文系", "教養 / 英語"])
    );
    assert_eq!(labels(undergraduate), ["卒業要件", "修得済単位"]);
    assert_eq!(undergraduate["rows"][0]["values"], json!(["2", "4"]));
}

/// A table whose rows and headers name no measure is still not read as
/// requirements: the page is some other table.
#[test]
fn a_requirement_table_naming_no_measure_is_refused() {
    let html = r#"<html><body><div class="tableSet02Tbl"><table class="tblYouken"><tbody>
        <tr><th></th><th>単位</th></tr><tr><th>文系教養科目</th><td>2</td></tr>
        </tbody></table></div></body></html>"#;
    let payload = extract_page(REQUIREMENTS, html);
    assert_eq!(payload.status, "error");
    assert_eq!(
        payload.message,
        "A requirement table has no recognized rows."
    );
}

/// A student menu as 教務Web renders it for one programme: the panel with
/// each link's own id and address, relative to the student top.
fn student_menu(links: &[(&str, &str)]) -> String {
    let items: String = links
        .iter()
        .map(|(id, href)| format!(r#"<li><a id="ctl00_sideMenu1_{id}" href="{href}">x</a></li>"#))
        .collect();
    format!(
        r#"<html><body><div id="ctl00_tdSide"><div id="ctl00_sideMenu1_pnlStudent"><ul>{items}</ul></div></div>
        <div id="ctl00_ContentPlaceHolder1"><a href="%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx">outside the menu</a></div></body></html>"#
    )
}

/// Issue #64: a 修士 account's 科目から and 成績一覧 are `PID1_2` and
/// `PID2_2`, and 教務Web sends it away from the 学士 copies. The menu's
/// addresses decide, never its ids, which differ between programmes
/// (成績一覧 is `HyperLink39` for 学士, `HyperLink8` for 修士).
#[test]
fn the_student_menu_picks_each_programmes_own_page() {
    const S: &str = "%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A";
    const G: &str = "%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7";
    let bachelor = student_menu(&[
        ("HyperLink37", &format!("{S}/PID1_0.aspx")),
        ("lnkKamokuKbn", &format!("{S}/PID1_1.aspx")),
        ("HyperLink39", &format!("{G}/PID2_0.aspx")),
        ("lnkGPA", &format!("{G}/PID2_7.aspx")),
        ("lnkYouken", &format!("{G}/PID2_6.aspx")),
    ]);
    let master = student_menu(&[
        ("HyperLink37", &format!("{S}/PID1_0.aspx")),
        ("lnkKamokuKbn", &format!("{S}/PID1_2.aspx")),
        // The page's own links use lower-case escapes and `../Student/`.
        (
            "HyperLink8",
            "../Student/%e6%88%90%e7%b8%be%e9%96%b2%e8%a6%a7/PID2_2.aspx",
        ),
        ("HyperLink160", &format!("{G}/PID2_7.aspx")),
        ("lnkYouken", &format!("{G}/PID2_6.aspx")),
    ]);
    let file = |menu: &str, kind: &str| {
        let url = Menu::read(STUDENT_TOP, menu).unwrap().url(kind).unwrap();
        assert_eq!(expected_kind(Url::parse(&url).unwrap().path()), Some(kind));
        url.rsplit('/').next().unwrap().to_string()
    };
    for (kind, bachelors, masters) in [
        ("registrations", "PID1_0.aspx", "PID1_0.aspx"),
        ("declared", "PID1_1.aspx", "PID1_2.aspx"),
        ("grades", "PID2_0.aspx", "PID2_2.aspx"),
        ("gpa", "PID2_7.aspx", "PID2_7.aspx"),
        ("requirements", "PID2_6.aspx", "PID2_6.aspx"),
        // Not in the menu: one page for every programme.
        ("profile", "PID4_2.aspx", "PID4_2.aspx"),
    ] {
        assert_eq!(file(&bachelor, kind), bachelors, "{kind}");
        assert_eq!(file(&master, kind), masters, "{kind}");
    }

    // A programme with an address of its own is not guessed: the error
    // names the menu's unknown link in that directory.
    let doctor = student_menu(&[("HyperLink9", &format!("{G}/PID2_5.aspx"))]);
    let error = Menu::read(STUDENT_TOP, &doctor)
        .unwrap()
        .url("grades")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("grades") && error.contains("PID2_5.aspx"),
        "{error}"
    );
    // A page without the student menu (a sign-in form, changed markup).
    assert!(Menu::read(STUDENT_TOP, "<html><body><form></form></body></html>").is_err());
    // A 修士 grades payload from its own page passes the trust boundary.
    let master_grades = GRADES.replace("PID2_0", "PID2_2");
    let (stage, _) = parse(json!({
        "status":"ok","sourceUrl":master_grades,"kind":"grades",
        "data":{"complete":true,"shape":"undergraduate-grouped","empty":true,"groups":[]}
    }))
    .unwrap();
    assert_eq!(stage, KyomuStage::Ready);
}
