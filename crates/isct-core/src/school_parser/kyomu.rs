use super::SchoolParseError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 5_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KyomuPayload {
    pub status: String,
    pub source_url: String,
    pub kind: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KyomuStage {
    Ready,
    Unavailable,
    ReconnectRequired,
}

fn expected_kind(path: &str) -> Option<&'static str> {
    match path {
        "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx" => Some("registrations"),
        "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx" => Some("declared"),
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx" => Some("grades"),
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx" => Some("requirements"),
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx" => Some("gpa"),
        _ => None,
    }
}

/// Validates the bounded DTO produced by the WebView capture adapter.
///
/// The adapter only reads the live DOM. Trust starts here: source identity,
/// completeness, and collection limits are checked before persistence sees it.
pub fn parse_payload(raw: &str) -> Result<(KyomuStage, Option<KyomuPayload>), SchoolParseError> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("kyomu payload exceeds the size limit".into());
    }
    let payload: KyomuPayload =
        serde_json::from_str(raw).map_err(|_| "kyomu payload is not valid JSON")?;
    let url = Url::parse(&payload.source_url).map_err(|_| "kyomu source URL is invalid")?;
    if payload.status == "login" {
        let known_login_host = matches!(
            url.host_str(),
            Some("kyomu0.gakumu.titech.ac.jp" | "portal.titech.ac.jp" | "rp.nap.gsic.titech.ac.jp")
        );
        if url.scheme() == "https" && known_login_host {
            return Ok((KyomuStage::ReconnectRequired, None));
        }
    }
    if url.scheme() != "https"
        || url.host_str() != Some("kyomu0.gakumu.titech.ac.jp")
        || expected_kind(url.path()) != Some(payload.kind.as_str())
    {
        return Err("kyomu source page is not approved".into());
    }
    if payload.status == "unavailable" {
        if payload.message.trim().is_empty() {
            return Err("kyomu unavailable result has no reason".into());
        }
        return Ok((KyomuStage::Unavailable, Some(payload)));
    }
    if payload.status != "ok"
        || !payload.data.is_object()
        || payload.data.get("complete").and_then(Value::as_bool) != Some(true)
    {
        return Err("kyomu extraction was incomplete".into());
    }
    let shape = payload
        .data
        .get("shape")
        .and_then(Value::as_str)
        .ok_or_else(|| SchoolParseError::from("kyomu payload shape is missing"))?;
    let empty = payload
        .data
        .get("empty")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let array = |key: &str| {
        payload
            .data
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| SchoolParseError::from("kyomu payload collection is missing"))
    };
    let count = match payload.kind.as_str() {
        "registrations" if shape == "registration-page" => {
            if !payload.data.get("state").is_some_and(Value::is_object) {
                return Err("kyomu registration state is missing".into());
            }
            array("courses")?.len()
        }
        "declared" if shape == "declared-course-tables" => array("courses")?.len(),
        "grades" if matches!(shape, "undergraduate-grouped" | "graduate-flat") => {
            let groups = array("groups")?;
            if shape == "graduate-flat" && !groups.is_empty() {
                let sections = groups
                    .iter()
                    .filter_map(|group| group.get("section").and_then(Value::as_str))
                    .collect::<std::collections::HashSet<_>>();
                if groups.len() != 2
                    || sections.len() != 2
                    || !sections.contains("graduate")
                    || !sections.contains("undergraduate")
                {
                    return Err("kyomu graduate grade sections are incomplete".into());
                }
            }
            let mut count = 0;
            for group in groups {
                if group
                    .get("category")
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.is_empty())
                {
                    return Err("kyomu grade group has no category".into());
                }
                let rows = group
                    .get("rows")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SchoolParseError::from("kyomu grade rows are missing"))?;
                if rows.iter().any(|row| {
                    row.get("title")
                        .and_then(Value::as_str)
                        .is_none_or(|value| value.is_empty())
                }) {
                    return Err("kyomu grade row has no course title".into());
                }
                count += rows.len();
            }
            count
        }
        "requirements" if shape == "requirements-tables" => {
            let tables = array("tables")?;
            let mut count = 0;
            for table in tables {
                let headers = table
                    .get("headers")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        SchoolParseError::from("kyomu requirement headers are missing")
                    })?;
                let rows = table
                    .get("rows")
                    .and_then(Value::as_array)
                    .ok_or_else(|| SchoolParseError::from("kyomu requirement rows are missing"))?;
                if headers.len() < 2
                    || rows.iter().any(|row| {
                        row.get("label")
                            .and_then(Value::as_str)
                            .is_none_or(|value| value.is_empty())
                            || row.get("values").and_then(Value::as_array).is_none()
                    })
                {
                    return Err("kyomu requirement table is incomplete".into());
                }
                count += rows.len();
            }
            count
        }
        "gpa" if shape == "gpa-summary" => {
            let overall = array("overall")?;
            let terms = array("terms")?;
            let categories = array("categories")?;
            if !empty && overall.is_empty() {
                return Err("kyomu GPA summary is missing".into());
            }
            overall.len() + terms.len() + categories.len()
        }
        "registrations" | "declared" | "grades" | "requirements" | "gpa" => {
            return Err("kyomu payload shape is not supported".into())
        }
        _ => return Err("kyomu payload kind is invalid".into()),
    };
    if count == 0 && !empty {
        return Err("kyomu empty result is not explicit".into());
    }
    if count > MAX_RECORDS {
        return Err("kyomu payload exceeds the record limit".into());
    }
    Ok((KyomuStage::Ready, Some(payload)))
}

#[cfg(test)]
mod tests {
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
}
