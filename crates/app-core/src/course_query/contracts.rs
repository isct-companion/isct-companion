//! Shell contracts for finder projections; catalog rows keep their stored shape.
use super::{Condition, Recommendation, Value};
use crate::contracts::{ClassFormat, CreditSplit, Localized, Quarter};
use crate::study_profile::CourseEligibility;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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
/// One search result with what its row shows: whether it is on the
/// timetable or clashes with it, what it would replace, its credits, its
/// eligibility when that is not plainly allowed, and its 教務 recommendation.
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderRowView {
    // Shared with the cached catalog; serializes as the row itself.
    #[cfg_attr(feature = "ts", ts(type = "FinderCourse"))]
    pub course: Arc<Value>,
    /// The labels of the either-or conditions this row met.
    pub matched: Vec<String>,
    /// Its code is on the term's timetable.
    pub placed: bool,
    /// One of its weekly slots overlaps the timetable.
    pub clash: bool,
    /// The placed plan courses adding it would replace.
    pub replaces: Vec<String>,
    pub credits: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub eligibility: Option<CourseEligibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub recommended: Option<Recommendation>,
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
        row: FinderRowView,
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
