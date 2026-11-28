use super::FinderContext;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::LazyLock;
#[cfg(feature = "ts")]
use ts_rs::TS;
use unicode_normalization::UnicodeNormalization;

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

pub(super) fn normalized(value: &str) -> String {
    value.nfkc().collect()
}
pub(super) fn lower(value: &str) -> String {
    normalized(value).to_lowercase()
}
fn false_value(value: &bool) -> bool {
    !*value
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
pub(crate) struct Factors {
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
            .chain(&d.names.ja)
            .chain(&d.names.en)
            .any(|v| lower(v) == lw)
    }) {
        let key = d.names.ja.as_ref().unwrap_or(&d.id);
        let label = d.names.ja.as_ref().or(d.names.en.as_ref()).unwrap_or(&d.id);
        return make("dept", key, label, json!(d.unit_code));
    }
    make("text", w, w, json!(lw))
}

pub(crate) fn condition(key: &str, ctx: &FinderContext) -> Condition {
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
pub(crate) fn factors_of(keys: &[String], ctx: &FinderContext) -> Factors {
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

// Stable keyboard categories; the authenticated form supplies their choices.
pub(super) const OFFICIAL_GROUPS: [(&str, &str, &[&str]); 9] = [
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
pub(super) fn official_group(input: &str) -> Option<(&'static str, &str)> {
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
pub(super) fn vocabulary(ctx: &FinderContext) -> Vec<(String, Vec<String>)> {
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
            d.names.ja.as_ref().unwrap_or(&d.id).clone(),
            std::iter::once(&d.id)
                .chain(&d.names.ja)
                .chain(&d.names.en)
                .filter(|v| !v.is_empty())
                .cloned()
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

/// The typed text that narrows the rows as it is typed, also while the
/// suggestions offer a condition it names (物理 narrows by text while 物理学系
/// is offered). None where text matching means nothing: an empty input, an
/// official category such as `level:`, an exclusion, or the syntax of a
/// condition itself (`4Q`, `火3-4`).
pub(crate) fn live_text(input: &str, ctx: &FinderContext) -> String {
    let value = input.trim();
    if value.is_empty()
        || official_group(value).is_some()
        || value.starts_with('!')
        || condition(value, ctx).r#type != "text"
    {
        String::new()
    } else {
        value.into()
    }
}
