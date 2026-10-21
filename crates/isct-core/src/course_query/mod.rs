//! The course finder's product grammar, shared by all shells.
//! Its pills and syllabus fan-out rules are application-specific; regex and
//! Unicode normalization handle the parsing primitives.
mod contracts;
pub use contracts::*;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::LazyLock,
};
#[cfg(feature = "ts")]
use ts_rs::TS;
use unicode_normalization::UnicodeNormalization;

pub use crate::study_profile::catalog_course_eligibility;
use crate::study_profile::{CourseEligibility, CourseEligibilityStatus};

const DAYS: [&str; 6] = ["月", "火", "水", "木", "金", "土"];
const SHORT_DAYS: [&str; 6] = ["mon", "tue", "wed", "thu", "fri", "sat"];
const LONG_DAYS: [&str; 6] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
const SLOTS: [(u8, u8); 5] = [(1, 2), (3, 4), (5, 6), (7, 8), (9, 10)];
const ORDER: [&str; 15] = [
    "quarter", "dept", "level", "code", "time", "format", "lang", "kind", "credits", "mark",
    "group", "free", "text", "exclude", "",
];

macro_rules! captures {
    ($pattern:literal, $text:expr) => {{
        static RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new($pattern).ok());
        RE.as_ref().and_then(|regex| regex.captures($text))
    }};
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn normalized(value: &str) -> String {
    value.nfkc().collect()
}
fn lower(value: &str) -> String {
    normalized(value).to_lowercase()
}
fn false_value(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderContext {
    pub current_quarter: u8,
    #[serde(default)]
    pub recommended: BTreeMap<String, Recommendation>,
    #[serde(default)]
    pub departments: Vec<FinderDepartment>,
    #[serde(default)]
    pub busy: Vec<BusySlot>,
    #[serde(default)]
    pub official_options: Vec<OfficialFilterOption>,
}
/// Public search controls captured from the authenticated official form.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OfficialFilterOption {
    pub id: String,
    pub value: String,
    pub label: String,
    pub family: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct Recommendation {
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderDepartment {
    pub id: String,
    pub name: BTreeMap<String, String>,
    pub unit_code: String,
    #[serde(default)]
    pub code_prefix: String,
    #[serde(default)]
    pub scope: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct BusySlot {
    pub day: u8,
    pub from: u8,
    pub to: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct Condition {
    pub r#type: String,
    pub key: String,
    pub label: String,
    #[cfg_attr(
        feature = "ts",
        ts(type = "(string | number | TimeValue | { id: string; value: string })[]")
    )]
    pub values: Vec<Value>,
    #[serde(default, skip_serializing_if = "false_value")]
    #[cfg_attr(feature = "ts", ts(optional, as = "Option<bool>"))]
    pub prefix: bool,
    #[serde(default, skip_serializing_if = "false_value")]
    #[cfg_attr(feature = "ts", ts(optional, as = "Option<bool>"))]
    pub exclude: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct Factor {
    pub r#type: String,
    pub alts: Vec<Condition>,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
pub struct Factors {
    pub factors: Vec<Factor>,
    pub duplicates: usize,
}

fn make(kind: &str, key: impl Into<String>, label: impl Into<String>, value: Value) -> Condition {
    Condition {
        r#type: kind.into(),
        key: key.into(),
        label: label.into(),
        values: vec![value],
        prefix: false,
        exclude: false,
    }
}
fn day_number(value: &str) -> Option<u8> {
    SHORT_DAYS
        .iter()
        .position(|day| *day == value)
        .or_else(|| LONG_DAYS.iter().position(|day| *day == value))
        .or_else(|| DAYS.iter().position(|day| *day == value))
        .map(|day| day as u8 + 1)
}
fn time_condition(day: Option<u8>, from: Option<u8>, to: Option<u8>) -> Condition {
    let (from, to) = match (from, to) {
        (Some(a), Some(b)) if b < a => (Some(b), Some(a)),
        (a, b) => (a, b.or(a)),
    };
    let d = day
        .and_then(|day| DAYS.get(usize::from(day.saturating_sub(1))))
        .copied()
        .unwrap_or("");
    let label = match from {
        None => format!("{d}曜"),
        Some(a) => {
            let periods = match to {
                Some(b) if b != a => format!("{a}-{b}"),
                _ => a.to_string(),
            };
            if d.is_empty() {
                format!("{periods}限")
            } else {
                format!("{d}{periods}")
            }
        }
    };
    let mut value = serde_json::Map::new();
    if let Some(day) = day {
        value.insert("day".into(), json!(day));
    }
    if let Some(from) = from {
        value.insert("from".into(), json!(from));
    }
    if let Some(to) = to {
        value.insert("to".into(), json!(to));
    }
    make("time", &label, &label, Value::Object(value))
}
fn recognize(word: &str, ctx: &FinderContext, next: Option<&str>) -> Condition {
    let normalized = normalized(word);
    let w = normalized.trim();
    let lw = w.to_lowercase();
    if let Some(option) = ctx.official_options.iter().find(|option| {
        w == format!("kyomu:{}:{}", option.id, option.value)
            || (option.family != "quarter" && w == option.label)
            || (option.family == "level"
                && [
                    format!("{}番台", option.label),
                    format!("{}-level", option.label),
                    format!("{}xx", option.label.trim_end_matches('0')),
                ]
                .iter()
                .any(|alias| lw == alias.to_lowercase()))
    }) {
        return make(
            &format!("official-{}", option.family),
            format!("kyomu:{}:{}", option.id, option.value),
            &option.label,
            json!({"id":option.id,"value":option.value}),
        );
    }
    if let Some((prefix, value)) = w.split_once(':').filter(|(_, v)| !v.trim().is_empty()) {
        let kind = match prefix.to_ascii_lowercase().as_str() {
            "teacher" | "instructor" | "教員" | "教师" => Some("teacher"),
            "room" | "教室" => Some("room"),
            _ => None,
        };
        if let Some(kind) = kind {
            return make(
                kind,
                format!("{kind}:{}", value.trim()),
                format!("{prefix}: {}", value.trim()),
                json!(value.trim().to_lowercase()),
            );
        }
    }
    if ["curr", "current", "当前", "今期"].contains(&lw.as_str()) {
        return make(
            "quarter",
            "curr",
            format!("{}Q", ctx.current_quarter),
            json!(ctx.current_quarter),
        );
    }
    if let Some(m) = captures!(r"^q?([1-4])q?$", &lw).filter(|_| lw.contains('q')) {
        let q = m.get(1).map(|x| x.as_str()).unwrap_or("1");
        return make(
            "quarter",
            format!("{q}Q"),
            format!("{q}Q"),
            json!(q.parse::<u8>().unwrap_or(1)),
        );
    }
    if let Some(m) = captures!(r"^([A-Za-z]{3})\.([A-Za-z])([0-9]{3})(?:\.[A-Za-z])?$", w) {
        let code = format!(
            "{}.{}{}",
            m.get(1).map(|s| s.as_str()).unwrap_or(""),
            m.get(2).map(|s| s.as_str()).unwrap_or(""),
            m.get(3).map(|s| s.as_str()).unwrap_or("")
        )
        .to_uppercase();
        return make("code", &code, &code, json!(code));
    }
    if captures!(r"^([A-Za-z]{3})(\.([A-Za-z][0-9]{0,2})?)?$", w).is_some()
        && (w.contains('.') || captures!(r"^[A-Z]{3}$", w).is_some())
    {
        let prefix = w.to_uppercase().trim_end_matches('.').to_string();
        let mut c = make(
            "code",
            &prefix,
            format!("{prefix}…"),
            json!(if prefix.contains('.') {
                prefix.clone()
            } else {
                format!("{prefix}.")
            }),
        );
        c.prefix = true;
        return c;
    }
    let from_match = |m: regex::Captures<'_>, day: Option<u8>, a: usize, b: usize| {
        time_condition(
            day,
            m.get(a).and_then(|v| v.as_str().parse().ok()),
            m.get(b).and_then(|v| v.as_str().parse().ok()),
        )
    };
    if let Some(m) = captures!(
        r"^([月火水木金土])(?:曜日?)?([0-9]{1,2})(?:-([0-9]{1,2}))?限?$",
        w
    ) {
        let day = m.get(1).and_then(|v| day_number(v.as_str()));
        return from_match(m, day, 2, 3);
    }
    if let Some(m) = captures!(
        r"^(mon|tue|wed|thu|fri|sat)([0-9]{1,2})(?:-([0-9]{1,2}))?$",
        &lw
    ) {
        let day = m.get(1).and_then(|v| day_number(v.as_str()));
        return from_match(m, day, 2, 3);
    }
    if let Some(day) = day_number(&lw) {
        if let Some(m) = next.and_then(|next| captures!(r"^([0-9]{1,2})(?:-([0-9]{1,2}))?$", next))
        {
            return from_match(m, Some(day), 1, 2);
        }
    }
    if let Some(m) = captures!(r"^([月火水木金土])曜日?$", w) {
        return time_condition(m.get(1).and_then(|v| day_number(v.as_str())), None, None);
    }
    if let Some(day) = day_number(&lw).filter(|_| !DAYS.contains(&w)) {
        return time_condition(Some(day), None, None);
    }
    if let Some(m) = captures!(r"^([0-9]{1,2})(?:-([0-9]{1,2}))?限$", w) {
        return from_match(m, None, 1, 2);
    }
    if let Some(m) =
        captures!(r"^([1-9])00(?:番台|-level)$", w).or_else(|| captures!(r"^([1-9])xx$", &lw))
    {
        let level = m
            .get(1)
            .and_then(|v| v.as_str().parse::<u16>().ok())
            .unwrap_or(1)
            * 100;
        return make(
            "level",
            format!("{level}番台"),
            format!("{level}番台"),
            json!(level),
        );
    }
    for (aliases, kind, key, label, value) in [
        (
            &["推荐", "推薦", "おすすめ", "rec", "recommended", "○"][..],
            "mark",
            "推荐",
            "Recommended",
            json!("any"),
        ),
        (
            &["必修", "required", "◎"][..],
            "mark",
            "必修",
            "Required",
            json!("required"),
        ),
        (
            &["空き", "free", "不冲突", "no-conflict"][..],
            "free",
            "空き",
            "No conflict",
            json!(true),
        ),
        (
            &["講義", "lecture"][..],
            "format",
            "講義",
            "講義",
            json!("lecture"),
        ),
        (
            &["演習", "exercise"][..],
            "format",
            "演習",
            "演習",
            json!("exercise"),
        ),
        (
            &["実験", "実験等", "experiment"][..],
            "format",
            "実験",
            "実験",
            json!("experiment"),
        ),
        (
            &["英語", "english"][..],
            "lang",
            "英語",
            "English-taught",
            json!("en"),
        ),
        (
            &["日本語", "japanese"][..],
            "lang",
            "日本語",
            "Japanese-taught",
            json!("ja"),
        ),
        (
            &["集中", "集中講義", "intensive"][..],
            "kind",
            "集中",
            "Intensive",
            json!("intensive"),
        ),
        (
            &["特課研", "特課pj", "seminar"][..],
            "kind",
            "特課研",
            "Seminar",
            json!("seminar"),
        ),
    ] {
        if aliases.contains(&lw.as_str()) {
            return make(kind, key, label, value);
        }
    }
    if let Some(group) = captures!(r"^([A-Z])群$", w)
        .and_then(|m| m.get(1).map(|v| v.as_str().to_string()))
        .or_else(|| {
            if lw != "group" {
                return None;
            }
            next.and_then(|v| captures!(r"^([A-Za-z])$", v))
                .and_then(|m| m.get(1).map(|v| v.as_str().to_uppercase()))
        })
    {
        return make(
            "group",
            format!("{group}群"),
            format!("{group}群"),
            json!(group),
        );
    }
    if let Some(m) = captures!(r"^([0-9](?:\.[0-9])?)(単位|cr|credits?)$", &lw) {
        let raw = m.get(1).map(|s| s.as_str()).unwrap_or("0");
        return make(
            "credits",
            format!("{raw}単位"),
            format!("{raw} cr"),
            json!(raw.parse::<f64>().unwrap_or(0.)),
        );
    }
    if let Some(d) = ctx.departments.iter().find(|d| {
        std::iter::once(&d.id)
            .chain(d.name.values())
            .any(|v| lower(v) == lw)
    }) {
        let key = d.name.get("ja").unwrap_or(&d.id);
        let label = d
            .name
            .get("ja")
            .or_else(|| d.name.get("en"))
            .unwrap_or(&d.id);
        return make("dept", key, label, json!(d.unit_code));
    }
    make("text", w, w, json!(lw))
}

pub fn condition(key: &str, ctx: &FinderContext) -> Condition {
    let excluded = key.starts_with('!') && key.len() > 1;
    let raw = if excluded {
        key.strip_prefix('!').unwrap_or(key)
    } else {
        key
    };
    let mut words = raw.split_whitespace();
    let first = words.next().unwrap_or("");
    let second = words.next();
    let mut c =
        if second.is_some() && (day_number(&lower(first)).is_some() || lower(first) == "group") {
            recognize(first, ctx, second)
        } else {
            recognize(raw, ctx, None)
        };
    if excluded {
        c.key = format!("!{}", c.key);
        c.exclude = true;
    }
    c
}
pub fn factors_of(keys: &[String], ctx: &FinderContext) -> Factors {
    let mut result = Factors {
        factors: Vec::new(),
        duplicates: 0,
    };
    for key in keys {
        let c = condition(key, ctx);
        if result
            .factors
            .iter()
            .any(|f| f.alts.iter().any(|a| a.key == c.key))
        {
            result.duplicates += 1;
            continue;
        }
        let kind = if c.exclude { "exclude" } else { &c.r#type };
        let existing = if ["text", "exclude"].contains(&kind) {
            None
        } else {
            result.factors.iter_mut().find(|f| f.r#type == kind)
        };
        if let Some(f) = existing {
            f.alts.push(c);
        } else {
            result.factors.push(Factor {
                r#type: kind.into(),
                alts: vec![c],
            });
        }
    }
    result.factors.sort_by_key(|f| {
        ORDER
            .iter()
            .position(|kind| *kind == f.r#type)
            .unwrap_or(ORDER.len())
    });
    result
}
fn weekly(course: &Value) -> impl Iterator<Item = &Value> {
    array(course, "schedule").iter().filter(|s| {
        text(s, "kind") == "weekly"
            && s.get("day").and_then(Value::as_u64).is_some_and(|v| v > 0)
            && array(s, "periods").len() == 2
    })
}
fn overlap(slot: &Value, from: u64, to: u64) -> bool {
    let p = array(slot, "periods");
    matches!((p.first().and_then(Value::as_u64),p.get(1).and_then(Value::as_u64)),(Some(a),Some(b)) if a<=to && from<=b)
}
pub fn total_credits(course: &Value) -> f64 {
    ["lecture", "exercise", "lab"]
        .iter()
        .map(|key| {
            course
                .pointer(&format!("/credits/{key}"))
                .and_then(Value::as_f64)
                .unwrap_or(0.)
        })
        .sum()
}
pub fn clashes(course: &Value, busy: &[BusySlot]) -> bool {
    weekly(course).any(|s| {
        busy.iter().any(|b| {
            s.get("day").and_then(Value::as_u64) == Some(u64::from(b.day))
                && overlap(s, u64::from(b.from), u64::from(b.to))
        })
    })
}
pub fn clashing_courses(course: &Value, placed: &[Value]) -> Vec<Value> {
    placed
        .iter()
        .filter(|p| {
            text(p, "code") != text(course, "code")
                && array(p, "meetings").iter().any(|m| {
                    let periods: Vec<_> = array(m, "periods")
                        .iter()
                        .filter_map(Value::as_u64)
                        .collect();
                    match (periods.iter().min(), periods.iter().max()) {
                        (Some(from), Some(to)) => weekly(course).any(|s| {
                            array(m, "days").iter().any(|d| Some(d) == s.get("day"))
                                && overlap(s, *from, *to)
                        }),
                        _ => false,
                    }
                })
        })
        .cloned()
        .collect()
}
fn searchable(course: &Value) -> String {
    let mut terms = vec![
        text(course, "code"),
        course
            .pointer("/title/ja")
            .and_then(Value::as_str)
            .unwrap_or(""),
        course
            .pointer("/title/en")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ];
    for i in array(course, "instructors") {
        for lang in ["ja", "en"] {
            if let Some(v) = i.get(lang).and_then(Value::as_str) {
                terms.push(v);
            }
        }
    }
    lower(
        &terms
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}
fn hit(course: &Value, c: &Condition, ctx: &FinderContext) -> bool {
    // Official form choices are read from each stored row the way the form
    // means them. A choice the captured form no longer lists, or whose
    // meaning the row cannot show, keeps the row.
    if c.r#type.starts_with("official-") {
        return c.values.is_empty()
            || c.values.iter().any(|value| {
                ctx.official_options
                    .iter()
                    .find(|option| {
                        value["id"] == option.id.as_str() && value["value"] == option.value.as_str()
                    })
                    .is_none_or(|option| crate::catalog::official_control_match(course, option))
            });
    }
    c.values.iter().any(|v| match c.r#type.as_str() {
        "teacher" => v.as_str().is_some_and(|needle| {
            array(course, "instructors").iter().any(|name| {
                ["ja", "en"]
                    .iter()
                    .any(|lang| lower(text(name, lang)).contains(needle))
            })
        }),
        "room" => v.as_str().is_some_and(|needle| {
            array(course, "schedule").iter().any(|slot| {
                ["ja", "en"].iter().any(|lang| {
                    lower(
                        slot.get("room")
                            .and_then(|r| r.get(*lang))
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    )
                    .contains(needle)
                })
            })
        }),
        "quarter" => array(course, "quarters").contains(&json!(format!("{}Q", v))),
        "time" => weekly(course).any(|s| {
            (v.get("day").is_none() || v.get("day") == s.get("day"))
                && v.get("from").and_then(Value::as_u64).is_none_or(|from| {
                    overlap(s, from, v.get("to").and_then(Value::as_u64).unwrap_or(from))
                })
        }),
        "code" => v.as_str().is_some_and(|v| {
            if c.prefix {
                text(course, "code").starts_with(v)
            } else {
                text(course, "code") == v
            }
        }),
        "level" => {
            Some(
                u64::from(
                    text(course, "code")
                        .chars()
                        .nth(5)
                        .and_then(|c| c.to_digit(10))
                        .unwrap_or(0),
                ) * 100,
            ) == v.as_u64()
        }
        "mark" => ctx.recommended.get(text(course, "code")).is_some_and(|r| {
            if v == "required" {
                r.mark.as_deref() == Some("required")
            } else {
                r.mark.as_ref().is_some_and(|s| !s.is_empty())
            }
        }),
        "group" => ctx
            .recommended
            .get(text(course, "code"))
            .is_some_and(|r| Some(r.group.as_str()) == v.as_str()),
        "dept" => course.get("unitCode") == Some(v),
        "lang" => {
            course.get("instructionLanguage") == Some(v)
                || text(course, "instructionLanguage") == "ja-en"
        }
        "format" => course
            .get("format")
            .is_some_and(|format| array(format, "kinds").contains(v)),
        "kind" => array(course, "schedule")
            .iter()
            .any(|s| s.get("kind") == Some(v)),
        "credits" => v.as_f64() == Some(total_credits(course)),
        "free" => !clashes(course, &ctx.busy),
        "text" => v.as_str().is_some_and(|s| searchable(course).contains(s)),
        _ => false,
    })
}
pub fn run(
    courses: &[Value],
    factors: &[Factor],
    ctx: &FinderContext,
    live: &str,
) -> Vec<FinderRow> {
    let live = lower(live.trim());
    let mut seen = HashSet::new();
    let mut rows: Vec<FinderRow> = courses
        .iter()
        .filter(|c| {
            (live.is_empty() || searchable(c).contains(&live))
                && factors.iter().all(|f| {
                    if f.r#type == "exclude" {
                        f.alts.first().is_none_or(|a| !hit(c, a, ctx))
                    } else {
                        f.alts.iter().any(|a| hit(c, a, ctx))
                    }
                })
                && seen.insert(text(c, "id"))
        })
        .map(|c| FinderRow {
            course: c.clone(),
            matched: factors
                .iter()
                .filter(|f| f.alts.len() > 1 && f.r#type != "exclude")
                .flat_map(|f| {
                    f.alts
                        .iter()
                        .filter(|a| hit(c, a, ctx))
                        .map(|a| a.label.clone())
                })
                .collect::<Vec<_>>(),
        })
        .collect();
    let weight = |c: &Value| {
        ctx.recommended
            .get(text(c, "code"))
            .map(|r| match r.mark.as_deref() {
                Some("required") => 2,
                Some("") | None => 0,
                Some(_) => 1,
            })
            .unwrap_or(0)
    };
    rows.sort_by(|a, b| {
        weight(&b.course)
            .cmp(&weight(&a.course))
            .then_with(|| text(&a.course, "code").cmp(text(&b.course, "code")))
    });
    rows
}

// Stable keyboard categories; the authenticated form supplies their choices.
const OFFICIAL_GROUPS: [(&str, &str, &[&str]); 9] = [
    ("level", "Course level", &["level", "番台", "课程番台"]),
    ("quarter", "Quarter", &["quarter", "学季", "クォーター"]),
    (
        "delivery",
        "Day / delivery",
        &["day", "weekday", "星期", "授课形式", "曜日"],
    ),
    ("start", "Starting period", &["period", "时限", "開始時限"]),
    (
        "category",
        "Course category",
        &["category", "科目分类", "科目区分"],
    ),
    (
        "requirement",
        "Requirement category",
        &["requirement", "必修分类", "必修"],
    ),
    (
        "language",
        "Teaching language",
        &["language", "开讲语言", "開講言語"],
    ),
    (
        "recommended",
        "Official recommendations",
        &["recommended", "教务推荐", "推奨"],
    ),
    (
        "other",
        "Other official filters",
        &["other", "其他教务筛选", "その他"],
    ),
];
fn official_group(input: &str) -> Option<(&'static str, &str)> {
    let (prefix, value) = input.split_once(':')?;
    OFFICIAL_GROUPS
        .iter()
        .find(|(family, _, aliases)| {
            prefix.eq_ignore_ascii_case(family)
                || aliases
                    .iter()
                    .any(|alias| prefix.eq_ignore_ascii_case(alias))
        })
        .map(|(family, _, _)| (*family, value.trim()))
}
fn vocabulary(ctx: &FinderContext) -> Vec<(String, Vec<String>)> {
    let mut out = vec![(
        "curr".into(),
        vec![
            "curr".into(),
            "current".into(),
            "当前".into(),
            "今期".into(),
        ],
    )];
    for q in 1..=4 {
        out.push((format!("{q}Q"), vec![format!("{q}q"), format!("q{q}")]));
    }
    for ((day, short), long) in DAYS.iter().zip(SHORT_DAYS).zip(LONG_DAYS) {
        out.push((
            format!("{day}曜"),
            vec![
                day.to_string(),
                format!("{day}曜"),
                short.into(),
                long.into(),
            ],
        ));
        for (a, b) in SLOTS {
            out.push((
                format!("{day}{a}-{b}"),
                vec![
                    format!("{day}{a}-{b}"),
                    format!("{day}{a}"),
                    format!("{day}{b}"),
                    format!("{short}{a}"),
                    format!("{short} {a}"),
                    format!("{short}{a}-{b}"),
                ],
            ));
        }
    }
    {
        out.extend([
            (
                "推荐".into(),
                vec!["推荐", "推薦", "おすすめ", "rec", "recommended"]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            ),
            ("必修".into(), vec!["必修".into(), "required".into()]),
        ]);
    }
    let groups: std::collections::BTreeSet<_> = ctx
        .recommended
        .values()
        .map(|r| &r.group)
        .filter(|g| !g.is_empty())
        .collect();
    for g in groups {
        out.push((
            format!("{g}群"),
            vec![format!("{g}群"), format!("group {}", g.to_lowercase())],
        ));
    }
    out.push((
        "空き".into(),
        vec!["空き", "free", "no conflict", "不冲突"]
            .into_iter()
            .map(String::from)
            .collect(),
    ));
    for d in &ctx.departments {
        out.push((
            d.name.get("ja").unwrap_or(&d.id).clone(),
            std::iter::once(d.id.clone())
                .chain(
                    ["ja", "en"]
                        .iter()
                        .filter_map(|key| d.name.get(*key).cloned()),
                )
                .filter(|v| !v.is_empty())
                .collect(),
        ));
    }
    for option in &ctx.official_options {
        out.push((
            format!("kyomu:{}:{}", option.id, option.value),
            if option.family == "level" {
                vec![
                    option.label.clone(),
                    format!("{}番台", option.label),
                    format!("{}xx", option.label.chars().next().unwrap_or('0')),
                ]
            } else {
                vec![option.label.clone()]
            },
        ));
    }
    for (key, aliases) in [
        ("講義", vec!["講義", "lecture"]),
        ("演習", vec!["演習", "exercise"]),
        ("実験", vec!["実験", "experiment"]),
        ("英語", vec!["英語", "english"]),
        ("日本語", vec!["日本語", "japanese"]),
        ("集中", vec!["集中", "集中講義", "intensive"]),
        ("特課研", vec!["特課研", "seminar"]),
    ] {
        out.push((key.into(), aliases.into_iter().map(String::from).collect()));
    }
    out
}
pub fn suggest(
    input: &str,
    keys: &[String],
    courses: &[Value],
    ctx: &FinderContext,
) -> Vec<Suggestion> {
    let exclude = input.starts_with('!');
    let typed = normalized(if exclude {
        input.strip_prefix('!').unwrap_or(input)
    } else {
        input
    });
    let typed = typed.trim();
    let lt = typed.to_lowercase();
    let mut out = Vec::new();
    let count = |key: &str| {
        let mut keys = keys.to_vec();
        keys.push(key.into());
        run(courses, &factors_of(&keys, ctx).factors, ctx, "").len()
    };
    let add = |key: &str, out: &mut Vec<Suggestion>| {
        let full = if exclude {
            format!("!{key}")
        } else {
            key.into()
        };
        if keys.contains(&full)
            || out
                .iter()
                .any(|v| matches!(v, Suggestion::Condition { key, .. } if key == &full))
        {
            return;
        }
        let c = condition(&full, ctx);
        // The official form only supports positive filters. Do not advertise
        // an exclusion that the server cannot execute or the cache cannot prove.
        if c.exclude && c.r#type.starts_with("official-") {
            return;
        }
        let remote = !c.exclude
            && (c.r#type.starts_with("official-")
                || ["dept", "code", "level", "text", "mark", "group"].contains(&c.r#type.as_str()));
        out.push(Suggestion::Condition {
            count: if remote { -1 } else { count(&full) as i64 },
            key: full,
            condition: c,
        });
    };
    if let Some((family, value)) = official_group(typed) {
        if !exclude {
            for option in ctx.official_options.iter().filter(|option| {
                option.family == family && lower(&option.label).contains(&lower(value))
            }) {
                add(&format!("kyomu:{}:{}", option.id, option.value), &mut out);
            }
        }
        return out;
    }
    if !exclude {
        for (family, label, aliases) in OFFICIAL_GROUPS {
            if ctx
                .official_options
                .iter()
                .any(|option| option.family == family)
                && (typed.is_empty()
                    || aliases
                        .iter()
                        .chain(std::iter::once(&label))
                        .any(|alias| lower(alias).contains(&lt)))
            {
                out.push(Suggestion::Category {
                    key: format!("{family}:"),
                    label: label.into(),
                });
            }
        }
    }
    if typed.is_empty() {
        for key in if exclude {
            &["集中", "特課研", "英語", "演習"][..]
        } else {
            &["curr", "推荐", "空き", "講義", "演習", "英語"][..]
        } {
            add(key, &mut out);
        }
        return out;
    }
    let mut scored = Vec::new();
    for (key, aliases) in vocabulary(ctx) {
        let best = std::iter::once(&key)
            .chain(&aliases)
            .map(|v| {
                let la = lower(v);
                if la == lt {
                    3
                } else if la.starts_with(&lt) {
                    2
                } else if lt.chars().count() >= 2 && la.contains(&lt) {
                    1
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0);
        if best > 0 {
            scored.push((best, key));
        }
    }
    scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let literal = condition(typed, ctx);
    if literal.r#type != "text"
        && (literal.r#type != "level"
            || ctx
                .official_options
                .iter()
                .any(|o| o.family == "level" && o.label == literal.values[0]))
        && !scored
            .iter()
            .any(|(score, key)| *key == literal.key || *score == 3)
    {
        scored.insert(0, (4, literal.key));
    }
    for (_, key) in scored.iter().take(6) {
        add(key, &mut out);
    }
    if !exclude {
        let upper = typed.to_uppercase();
        let mut seen = HashSet::new();
        let mut matching: Vec<_> = courses
            .iter()
            .filter(|c| {
                (text(c, "code").starts_with(&upper) || searchable(c).contains(&lt))
                    && seen.insert(text(c, "code"))
            })
            .collect();
        matching.sort_by_key(|c| !text(c, "code").starts_with(&upper));
        for course in matching.into_iter().take(6) {
            out.push(Suggestion::Course {
                course: course.clone(),
            });
        }
    }
    let key = if exclude {
        format!("!{typed}")
    } else {
        typed.into()
    };
    if !keys.contains(&key) {
        out.push(Suggestion::Text {
            count: if key.starts_with('!') {
                count(&key) as i64
            } else {
                -1
            },
            key,
        });
    }
    out
}
pub fn live_text(input: &str, keys: &[String], courses: &[Value], ctx: &FinderContext) -> String {
    let value = input.trim();
    if value.is_empty()
        || official_group(value).is_some()
        || value.starts_with('!')
        || condition(value, ctx).r#type != "text"
        || suggest(value, keys, courses, ctx)
            .iter()
            .any(|s| matches!(s, Suggestion::Condition { .. }))
    {
        String::new()
    } else {
        value.into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderRequest {
    pub year: u16,
    pub term: String,
    /// Explicitly include courses that cannot be newly added to this term.
    #[serde(default)]
    pub include_ineligible: bool,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub action: Option<FinderAction>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FinderAction {
    Commit { key: String },
    ToggleTime { key: String, append: bool },
    Paste { text: String },
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FinderView {
    pub context: FinderContext,
    #[cfg_attr(feature = "ts", ts(type = "FinderCourse[]"))]
    pub courses: Vec<Value>,
    pub factors: Vec<Factor>,
    pub duplicates: usize,
    pub keys: Vec<String>,
    pub rows: Vec<FinderRow>,
    pub suggestions: Vec<Suggestion>,
    pub live: String,
    pub clashes: Vec<String>,
    pub ineligible_ids: Vec<String>,
    pub eligibility: BTreeMap<String, CourseEligibility>,
    pub replacements: BTreeMap<String, Vec<String>>,
    pub credits: BTreeMap<String, f64>,
    pub on_timetable: Vec<String>,
    pub active_slots: Vec<String>,
    pub commit_key: Option<String>,
    pub paste_handled: bool,
    pub pasted_ids: Vec<String>,
    pub missing_codes: Vec<String>,
    pub ineligible_codes: Vec<String>,
}
#[derive(Debug, thiserror::Error)]
pub enum FinderError {
    #[error("{0}")]
    InvalidInput(String),
    #[error(transparent)]
    Schedule(#[from] crate::schedule::ScheduleError),
    #[error(transparent)]
    Data(#[from] crate::data::DataError),
    #[error("Course finder data is invalid: {0}")]
    Compatibility(#[from] serde_json::Error),
}

pub fn department_name_ja(id: &str) -> Result<Option<String>, serde_json::Error> {
    let departments: Vec<FinderDepartment> =
        serde_json::from_str(include_str!("departments.json"))?;
    Ok(departments
        .into_iter()
        .find(|department| department.id == id || id.ends_with(&format!(":{}", department.id)))
        .and_then(|department| department.name.get("ja").cloned()))
}

pub fn finder_view(
    workspace: &Value,
    offerings: Vec<Value>,
    request: FinderRequest,
) -> Result<FinderView, FinderError> {
    crate::schedule::Term::parse(&request.term)?;
    if !(1000..=9999).contains(&request.year)
        || request.keys.len() > 200
        || request.keys.iter().any(|k| k.len() > 2048)
        || request.input.len() > 4096
    {
        return Err(FinderError::InvalidInput(
            "Course search exceeds its input limits.".into(),
        ));
    }
    if matches!(&request.action,Some(FinderAction::Commit{key}|FinderAction::ToggleTime{key,..}) if key.len()>2048)
    {
        return Err(FinderError::InvalidInput(
            "Course condition is too long.".into(),
        ));
    }
    let departments: Vec<FinderDepartment> =
        serde_json::from_str(include_str!("departments.json"))?;
    let selected_department = workspace
        .pointer("/settings/departmentId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let department_name = department_name_ja(selected_department)?;
    let recommended = offerings
        .iter()
        .filter(|r| {
            r["year"].as_u64() == Some(u64::from(request.year))
                && department_name.as_deref().is_some_and(|name| {
                    r.pointer("/kyomu/department").and_then(Value::as_str) == Some(name)
                })
        })
        .filter(|r| {
            catalog_course_eligibility(workspace, &request.term, r).status
                != CourseEligibilityStatus::Ineligible
        })
        .filter_map(|r| {
            let group = r.pointer("/kyomu/group")?.as_str()?;
            if group.is_empty() {
                return None;
            }
            Some((
                text(r, "code").to_string(),
                Recommendation {
                    group: group.into(),
                    mark: r
                        .pointer("/kyomu/mark")
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .map(str::to_string),
                },
            ))
        })
        .collect();
    let placed = crate::schedule::scheduled_courses(workspace, &request.term);
    let busy = placed
        .iter()
        .flat_map(|c| array(c, "meetings"))
        .flat_map(|m| {
            let periods: Vec<_> = array(m, "periods")
                .iter()
                .filter_map(Value::as_u64)
                .collect();
            let (from, to) = match (periods.iter().min(), periods.iter().max()) {
                (Some(a), Some(b)) => (*a as u8, *b as u8),
                _ => return Vec::new(),
            };
            array(m, "days")
                .iter()
                .filter_map(|d| {
                    d.as_u64().map(|day| BusySlot {
                        day: day as u8,
                        from,
                        to,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let ctx = FinderContext {
        current_quarter: text(workspace, "currentTerm")
            .chars()
            .rev()
            .nth(1)
            .and_then(|c| c.to_digit(10))
            .unwrap_or(1) as u8,
        recommended,
        departments,
        busy,
        official_options: workspace
            .get("kyomuSearchOptions")
            .map(|options| {
                serde_json::from_value(options.clone())
                    .map_err(|e| FinderError::InvalidInput(e.to_string()))
            })
            .transpose()?
            .unwrap_or_default(),
    };
    let language = crate::schedule::content_language(workspace);
    let courses = offerings
        .into_iter()
        .filter_map(normalize_offering)
        .map(|mut course| {
            let url = crate::catalog::syllabus_url(text(&course, "id"), language);
            course["syllabusUrl"] = url.into();
            course
        })
        .collect::<Vec<_>>();
    let eligibility = courses
        .iter()
        .map(|course| {
            (
                text(course, "id").to_string(),
                catalog_course_eligibility(workspace, &request.term, course),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let ineligible_ids = eligibility
        .iter()
        .filter(|(_, result)| result.status == CourseEligibilityStatus::Ineligible)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let ineligible = ineligible_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut keys = request.keys;
    match &request.action {
        Some(FinderAction::Commit { key }) => keys.push(key.clone()),
        Some(FinderAction::ToggleTime { key, append }) => {
            if keys.contains(key) {
                keys.retain(|k| k != key);
            } else {
                if !append {
                    keys.retain(|k| condition(k, &ctx).r#type != "time" && k != "集中");
                }
                keys.push(key.clone());
            }
        }
        _ => {}
    }
    let parsed = factors_of(&keys, &ctx);
    if parsed
        .factors
        .iter()
        .filter(|f| f.r#type == "exclude")
        .flat_map(|f| &f.alts)
        .any(|c| c.r#type.starts_with("official-"))
    {
        return Err(FinderError::InvalidInput(
            "Official form filters support inclusion only.".into(),
        ));
    }
    let keys: Vec<_> = parsed
        .factors
        .iter()
        .flat_map(|f| f.alts.iter().map(|c| c.key.clone()))
        .collect();
    let base = courses.clone();
    let eligible_base = base
        .iter()
        .filter(|course| !ineligible.contains(text(course, "id")))
        .cloned()
        .collect::<Vec<_>>();
    let visible_base = if request.include_ineligible {
        &base
    } else {
        &eligible_base
    };
    let live = live_text(&request.input, &keys, visible_base, &ctx);
    let factors = parsed.factors.clone();
    let rows = run(visible_base, &factors, &ctx, &live);
    let suggestions = suggest(&request.input, &keys, visible_base, &ctx);
    let commit_key =
        if request.input.ends_with(char::is_whitespace) && !request.input.trim().is_empty() {
            let canonical = condition(request.input.trim(), &ctx);
            suggestions.first().and_then(|suggestion| match suggestion {
                Suggestion::Condition { key, condition, .. }
                    if condition.r#type != "text" && *key == canonical.key =>
                {
                    Some(key.clone())
                }
                _ => None,
            })
        } else {
            None
        };
    let active_slots = parsed
        .factors
        .iter()
        .filter(|f| f.r#type == "time")
        .flat_map(|f| &f.alts)
        .flat_map(|c| &c.values)
        .filter_map(|v| {
            Some(format!(
                "{}:{}-{}",
                v.get("day")?.as_u64()?,
                v.get("from")?.as_u64()?,
                v.get("to")?.as_u64()?
            ))
        })
        .collect();
    let replaceable: Vec<_> = placed
        .iter()
        .filter(|course| {
            array(workspace, "planItems").iter().any(|p| {
                text(p, "courseId") == text(course, "id")
                    && text(p, "termId") == request.term
                    && p.get("deletedAt").is_none_or(Value::is_null)
            })
        })
        .cloned()
        .collect();
    let replacements = courses
        .iter()
        .map(|c| {
            (
                text(c, "id").to_string(),
                clashing_courses(c, &replaceable)
                    .iter()
                    .map(|v| text(v, "id").to_string())
                    .collect(),
            )
        })
        .collect();
    let credits = courses
        .iter()
        .map(|c| (text(c, "id").to_string(), total_credits(c)))
        .collect();
    let clashes = courses
        .iter()
        .filter(|c| clashes(c, &ctx.busy))
        .map(|c| text(c, "id").to_string())
        .collect();
    let on_timetable = placed
        .iter()
        .map(|c| text(c, "code").to_string())
        .collect::<Vec<_>>();
    let mut paste_handled = false;
    let mut pasted_ids = Vec::new();
    let mut missing_codes = Vec::new();
    let mut ineligible_codes = Vec::new();
    if let Some(FinderAction::Paste { text: raw }) = &request.action {
        if raw.len() > 64 * 1024 {
            return Err(FinderError::InvalidInput(
                "Pasted course list is too large.".into(),
            ));
        }
        let normalized = normalized(raw);
        let regex = Regex::new(r"(?i)[a-z]{3}\.[a-z][0-9]{3}(?:-[0-9]{2})?")
            .map_err(|e| FinderError::InvalidInput(e.to_string()))?;
        let mut seen = HashSet::new();
        let codes = regex
            .find_iter(&normalized)
            .map(|m| m.as_str().to_uppercase())
            .filter(|code| seen.insert(code.clone()))
            .collect::<Vec<_>>();
        paste_handled = codes.len() >= 2;
        for code in codes.into_iter().filter(|_| paste_handled) {
            let quarter = request.term.rsplit('-').next().unwrap_or("");
            let found = courses
                .iter()
                .find(|c| text(c, "code") == code && array(c, "quarters").contains(&json!(quarter)))
                .or_else(|| courses.iter().find(|c| text(c, "code") == code));
            if let Some(c) = found {
                if ineligible.contains(text(c, "id")) {
                    ineligible_codes.push(code);
                } else if !on_timetable.contains(&code) {
                    pasted_ids.push(text(c, "id").into());
                }
            } else {
                missing_codes.push(code);
            }
        }
    }
    Ok(FinderView {
        context: ctx,
        courses,
        factors: parsed.factors,
        duplicates: parsed.duplicates,
        keys,
        rows,
        suggestions,
        live,
        clashes,
        ineligible_ids,
        eligibility,
        replacements,
        credits,
        on_timetable,
        active_slots,
        commit_key,
        paste_handled,
        pasted_ids,
        missing_codes,
        ineligible_codes,
    })
}

pub fn normalize_offering(mut row: Value) -> Option<Value> {
    if !row.is_object()
        || !row.get("id").is_some_and(Value::is_string)
        || !row.get("code").is_some_and(Value::is_string)
        || !row.get("quarters").is_some_and(Value::is_array)
        || !row.get("title").is_some_and(Value::is_object)
    {
        return None;
    }
    if !row.get("schedule").is_some_and(Value::is_array) {
        row["schedule"] = json!([]);
    }
    if !row.get("credits").is_some_and(Value::is_object) {
        row["credits"] = json!({"lecture":0,"exercise":0,"lab":0});
    }
    if !row.get("instructors").is_some_and(Value::is_array) {
        row["instructors"] = json!([]);
    }
    if !row.pointer("/format/kinds").is_some_and(Value::is_array) {
        row["format"] = json!({"kinds":[]});
    }
    if !row.get("instructionLanguage").is_some_and(Value::is_string) {
        row["instructionLanguage"] = json!("other");
    }
    Some(row)
}

#[cfg(test)]
#[path = "../tests/course_query.rs"]
mod tests;

#[cfg(feature = "ts")]
pub fn declarations() -> Vec<String> {
    vec![
        TimeValue::decl(),
        FinderCourse::decl(),
        FinderSchedule::decl(),
        FinderRow::decl(),
        Suggestion::decl(),
        OfficialSelection::decl(),
        OfficialFilterOption::decl(),
        FinderContext::decl(),
        Recommendation::decl(),
        FinderDepartment::decl(),
        BusySlot::decl(),
        Condition::decl(),
        Factor::decl(),
        Factors::decl(),
        FinderRequest::decl(),
        FinderAction::decl(),
        FinderView::decl(),
    ]
}
