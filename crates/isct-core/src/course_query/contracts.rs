//! Shell contracts for finder projections; catalog rows keep their stored shape.
use super::{Condition, Value};
use crate::contracts::{ClassFormat, CreditSplit, Localized, Quarter};
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct TimeValue {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub day: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub from: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub to: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderCourse {
    pub id: String,
    pub code: String,
    pub year: i32,
    pub quarters: Vec<Quarter>,
    pub title: Localized,
    pub instructors: Vec<Localized>,
    pub credits: CreditSplit,
    pub instruction_language: String,
    pub format: ClassFormat,
    pub schedule: Vec<FinderSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub unit_code: Option<String>,
    pub syllabus_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub detailed: Option<bool>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct FinderSchedule {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub day: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub periods: Option<(u8, u8)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub room: Option<Localized>,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct FinderRow {
    #[cfg_attr(feature = "ts", ts(type = "FinderCourse"))]
    pub course: Value,
    pub matched: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Suggestion {
    Category {
        key: String,
        label: String,
    },
    Condition {
        key: String,
        condition: Condition,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        count: i64,
    },
    Course {
        #[cfg_attr(feature = "ts", ts(type = "FinderCourse"))]
        course: Value,
    },
    Text {
        key: String,
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        count: i64,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(deny_unknown_fields)]
pub struct OfficialSelection {
    pub id: String,
    pub value: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RemoteQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub controls: Option<Vec<OfficialSelection>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub recommended: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub quarters: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub days: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub period_starts: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub requirement_groups: Option<Vec<String>>,
    pub q: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub teacher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub groups: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(untagged)]
pub enum RemotePlan {
    Queries {
        queries: Vec<RemoteQuery>,
    },
    Unsupported {
        #[cfg_attr(feature = "ts", ts(type = "\"empty\" | \"too-many\""))]
        unsupported: String,
    },
}
