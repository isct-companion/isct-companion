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
