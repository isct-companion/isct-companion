use super::{
    clashes, condition,
    conditions::{lower, normalized, official_group, vocabulary, OFFICIAL_GROUPS},
    factors_of,
    personal::{overlap, weekly},
    total_credits, Condition, Factor, FinderContext, FinderRowView, Suggestion,
};
use crate::{
    locale::{pick, ContentLanguage},
    schedule::{array, text_or_empty as text},
};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};

/// A catalog row as the finder lists it, its names in the content language,
/// with the lowercase text of both languages a typed word is looked up in,
/// built once per catalog rather than per search.
pub struct IndexedCourse {
    pub course: Arc<Value>,
    haystack: String,
    teachers: Vec<String>,
    rooms: Vec<String>,
}
impl IndexedCourse {
    pub fn new(mut course: Value, language: ContentLanguage) -> Self {
        let haystack = searchable(&course);
        let both = |names: &[&Value]| {
            names
                .iter()
                .flat_map(|name| ["ja", "en"].map(|lang| lower(text(name, lang))))
                .filter(|name| !name.is_empty())
                .collect::<Vec<String>>()
        };
        let teachers = both(&array(&course, "instructors").iter().collect::<Vec<_>>());
        let rooms = both(
            &array(&course, "schedule")
                .iter()
                .filter_map(|slot| slot.get("room"))
                .collect::<Vec<_>>(),
        );
        let title = pick(course.get("title"), language)
            .unwrap_or_default()
            .to_owned();
        course["title"] = title.into();
        let instructors: Vec<Value> = array(&course, "instructors")
            .iter()
            .filter_map(|name| pick(Some(name), language))
            .map(Value::from)
            .collect();
        course["instructors"] = instructors.into();
        for slot in course["schedule"].as_array_mut().into_iter().flatten() {
            let room = pick(slot.get("room"), language).map(str::to_owned);
            if let Some(slot) = slot.as_object_mut() {
                match room {
                    Some(room) => slot.insert("room".into(), room.into()),
                    None => slot.remove("room"),
                };
            }
        }
        Self {
            course: Arc::new(course),
            haystack,
            teachers,
            rooms,
        }
    }
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
fn hit(indexed: &IndexedCourse, c: &Condition, ctx: &FinderContext) -> bool {
    let course = &indexed.course;
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
        "teacher" => v
            .as_str()
            .is_some_and(|needle| indexed.teachers.iter().any(|name| name.contains(needle))),
        "room" => v
            .as_str()
            .is_some_and(|needle| indexed.rooms.iter().any(|room| room.contains(needle))),
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
        "text" => v.as_str().is_some_and(|s| indexed.haystack.contains(s)),
        _ => false,
    })
}
fn matches(c: &IndexedCourse, factors: &[Factor], ctx: &FinderContext, live: &str) -> bool {
    (live.is_empty() || c.haystack.contains(live))
        && factors.iter().all(|f| {
            if f.r#type == "exclude" {
                f.alts.first().is_none_or(|a| !hit(c, a, ctx))
            } else {
                f.alts.iter().any(|a| hit(c, a, ctx))
            }
        })
}
/// How many rows [`run`] would return, without building them.
fn count_rows(courses: &[&IndexedCourse], factors: &[Factor], ctx: &FinderContext) -> usize {
    let mut seen = HashSet::new();
    courses
        .iter()
        .filter(|c| matches(c, factors, ctx, "") && seen.insert(text(&c.course, "id")))
        .count()
}
/// A row that met every condition, with the either-or labels it met.
pub struct FinderMatch<'a> {
    pub course: &'a IndexedCourse,
    pub matched: Vec<String>,
}
/// Every matching row, recommended first, then by code.
pub fn run<'a>(
    courses: &[&'a IndexedCourse],
    factors: &[Factor],
    ctx: &FinderContext,
    live: &str,
) -> Vec<FinderMatch<'a>> {
    let live = lower(live.trim());
    let mut seen = HashSet::new();
    let mut rows: Vec<FinderMatch<'a>> = courses
        .iter()
        .filter(|c| matches(c, factors, ctx, &live) && seen.insert(text(&c.course, "id")))
        .map(|c| FinderMatch {
            course: c,
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
        weight(&b.course.course)
            .cmp(&weight(&a.course.course))
            .then_with(|| text(&a.course.course, "code").cmp(text(&b.course.course, "code")))
    });
    rows
}

/// What a typed word offers: categories, conditions with their counts,
/// matching courses (as `row` presents them) and the word itself.
pub fn suggest(
    input: &str,
    keys: &[String],
    courses: &[&IndexedCourse],
    ctx: &FinderContext,
    row: &dyn Fn(&IndexedCourse) -> FinderRowView,
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
        count_rows(courses, &factors_of(&keys, ctx).factors, ctx)
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
                (text(&c.course, "code").starts_with(&upper) || c.haystack.contains(&lt))
                    && seen.insert(text(&c.course, "code"))
            })
            .collect();
        matching.sort_by_key(|c| !text(&c.course, "code").starts_with(&upper));
        for c in matching.into_iter().take(6) {
            out.push(Suggestion::Course { row: row(c) });
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
