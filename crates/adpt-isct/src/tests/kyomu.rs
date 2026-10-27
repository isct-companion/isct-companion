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
