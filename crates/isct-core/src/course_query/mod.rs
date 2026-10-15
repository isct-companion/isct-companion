//! The course finder's product grammar, shared by all shells.
//! Its pills and syllabus fan-out rules are application-specific; regex and
//! Unicode normalization handle the parsing primitives.
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    sync::LazyLock,
};
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
#[serde(rename_all = "camelCase")]
pub struct FinderContext {
    pub current_quarter: u8,
    #[serde(default)]
    pub recommended: BTreeMap<String, Recommendation>,
    #[serde(default)]
    pub departments: Vec<Department>,
    #[serde(default)]
    pub busy: Vec<BusySlot>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Recommendation {
    pub group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Department {
    pub id: String,
    pub name: BTreeMap<String, String>,
    pub unit_code: String,
    #[serde(default)]
    pub scope: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BusySlot {
    pub day: u8,
    pub from: u8,
    pub to: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Condition {
    pub r#type: String,
    pub key: String,
    pub label: String,
    pub values: Vec<Value>,
    #[serde(default, skip_serializing_if = "false_value")]
    pub prefix: bool,
    #[serde(default, skip_serializing_if = "false_value")]
    pub exclude: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Factor {
    pub r#type: String,
    pub alts: Vec<Condition>,
}
#[derive(Clone, Debug, Serialize)]
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
        captures!(r"^([1-6])00(?:番台|-level)$", w).or_else(|| captures!(r"^([1-6])xx$", &lw))
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
fn remote_factor(f: &Factor) -> bool {
    f.r#type != "exclude" && ["dept", "code", "level", "text"].contains(&f.r#type.as_str())
}
fn hit(course: &Value, c: &Condition, ctx: &FinderContext) -> bool {
    c.values.iter().any(|v| match c.r#type.as_str() {
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
pub fn run(courses: &[Value], factors: &[Factor], ctx: &FinderContext, live: &str) -> Vec<Value> {
    let live = lower(live.trim());
    let mut seen = HashSet::new();
    let mut rows:Vec<Value>=courses.iter().filter(|c| {
        (live.is_empty() || searchable(c).contains(&live)) && factors.iter().all(|f|if f.r#type=="exclude" {f.alts.first().is_none_or(|a|!hit(c,a,ctx))} else {f.alts.iter().any(|a|hit(c,a,ctx))}) && seen.insert(text(c,"id"))
    }).map(|c|json!({"course":c,"matched":factors.iter().filter(|f|f.alts.len()>1 && f.r#type!="exclude").flat_map(|f|f.alts.iter().filter(|a|hit(c,a,ctx)).map(|a|a.label.clone())).collect::<Vec<_>>()})).collect();
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
        weight(&b["course"])
            .cmp(&weight(&a["course"]))
            .then_with(|| text(&a["course"], "code").cmp(text(&b["course"], "code")))
    });
    rows
}
pub fn remote_plan(factors: &[Factor], scopes: &BTreeMap<String, String>) -> Value {
    let alts = |kind: &str| {
        factors
            .iter()
            .filter(move |f| f.r#type == kind)
            .flat_map(|f| &f.alts)
            .collect::<Vec<_>>()
    };
    let words = alts("text")
        .iter()
        .map(|c| c.key.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let scoped: Vec<_> = alts("dept")
        .iter()
        .filter_map(|c| {
            c.values
                .first()?
                .as_str()
                .and_then(|v| scopes.get(v))
                .cloned()
        })
        .collect();
    let codes: Vec<_> = alts("code").iter().map(|c| c.key.clone()).collect();
    let groups: Vec<_> = alts("level")
        .iter()
        .filter_map(|c| c.values.first().map(Value::to_string))
        .collect();
    if words.is_empty() && scoped.is_empty() && codes.is_empty() {
        return json!({"unsupported":"empty"});
    }
    if scoped.len().max(1) * codes.len().max(1) > 4 {
        return json!({"unsupported":"too-many"});
    }
    let q = if words.is_empty() { "." } else { &words };
    let scoped = if scoped.is_empty() {
        vec![None]
    } else {
        scoped.into_iter().map(Some).collect()
    };
    let codes = if codes.is_empty() {
        vec![None]
    } else {
        codes.into_iter().map(Some).collect()
    };
    let mut queries = Vec::new();
    for scope in &scoped {
        for code in &codes {
            let mut query = json!({"q":q});
            if let Some(scope) = scope {
                query["scope"] = json!(scope);
            }
            if let Some(code) = code {
                query["code"] = json!(code);
            }
            if !groups.is_empty() {
                query["groups"] = json!(groups);
            }
            queries.push(query);
        }
    }
    json!({"queries":queries})
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
    if !ctx.recommended.is_empty() {
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
    for (key, aliases) in [
        ("200番台", vec!["200番台", "200", "2xx"]),
        ("300番台", vec!["300番台", "300", "3xx"]),
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
pub fn suggest(input: &str, keys: &[String], courses: &[Value], ctx: &FinderContext) -> Vec<Value> {
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
    let add = |key: &str, out: &mut Vec<Value>| {
        let full = if exclude {
            format!("!{key}")
        } else {
            key.into()
        };
        if keys.contains(&full)
            || out
                .iter()
                .any(|v| text(v, "kind") == "condition" && text(v, "key") == full)
        {
            return;
        }
        let c = condition(&full, ctx);
        if ctx.recommended.is_empty() && ["mark", "group"].contains(&c.r#type.as_str()) {
            return;
        }
        let remote = !c.exclude && ["dept", "code", "level", "text"].contains(&c.r#type.as_str());
        out.push(json!({"kind":"condition","key":full,"condition":c,"count":if remote{-1}else{count(&full) as i64}}));
    };
    if typed.is_empty() {
        for key in if exclude {
            &["集中", "特課研", "英語", "演習"][..]
        } else {
            &["curr", "推荐", "空き", "講義", "演習", "英語"][..]
        } {
            if *key != "推荐" || !ctx.recommended.is_empty() {
                add(key, &mut out);
            }
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
            out.push(json!({"kind":"course","course":course}));
        }
    }
    let key = if exclude {
        format!("!{typed}")
    } else {
        typed.into()
    };
    if !keys.contains(&key) {
        out.push(json!({"kind":"text","key":key,"count":if key.starts_with('!'){count(&key) as i64}else{-1}}));
    }
    out
}
pub fn live_text(input: &str, keys: &[String], courses: &[Value], ctx: &FinderContext) -> String {
    let value = input.trim();
    if value.is_empty()
        || value.starts_with('!')
        || condition(value, ctx).r#type != "text"
        || suggest(value, keys, courses, ctx)
            .iter()
            .any(|s| text(s, "kind") == "condition")
    {
        String::new()
    } else {
        value.into()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    /// None uses the stored catalog; an empty list represents a remote search in progress.
    #[serde(default)]
    pub remote_ids: Option<Vec<String>>,
    #[serde(default)]
    pub action: Option<FinderAction>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
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
#[serde(rename_all = "camelCase")]
pub struct FinderView {
    pub context: FinderContext,
    pub courses: Vec<Value>,
    pub factors: Vec<Factor>,
    pub duplicates: usize,
    pub keys: Vec<String>,
    pub plan: Value,
    pub rows: Vec<Value>,
    pub suggestions: Vec<Value>,
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
    pub detail_ids: Vec<String>,
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
    let departments: Vec<Department> = serde_json::from_str(include_str!("departments.json"))?;
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
    if request
        .remote_ids
        .as_ref()
        .is_some_and(|ids| ids.len() > 10000 || ids.iter().any(|id| id.len() > 2048))
    {
        return Err(FinderError::InvalidInput(
            "Too many syllabus results.".into(),
        ));
    }
    if matches!(&request.action,Some(FinderAction::Commit{key}|FinderAction::ToggleTime{key,..}) if key.len()>2048)
    {
        return Err(FinderError::InvalidInput(
            "Course condition is too long.".into(),
        ));
    }
    let departments: Vec<Department> = serde_json::from_str(include_str!("departments.json"))?;
    let selected_department = workspace
        .pointer("/settings/departmentId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let admission = workspace.pointer("/academicProfile/admissionYear");
    let recommended = array(workspace, "recommendedCourses")
        .iter()
        .filter(|r| {
            r.get("deletedAt").is_none_or(Value::is_null)
                && r.get("admissionYear") == admission
                && catalog_course_eligibility(workspace, &request.term, r).status
                    != CourseEligibilityStatus::Ineligible
                && !selected_department.is_empty()
                && (text(r, "departmentId") == selected_department
                    || selected_department.ends_with(&format!(":{}", text(r, "departmentId"))))
        })
        .map(|r| {
            (
                text(r, "code").to_string(),
                Recommendation {
                    group: text(r, "group").into(),
                    mark: r.get("mark").and_then(Value::as_str).map(String::from),
                },
            )
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
    };
    let courses = offerings
        .into_iter()
        .filter_map(normalize_offering)
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
    let keys: Vec<_> = parsed
        .factors
        .iter()
        .flat_map(|f| f.alts.iter().map(|c| c.key.clone()))
        .collect();
    let scopes = ctx
        .departments
        .iter()
        .filter_map(|d| d.scope.clone().map(|scope| (d.unit_code.clone(), scope)))
        .collect();
    let plan = remote_plan(&parsed.factors, &scopes);
    let base = match &request.remote_ids {
        Some(ids) => {
            let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
            courses
                .iter()
                .filter(|c| ids.contains(text(c, "id")))
                .cloned()
                .collect()
        }
        None => courses.clone(),
    };
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
    let factors = if request.remote_ids.is_some() {
        parsed
            .factors
            .iter()
            .filter(|f| !remote_factor(f))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        parsed.factors.clone()
    };
    let rows = run(visible_base, &factors, &ctx, &live);
    let suggestions = suggest(&request.input, &keys, visible_base, &ctx);
    let commit_key =
        if request.input.ends_with(char::is_whitespace) && !request.input.trim().is_empty() {
            let canonical = condition(request.input.trim(), &ctx);
            suggestions
                .first()
                .filter(|v| {
                    text(v, "kind") == "condition"
                        && v.pointer("/condition/type").and_then(Value::as_str) != Some("text")
                        && text(v, "key") == canonical.key
                })
                .map(|v| text(v, "key").to_string())
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
    let quarter_factors = parsed
        .factors
        .iter()
        .filter(|f| f.r#type == "quarter")
        .cloned()
        .collect::<Vec<_>>();
    let detail_ids = run(visible_base, &quarter_factors, &ctx, "")
        .iter()
        .filter(|row| row.pointer("/course/detailed") == Some(&Value::Bool(false)))
        .map(|row| text(&row["course"], "id").to_string())
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
        plan,
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
        detail_ids,
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
    row["detailed"] = json!(
        row.get("schedule").is_some_and(Value::is_array)
            && row.get("credits").is_some_and(Value::is_object)
    );
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
mod tests {
    use super::*;
    fn eligibility_offering(id: &str, code: &str) -> Value {
        json!({
            "id":id, "year":2026, "code":code, "quarters":["3Q"],
            "unitCode":"342300", "title":{"en":code}, "format":{"kinds":[]}
        })
    }

    #[test]
    fn first_year_eligibility_uses_the_planned_year_and_keeps_all_course_search_visible() {
        let offerings = vec![
            eligibility_offering("100", "CSC.T101"),
            eligibility_offering("200", "CSC.T201"),
            eligibility_offering("humanities", "LAH.T209-01"),
            eligibility_offering("law", "LAW.W221-12"),
            eligibility_offering("300", "CSC.T301"),
        ];
        let workspace = json!({
            "currentTerm":"2026-3Q",
            "academicProfile":{"admissionYear":2026,"studyEnrollments":[{
                "stage":"undergraduate","entryKind":"regular","entryGrade":1,
                "admittedTerm":"2026-1Q"
            }]},
            "settings":{"departmentId":"cs"},
            "recommendedCourses":[
                {"admissionYear":2026,"departmentId":"cs","code":"CSC.T101","group":"A"},
                {"admissionYear":2026,"departmentId":"cs","code":"CSC.T201","group":"B"}
            ],
            "courses":[], "memberships":[], "planItems":[]
        });
        let request = |term: &str,
                       keys: Vec<&str>,
                       include_ineligible: bool,
                       action: Option<FinderAction>| FinderRequest {
            year: term[..4].parse().unwrap(),
            term: term.into(),
            include_ineligible,
            keys: keys.into_iter().map(str::to_string).collect(),
            input: String::new(),
            remote_ids: None,
            action,
        };

        let default = finder_view(
            &workspace,
            offerings.clone(),
            request("2026-3Q", vec!["3Q"], false, None),
        )
        .unwrap();
        assert_eq!(
            default.ineligible_ids,
            vec!["200", "300", "humanities", "law"]
        );
        assert_eq!(
            default.eligibility["100"].status,
            CourseEligibilityStatus::Allowed
        );
        assert_eq!(
            default.eligibility["humanities"].reason,
            crate::study_profile::CourseEligibilityReason::UndergraduateFirstYearUpperLevel
        );
        assert_eq!(
            default
                .rows
                .iter()
                .map(|row| text(&row["course"], "id"))
                .collect::<Vec<_>>(),
            vec!["100"]
        );
        assert!(default.context.recommended.contains_key("CSC.T101"));
        assert!(!default.context.recommended.contains_key("CSC.T201"));

        let all = finder_view(
            &workspace,
            offerings.clone(),
            request("2026-3Q", vec!["3Q"], true, None),
        )
        .unwrap();
        assert_eq!(
            all.rows.len(),
            5,
            "explicit all-course search stays visible"
        );

        let pasted = finder_view(
            &workspace,
            offerings.clone(),
            request(
                "2026-3Q",
                vec![],
                false,
                Some(FinderAction::Paste {
                    text: "CSC.T101 CSC.T201 LAH.T209-01 LAW.W221-12 CSC.T301".into(),
                }),
            ),
        )
        .unwrap();
        assert_eq!(pasted.pasted_ids, vec!["100"]);
        assert_eq!(
            pasted.ineligible_codes,
            vec!["CSC.T201", "LAH.T209-01", "LAW.W221-12", "CSC.T301"]
        );

        let second_year = finder_view(
            &workspace,
            offerings.clone(),
            request("2027-3Q", vec![], false, None),
        )
        .unwrap();
        assert!(second_year.ineligible_ids.is_empty());

        let mut unknown = workspace;
        unknown["academicProfile"] = json!({});
        let unknown =
            finder_view(&unknown, offerings, request("2026-3Q", vec![], false, None)).unwrap();
        assert!(unknown.ineligible_ids.is_empty());
        assert!(unknown
            .eligibility
            .values()
            .all(|result| result.status == CourseEligibilityStatus::Unknown));
    }

    #[test]
    fn eligibility_does_not_guess_without_study_identity_or_course_number() {
        let upper_level = eligibility_offering("upper", "CSC.T301");
        assert_eq!(
            catalog_course_eligibility(&json!({"academicProfile":{}}), "2026-3Q", &upper_level)
                .status,
            CourseEligibilityStatus::Unknown
        );
        let undergraduate = json!({"academicProfile":{"studyEnrollments":[{
            "stage":"undergraduate","entryKind":"regular","entryGrade":1,
            "admittedTerm":"2026-1Q"
        }]}});
        assert_eq!(
            catalog_course_eligibility(
                &undergraduate,
                "2026-3Q",
                &eligibility_offering("special", "SPECIAL")
            )
            .status,
            CourseEligibilityStatus::Unknown
        );
        assert_eq!(
            catalog_course_eligibility(
                &undergraduate,
                "2026-3Q",
                &eligibility_offering("foreign", "SPECIAL.2026")
            )
            .status,
            CourseEligibilityStatus::Unknown
        );
    }

    #[test]
    fn replacement_ignores_same_course_and_nonweekly_results() {
        let fixture: Value = serde_json::from_str(include_str!("fixtures.json")).unwrap();
        let placed = json!([
            {"code":"X.1","meetings":[{"days":[2],"periods":[3,4]}]},
            {"code":"X.2","meetings":[{"days":[2],"periods":[5,6]}]},
            {"code":"X.3","meetings":[{"days":[],"periods":[]}]},
            {"code":"CSC.T262","meetings":[{"days":[2],"periods":[3,4]}]}
        ]);
        assert_eq!(
            clashing_courses(&fixture["courses"][0], placed.as_array().unwrap())
                .iter()
                .map(|row| text(row, "code"))
                .collect::<Vec<_>>(),
            vec!["X.1"]
        );
        assert!(clashing_courses(&fixture["courses"][3], placed.as_array().unwrap()).is_empty());
    }
    #[test]
    fn a_three_q_four_q_course_keeps_its_periods_busy_in_four_q() {
        let workspace = json!({
            "currentTerm":"2026-4Q", "settings":{},
            "courses":[{"id":"multi","termId":"2026-3Q","code":"MTH.B301",
                "meetings":[{"days":[1],"periods":[1,2]}]}],
            "memberships":[],
            "planItems":[
                {"id":"plan-3q","termId":"2026-3Q","courseId":"multi"},
                {"id":"plan-4q","termId":"2026-4Q","courseId":"multi"}
            ]
        });
        let view = finder_view(
            &workspace,
            vec![],
            serde_json::from_value(json!({"year":2026,"term":"2026-4Q"})).unwrap(),
        )
        .unwrap();
        assert_eq!(view.context.busy.len(), 1);
        assert_eq!(
            (
                view.context.busy[0].day,
                view.context.busy[0].from,
                view.context.busy[0].to
            ),
            (1, 1, 2)
        );
    }
    #[test]
    fn partial_offerings_have_stable_finder_defaults() {
        let row = normalize_offering(
            json!({"id":"a","code":"ABC.A123","title":{"ja":"A"},"quarters":["3Q"],"format":{}}),
        )
        .unwrap();
        assert_eq!(row["detailed"], false);
        assert_eq!(row["instructionLanguage"], "other");
        assert_eq!(row["format"]["kinds"], json!([]));
        assert_eq!(total_credits(&row), 0.0);
    }
    #[test]
    fn matches_original_typescript_finder_traces() {
        let fixture: Value = serde_json::from_str(include_str!("fixtures.json")).unwrap();
        let ctx: FinderContext = serde_json::from_value(fixture["context"].clone()).unwrap();
        let scopes: BTreeMap<String, String> =
            serde_json::from_value(fixture["scopes"].clone()).unwrap();
        let courses = fixture["courses"].as_array().unwrap();
        for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            let mut ctx = ctx.clone();
            let input = &case["input"];
            if input["bare"] == true {
                ctx.recommended.clear();
            }
            let keys: Vec<String> =
                serde_json::from_value(input.get("keys").cloned().unwrap_or(json!([]))).unwrap();
            let factors = factors_of(&keys, &ctx);
            let actual = match text(case, "kind") {
                "condition" => serde_json::to_value(condition(text(input, "word"), &ctx)).unwrap(),
                "factors" => serde_json::to_value(factors).unwrap(),
                "run" => json!(run(courses, &factors.factors, &ctx, "")),
                "plan" => remote_plan(&factors.factors, &scopes),
                "suggest" => json!(suggest(text(input, "input"), &keys, courses, &ctx)),
                "live" => json!(live_text(text(input, "input"), &keys, courses, &ctx)),
                other => panic!("unknown trace {other}"),
            };
            assert_eq!(actual, case["expected"], "case {index}: {}", case["input"]);
        }
    }
}
