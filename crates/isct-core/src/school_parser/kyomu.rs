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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KyomuStage {
    Ready,
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
    if payload.status != "ok" || !payload.data.is_object() {
        return Err("kyomu extraction was incomplete".into());
    }
    let count = match payload.kind.as_str() {
        "registrations" | "declared" => payload
            .data
            .pointer("/courses")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        "grades" => payload
            .data
            .pointer("/groups")
            .and_then(Value::as_array)
            .map_or(0, |groups| {
                groups
                    .iter()
                    .filter_map(|group| group.get("rows").and_then(Value::as_array))
                    .map(Vec::len)
                    .sum()
            }),
        "requirements" => payload
            .data
            .pointer("/tables")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        "gpa" => payload
            .data
            .pointer("/terms")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        _ => return Err("kyomu payload kind is invalid".into()),
    };
    if count > MAX_RECORDS {
        return Err("kyomu payload exceeds the record limit".into());
    }
    Ok((KyomuStage::Ready, Some(payload)))
}
