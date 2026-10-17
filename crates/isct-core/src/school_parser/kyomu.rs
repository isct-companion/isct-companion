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
    Maintenance,
    Unavailable,
    ReconnectRequired,
}

pub(crate) const PAGES: [(&str, &str); 6] = [
    (
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx",
        "gpa",
    ),
    (
        "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx",
        "registrations",
    ),
    (
        "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx",
        "declared",
    ),
    (
        "/Titech/Student/%E5%AD%A6%E7%94%9F%E6%83%85%E5%A0%B1/PID4_2.aspx",
        "profile",
    ),
    (
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx",
        "grades",
    ),
    (
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx",
        "requirements",
    ),
];

pub(crate) fn expected_kind(path: &str) -> Option<&'static str> {
    if path.eq_ignore_ascii_case(crate::catalog::KYOMU_SEARCH_PATH) {
        return Some("catalog");
    }
    PAGES
        .iter()
        .find(|(approved, _)| path.eq_ignore_ascii_case(approved))
        .map(|(_, kind)| *kind)
}

/// The page kind owned by an approved 教務 source URL, read from the URL alone
/// so a rejected payload is still reported against the page it came from.
pub(crate) fn source_kind(raw: &str) -> Option<&'static str> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Source {
        source_url: String,
    }
    if raw.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    let source: Source = serde_json::from_str(raw).ok()?;
    let url = Url::parse(&source.source_url).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("kyomu0.gakumu.titech.ac.jp") {
        return None;
    }
    expected_kind(url.path())
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
    if payload.status == "maintenance" {
        if url.scheme() != "https"
            || url.host_str() != Some("kyomu0.gakumu.titech.ac.jp")
            || payload.message.trim().is_empty()
            || payload.message.len() > 8192
        {
            return Err("kyomu maintenance notice is invalid".into());
        }
        return Ok((KyomuStage::Maintenance, Some(payload)));
    }
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
    // The extractor names the selector that rejected the page. Keep it: the
    // generic text alone cannot tell a changed layout from a closed window.
    if payload.status == "error" && !payload.message.trim().is_empty() {
        let reason: String = payload.message.trim().chars().take(256).collect();
        return Err(format!("kyomu extraction was incomplete: {reason}").into());
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
        "catalog" if shape == "course-search" => {
            let courses = array("courses")?;
            if courses.len() > 100
                || payload
                    .data
                    .get("nextCursor")
                    .and_then(Value::as_str)
                    .is_some_and(|cursor| uuid::Uuid::parse_str(cursor).is_err())
            {
                return Err("kyomu course page exceeds its limit or has an invalid cursor".into());
            }
            for course in courses {
                let id = course["id"].as_str().unwrap_or("");
                if id.len() != 9
                    || !id.bytes().all(|b| b.is_ascii_digit())
                    || course["year"].as_i64() != id[..4].parse::<i64>().ok()
                {
                    return Err("kyomu course identity is invalid".into());
                }
                serde_json::from_value::<crate::syllabus::model::Offering>(course.clone())
                    .map_err(|_| SchoolParseError::from("kyomu course result is invalid"))?;
            }
            courses.len()
        }
        "profile" if shape == "academic-profile" => {
            for field in ["department", "programme", "admittedOn"] {
                if payload
                    .data
                    .get(field)
                    .and_then(Value::as_str)
                    .is_none_or(|v| v.is_empty() || v.len() > 200)
                {
                    return Err("kyomu academic profile is incomplete".into());
                }
            }
            chrono::NaiveDate::parse_from_str(
                payload.data["admittedOn"].as_str().unwrap_or(""),
                "%Y-%m-%d",
            )
            .map_err(|_| SchoolParseError::from("kyomu admission date is invalid"))?;
            1
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
#[path = "../tests/school_parser__kyomu.rs"]
mod tests;
