//! Reader for the six approved 教務Web pages, fetched by `net::kyomu`.
//!
//! It produces the same `KyomuPayload` the former injected reader produced,
//! so validation (`parse_payload`) and persistence (`apply_kyomu_payload`)
//! are unchanged. Structural checks reject a page whose shape is unfamiliar:
//! a missing table, an unknown header or an empty list without the page's
//! own "no data" text is an incomplete read, never an empty one.

use super::html::{
    by_id, cells, clean, clean_text, closest, corner_depth, explicit_empty, explicit_empty_element,
    first, group, has_class, heading_before, is, js_number, jwc, lines, maintenance_notice, matrix,
    next_element_sibling, nine_digits, select, table_data, table_rows, text_at, text_content,
    thead_rows,
};
use super::{expected_kind, KyomuPayload, HOST};
use scraper::{ElementRef, Html};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

type Read = Result<Complete, &'static str>;

struct Complete {
    shape: &'static str,
    data: Map<String, Value>,
    empty: bool,
}

fn complete(shape: &'static str, data: Value, empty: bool) -> Read {
    Ok(Complete {
        shape,
        data: match data {
            Value::Object(map) => map,
            _ => Map::new(),
        },
        empty,
    })
}

/// Reads one fetched page; `source_url` is the URL that was requested.
pub fn extract_page(source_url: &str, html: &str) -> KyomuPayload {
    let document = Html::parse_document(html);
    let url = url::Url::parse(source_url).ok();
    let kind = url.as_ref().and_then(|url| expected_kind(url.path()));
    let result = |status: &str, data: Value, message: &str| KyomuPayload {
        status: status.into(),
        source_url: source_url.into(),
        kind: kind.unwrap_or("unknown").into(),
        data,
        message: message.into(),
    };
    let https = url.as_ref().is_some_and(|url| url.scheme() == "https");
    let host = url.as_ref().and_then(|url| url.host_str()).unwrap_or("");
    if https && host == HOST {
        if let Some(notice) = maintenance_notice(&document) {
            return result("maintenance", json!({}), &notice);
        }
    }
    let root = document.root_element();
    let login_form = select(root, "form")
        .into_iter()
        .find(|form| {
            form.attr("action")
                .is_some_and(|action| action.to_lowercase().contains("login"))
        })
        .or_else(|| {
            select(root, "input")
                .into_iter()
                .find(|input| {
                    input
                        .attr("type")
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("password"))
                })
                .and_then(|input| closest(input, |element| is(element, "form")))
        });
    let known_login_host =
        [HOST, "portal.titech.ac.jp", "rp.nap.gsic.titech.ac.jp"].contains(&host);
    if https && known_login_host && login_form.is_some() {
        return result("login", json!({}), "");
    }
    let Some(kind) = kind.filter(|_| https && host == HOST) else {
        return result("error", json!({}), "The source page is not approved.");
    };
    // Only an explicit message in the page's error region proves that a page
    // is not offered to this account. A missing table or unfamiliar DOM is an
    // error.
    let unavailable = select(
        root,
        "#ctl00_ContentPlaceHolder1_lblError, .errorMessage, .validation-summary-errors, [role=alert]",
    )
    .into_iter()
    .map(clean_text)
    .find(|value| {
        regex!(r"(?:このページ|本機能|当機能).*(?:利用できません|対象外)")
            .is_some_and(|pattern| pattern.is_match(value))
    });
    if let Some(message) = unavailable {
        return result("unavailable", json!({}), &message);
    }
    let base = url.as_ref();
    let read = match kind {
        "profile" => profile(&document),
        "registrations" => registrations(&document, base),
        "declared" => declared(&document, base),
        "grades" => grades(&document),
        "requirements" => requirements(&document),
        _ => gpa(&document),
    };
    match read {
        Ok(Complete { shape, data, empty }) => {
            let mut object = Map::new();
            object.insert("complete".into(), true.into());
            object.insert("shape".into(), shape.into());
            object.insert("empty".into(), empty.into());
            object.extend(data);
            result("ok", Value::Object(object), "")
        }
        Err(message) => result("error", json!({}), message),
    }
}

/// A registration requirement mark such as `A (◎)`, or the raw text.
fn requirement(raw: &str) -> Value {
    let raw = clean(raw);
    match regex!(r"^([A-Z])\s*\((◎|○|選択)\)$").and_then(|pattern| pattern.captures(&raw)) {
        Some(found) => json!({
            "category": group(&found, 1),
            "mark": match group(&found, 2) {
                "◎" => "required",
                "○" => "recommended",
                _ => "elective",
            },
        }),
        None => json!({ "raw": raw }),
    }
}

/// `[2-1-0]` lecture/exercise/lab credits, or null.
fn credits(raw: &str) -> Value {
    regex!(r"\[\s*([0-9.]+)-([0-9.]+)-([0-9.]+)\s*\]")
        .and_then(|pattern| pattern.captures(raw))
        .map_or(Value::Null, |found| {
            json!({
                "lecture": js_number(group(&found, 1)),
                "exercise": js_number(group(&found, 2)),
                "lab": js_number(group(&found, 3)),
            })
        })
}

/// `1-2Q: 月 3-4 (room)` lines expanded per quarter; other lines stay raw.
fn schedule(rows: &[String]) -> Vec<Value> {
    let Some(pattern) = regex!(
        r"^([1-4])(?:\s*[~〜-]\s*([1-4]))?Q[:：]\s*([月火水木金土日])\s*([0-9]+)\s*[-–〜~]\s*([0-9]+)"
    ) else {
        return rows.iter().map(|part| json!({ "raw": part })).collect();
    };
    let mut out = Vec::new();
    for part in rows {
        let Some(found) = pattern.captures(part) else {
            out.push(json!({ "raw": part }));
            continue;
        };
        let from = group(&found, 1).parse::<u32>().unwrap_or(0);
        let to = match group(&found, 2) {
            "" => from,
            to => to.parse::<u32>().unwrap_or(0),
        };
        let (first, last) = (
            group(&found, 4).parse::<u64>(),
            group(&found, 5).parse::<u64>(),
        );
        // School periods are single digits; an absurd range is kept raw
        // rather than expanded.
        let (Ok(first), Ok(last)) = (first, last) else {
            out.push(json!({ "raw": part }));
            continue;
        };
        if last > first + 24 {
            out.push(json!({ "raw": part }));
            continue;
        }
        let periods: Vec<u64> = (first..=last).collect();
        let matched_end = found.get(0).map_or(0, |whole| whole.end());
        let rest = clean(part.get(matched_end..).unwrap_or(""));
        let room = rest
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .map_or(rest.clone(), str::to_string);
        for quarter in from..=to {
            out.push(json!({
                "quarter": format!("{quarter}Q"),
                "day": group(&found, 3),
                "periods": periods,
                "room": room,
            }));
        }
    }
    out
}

fn profile(document: &Html) -> Read {
    let root = by_id(document, "ctl00_ContentPlaceHolder1_divMain")
        .ok_or("The academic profile page shape is unknown.")?;
    let heading = select(root, "h2")
        .into_iter()
        .find(|node| text_content(*node).contains("【現在】"));
    let next = heading.and_then(next_element_sibling);
    let table = next.and_then(|next| {
        if is(next, "table") {
            Some(next)
        } else {
            first(next, "table")
        }
    });
    let (Some(heading), Some(table)) = (heading, table) else {
        return Err("The current academic profile is missing.");
    };
    // Only academic identity leaves the page. Names, student numbers,
    // contacts and other personal rows are deliberately excluded.
    let mut fields = HashMap::new();
    for row in table_rows(table) {
        let row = cells(row);
        fields.insert(
            row.first()
                .map(|cell| clean_text(*cell))
                .unwrap_or_default(),
            row.get(1).map(|cell| clean_text(*cell)).unwrap_or_default(),
        );
    }
    let field = |name: &str| fields.get(name).cloned().unwrap_or_default();
    let admitted = field("入学年月日");
    if field("所属").is_empty()
        || !regex!(r"^[0-9]{4}/[0-9]{2}/[0-9]{2}$")
            .is_some_and(|pattern| pattern.is_match(&admitted))
    {
        return Err("The academic profile fields are incomplete.");
    }
    complete(
        "academic-profile",
        json!({
            "department": field("所属"),
            "admittedOn": admitted.replace('/', "-"),
            "programme": clean_text(heading).replacen("【現在】", "", 1).trim(),
            "entryCategory": field("入学試験区分"),
        }),
        false,
    )
}

fn registration_state(raw: &str) -> &'static str {
    match raw {
        "一時保存" => "saved",
        "要エラーチェック" => "needs-check",
        "本登録可能" => "registrable",
        "承認待ち" | "要承認申請" | "承認待ち / 要承認申請" | "承認待ち/要承認申請" => {
            "needs-approval"
        }
        "本登録不可" => "rejected",
        "本登録" | "本登録済" => "registered",
        _ => "unknown",
    }
}

fn timetable_times(block: ElementRef<'_>) -> Vec<String> {
    lines(first(block, ".ttYoubi").or_else(|| first(block, ".ttYoubi2")))
}

fn course_tables(document: &Html) -> Vec<ElementRef<'_>> {
    select(
        document.root_element(),
        r#"table[id^="kamokuUnit_"][id$="_gvKamoku"]"#,
    )
}

/// Data rows of a GridView: every row after the first that has a cell with
/// text and is not the explicit empty-data row.
fn grid_rows(table: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    select(table, "tr")
        .into_iter()
        .skip(1)
        .filter(|row| {
            !explicit_empty_element(*row)
                && first(*row, "td").is_some()
                && !clean_text(*row).is_empty()
        })
        .collect()
}

fn registrations(document: &Html, base: Option<&url::Url>) -> Read {
    let root = document.root_element();
    by_id(document, "divSinkokuStatus").ok_or("The registration page shape is unknown.")?;
    let timetable = first(root, "table#timetable");
    let list_tables = course_tables(document);
    let Some(timetable) = timetable.filter(|_| !list_tables.is_empty()) else {
        return Err("The registration course area is missing.");
    };
    let raw_state = text_at(root, "#ctl00_ContentPlaceHolder1_ctl00_lblStatus2");
    let mut seen = HashSet::new();
    let mut timetable_index = HashMap::new();
    let mut courses: Vec<Value> = Vec::new();
    for block in select(root, "table#timetable [data-jwc]") {
        let offering_id = clean(block.attr("data-jwc").unwrap_or(""));
        if !nine_digits(&offering_id) {
            return Err("A registration course identifier is invalid.");
        }
        if let Some(index) = timetable_index.get(&offering_id).copied() {
            // A course in several cells: later cells may name the room of a
            // slot the first cell listed without one.
            let entries = schedule(&timetable_times(block));
            let rows = courses
                .get_mut(index)
                .and_then(|course| course.get_mut("schedule"))
                .and_then(Value::as_array_mut);
            if let Some(rows) = rows {
                for entry in entries {
                    let Some(room) = entry
                        .get("room")
                        .and_then(Value::as_str)
                        .filter(|room| !room.is_empty())
                    else {
                        continue;
                    };
                    if let Some(twin) = rows.iter_mut().find(|row| {
                        row.get("quarter") == entry.get("quarter")
                            && row.get("day") == entry.get("day")
                            && row.get("periods") == entry.get("periods")
                    }) {
                        twin["room"] = room.into();
                    }
                }
            }
            continue;
        }
        seen.insert(offering_id.clone());
        timetable_index.insert(offering_id.clone(), courses.len());
        let cell = closest(block, |element| is(element, "td"));
        let marked = |class: &str| {
            has_class(block, class) || cell.is_some_and(|cell| has_class(cell, class))
        };
        let course_state = if marked("timetableError") {
            "rejected"
        } else if [
            "timetableAlert1",
            "timetableAlert2",
            "timetableAlert3",
            "timetableAlert22",
        ]
        .iter()
        .any(|class| marked(*class))
        {
            "needs-approval"
        } else if marked("timetableUnchecked") {
            "needs-check"
        } else {
            "registrable"
        };
        courses.push(json!({
            "offeringId": offering_id,
            "title": text_at(block, ".ttKamokuName a"),
            "code": text_at(block, ".ttSinkokuNo"),
            "instructor": text_at(block, ".ttKyoin"),
            "schedule": schedule(&timetable_times(block)),
            "requirement": requirement(&text_at(block, ".ttSuisho")),
            "courseState": course_state,
            "declaredOnly": false,
        }));
    }
    for table in &list_tables {
        let headers: Vec<String> = select(*table, "tr:first-child th")
            .into_iter()
            .map(clean_text)
            .collect();
        // An empty GridView renders only its empty-data row, without a header.
        if headers.is_empty() && table_rows(*table).len() == 1 && explicit_empty_element(*table) {
            continue;
        }
        if !headers.iter().any(|header| header.contains("授業担当教員"))
            || !headers.iter().any(|header| header.contains("備考"))
        {
            return Err("A registration list header is incomplete.");
        }
        for row in grid_rows(*table) {
            let link = first(row, "a.kamokuGridKamokuName")
                .ok_or("A registration list row is incomplete.")?;
            let offering_id = jwc(base, link.attr("href"));
            if !nine_digits(&offering_id) {
                return Err("A registration list identifier is invalid.");
            }
            if !seen.insert(offering_id.clone()) {
                continue;
            }
            let values: Vec<String> = cells(row).into_iter().map(clean_text).collect();
            let at = |label: &str| {
                headers
                    .iter()
                    .position(|header| header.contains(label))
                    .and_then(|index| values.get(index))
                    .cloned()
                    .unwrap_or_default()
            };
            let times = first(row, "td.tdYoubi");
            courses.push(json!({
                "offeringId": offering_id,
                "title": clean_text(link),
                "code": text_at(row, "span.resultUnitKamokuCodeSP"),
                "instructor": at("授業担当教員"),
                "schedule": schedule(&lines(times)),
                "declaredOnly": times.map(clean_text).unwrap_or_default() == "申告専用",
                "requirement": requirement(&at("備考")),
                "credits": credits(&text_at(row, "span.resultUnitKamokuTaniSP")),
            }));
        }
    }
    let saved_raw = text_at(root, "#ctl00_ContentPlaceHolder1_ctl00_lblStatus3");
    let saved_at = regex!(r"([0-9]{4})/([0-9]{2})/([0-9]{2})\s+([0-9]{2}):([0-9]{2}):([0-9]{2})")
        .and_then(|pattern| pattern.captures(&saved_raw))
        .map_or(Value::Null, |found| {
            format!(
                "{}-{}-{}T{}:{}:{}+09:00",
                group(&found, 1),
                group(&found, 2),
                group(&found, 3),
                group(&found, 4),
                group(&found, 5),
                group(&found, 6)
            )
            .into()
        });
    if courses.is_empty()
        && !explicit_empty_element(timetable)
        && !list_tables
            .iter()
            .any(|table| explicit_empty_element(*table))
    {
        return Err("The empty registration list is not explicit.");
    }
    let quarters: Vec<String> = by_id(document, "rdoQAll")
        .and_then(|radio| radio.parent().and_then(ElementRef::wrap))
        .map(clean_text)
        .unwrap_or_default()
        .chars()
        .filter(|c| ('1'..='4').contains(c))
        .map(|quarter| format!("{quarter}Q"))
        .collect();
    let empty = courses.is_empty();
    complete(
        "registration-page",
        json!({
            "state": {
                "state": registration_state(&raw_state),
                "raw": raw_state,
                "savedAt": saved_at,
            },
            "courses": courses,
            "quarters": quarters,
        }),
        empty,
    )
}

fn declared(document: &Html, base: Option<&url::Url>) -> Read {
    let tables = course_tables(document);
    if tables.is_empty() {
        return Err("The declared-course page shape is unknown.");
    }
    let mut courses = Vec::new();
    for table in &tables {
        let unit = table
            .attr("id")
            .unwrap_or("")
            .strip_suffix("_gvKamoku")
            .unwrap_or("")
            .to_string();
        let sum_text = by_id(
            document,
            &format!("ctl00_ContentPlaceHolder1_{unit}_spanSum"),
        )
        .map(clean_text)
        .unwrap_or_default();
        let unit_total = regex!(r"([0-9.]+)")
            .and_then(|pattern| pattern.captures(&sum_text))
            .map_or(Value::Null, |found| js_number(group(&found, 1)));
        for row in grid_rows(*table) {
            let link = first(row, "a.kamokuGridKamokuName")
                .ok_or("A declared-course row is incomplete.")?;
            let offering_id = jwc(base, link.attr("href"));
            if !nine_digits(&offering_id) {
                return Err("A declared-course identifier is invalid.");
            }
            let range = text_at(row, "td.tdQ");
            let quarters: Vec<u32> = regex!(r"^([1-4])(?:\s*-\s*([1-4]))?$")
                .and_then(|pattern| pattern.captures(&range))
                .map(|found| {
                    let from = group(&found, 1).parse::<u32>().unwrap_or(0);
                    let to = match group(&found, 2) {
                        "" => from,
                        to => to.parse::<u32>().unwrap_or(0),
                    };
                    (from..=to).collect()
                })
                .unwrap_or_default();
            courses.push(json!({
                "offeringId": offering_id,
                "unit": unit,
                "unitTotal": unit_total,
                "code": text_at(row, "span.resultUnitKamokuCodeSP"),
                "title": clean_text(link),
                "quarters": quarters,
                "credits": credits(&text_at(row, "span.resultUnitKamokuTaniSP")),
            }));
        }
    }
    if courses.is_empty() && !tables.iter().any(|table| explicit_empty_element(*table)) {
        return Err("The declared-course list is incomplete.");
    }
    let empty = courses.is_empty();
    complete(
        "declared-course-tables",
        json!({ "courses": courses }),
        empty,
    )
}

fn grades(document: &Html) -> Read {
    let root = document.root_element();
    let mut groups: Vec<Value> = Vec::new();
    let mut category = String::new();
    let mut grouped_count = 0;
    let mut awaiting_group = false;
    for node in select(root, r#"h3[id$="_h3"], table[id$="_grid"]"#) {
        if is(node, "h3") {
            if awaiting_group {
                return Err("A grade group has no result table.");
            }
            category = clean_text(node);
            awaiting_group = true;
            continue;
        }
        let page = table_data(node, 0);
        if category.is_empty()
            || !awaiting_group
            || !page
                .headers
                .iter()
                .any(|header| header.contains("授業科目"))
            || !page.headers.iter().any(|header| header.contains("成績"))
        {
            return Err("A grouped grade table is incomplete.");
        }
        grouped_count += 1;
        awaiting_group = false;
        let mut rows = Vec::new();
        for row in grid_rows(node) {
            let title = text_at(row, "td.resultUnitKamokuName");
            if title.is_empty() {
                return Err("A grouped grade row is incomplete.");
            }
            let code = match text_at(row, "td.resultUnitSuisho + td.mediaPC") {
                code if code.is_empty() => text_at(row, "span.resultUnitKamokuCodeSP"),
                code => code,
            };
            let mut entry = json!({
                "title": title,
                "code": code,
                "recommended": text_at(row, "td.resultUnitSuisho"),
                "credits": text_at(row, "td.resultUnitTani"),
                "result": text_at(row, "td.resultUnitSeiseki"),
                "quarter": select(row, r#"[class*="tdQuarter"]"#)
                    .into_iter()
                    .map(clean_text)
                    .collect::<Vec<_>>()
                    .join(" "),
                "completed": text_at(row, "td.resultUnitYG span.mediaPC"),
            });
            if let Some(sort) = first(row, "td.resultUnitTani")
                .and_then(|cell| cell.attr("data-sort"))
                .filter(|sort| !sort.trim().is_empty())
            {
                entry["effectiveCredits"] = js_number(sort);
            }
            rows.push(entry);
        }
        if rows.is_empty() && !explicit_empty_element(node) {
            return Err("A grouped grade table has no explicit rows.");
        }
        groups.push(json!({ "category": category, "rows": rows }));
    }
    if awaiting_group {
        return Err("A grade group has no result table.");
    }
    if grouped_count > 0 {
        let empty = groups
            .iter()
            .all(|group| group["rows"].as_array().is_none_or(Vec::is_empty));
        return complete("undergraduate-grouped", json!({ "groups": groups }), empty);
    }

    let mut flat_groups: Vec<(&'static str, Value)> = Vec::new();
    for table in select(root, "table") {
        if first(table, "table").is_some() {
            continue;
        }
        let page = table_data(table, 0);
        let column = |label: &str| {
            page.headers
                .iter()
                .position(|header| header.contains(label))
        };
        let columns = [
            column("授業科目名"),
            column("授業担当教員"),
            column("単位"),
            column("成績"),
            column("修得時期"),
        ];
        let recognized: Vec<usize> = columns.iter().flatten().copied().collect();
        if recognized.is_empty() {
            continue;
        }
        let distinct: HashSet<_> = recognized.iter().collect();
        if recognized.len() != columns.len() || distinct.len() != columns.len() {
            return Err("A flat grade table is incomplete.");
        }
        let label = heading_before(document, table);
        let section = if label.contains("大学院科目") {
            "graduate"
        } else if label.contains("学部科目") {
            "undergraduate"
        } else {
            return Err("A flat grade table has no recognized section label.");
        };
        let value = |row: &[String], index: usize| {
            recognized
                .get(index)
                .and_then(|column| row.get(*column))
                .cloned()
                .unwrap_or_default()
        };
        let rows: Vec<Value> = page
            .rows
            .iter()
            .filter(|row| {
                row.iter().any(|cell| !cell.is_empty()) && !explicit_empty(&row.join(" "))
            })
            .map(|row| {
                json!({
                    "title": value(row, 0),
                    "instructor": value(row, 1),
                    "credits": value(row, 2),
                    "result": value(row, 3),
                    "completed": value(row, 4),
                })
            })
            .collect();
        if rows
            .iter()
            .any(|row| row["title"].as_str().is_none_or(str::is_empty))
        {
            return Err("A flat grade row has no course title.");
        }
        if rows.is_empty() && !explicit_empty_element(table) {
            return Err("A flat grade table has no explicit rows.");
        }
        flat_groups.push((
            section,
            json!({ "section": section, "category": label, "rows": rows }),
        ));
    }
    let sections: HashSet<&str> = flat_groups.iter().map(|(section, _)| *section).collect();
    if flat_groups.len() == 2 && sections.contains("graduate") && sections.contains("undergraduate")
    {
        let empty = flat_groups
            .iter()
            .all(|(_, group)| group["rows"].as_array().is_none_or(Vec::is_empty));
        let groups: Vec<Value> = flat_groups.into_iter().map(|(_, group)| group).collect();
        return complete("graduate-flat", json!({ "groups": groups }), empty);
    }
    if by_id(document, "ctl00_ContentPlaceHolder1").is_some_and(explicit_empty_element) {
        return complete("graduate-flat", json!({ "groups": [] }), true);
    }
    Err("The grade page shape is unknown.")
}

fn requirements(document: &Html) -> Read {
    let root = first(document.root_element(), "div.tableSet02Tbl")
        .ok_or("The requirement page shape is unknown.")?;
    let mut tables = Vec::new();
    for table in select(root, "table") {
        if !has_class(table, "tblYouken") {
            return Err("An unknown requirement table is present.");
        }
        let known = match thead_rows(table) {
            0 => corner_depth(&matrix(table)),
            rows => rows,
        };
        let page = table_data(table, known);
        if page.headers.len() < 2 {
            return Err("A requirement table has no semantic header.");
        }
        let rows: Vec<(String, Vec<String>)> = page
            .rows
            .iter()
            .filter(|row| {
                row.iter().any(|cell| !cell.is_empty()) && !explicit_empty(&row.join(" "))
            })
            .map(|row| {
                (
                    row.first().cloned().unwrap_or_default(),
                    row.iter().skip(1).cloned().collect(),
                )
            })
            .collect();
        if !rows.iter().any(|(label, _)| {
            regex!(r"(?:卒業|修了)要件|修得済単位").is_some_and(|pattern| pattern.is_match(label))
        }) {
            return Err("A requirement table has no recognized rows.");
        }
        if rows.iter().any(|(label, _)| label.is_empty()) {
            return Err("A requirement row has no label.");
        }
        let rows: Vec<Value> = rows
            .into_iter()
            .map(|(label, values)| json!({ "label": label, "values": values }))
            .collect();
        tables.push(json!({ "headers": page.headers, "rows": rows }));
    }
    if !tables.is_empty() {
        return complete("requirements-tables", json!({ "tables": tables }), false);
    }
    if explicit_empty_element(root) {
        return complete("requirements-tables", json!({ "tables": [] }), true);
    }
    Err("The requirement page shape is unknown.")
}

fn gpa(document: &Html) -> Read {
    let root = document.root_element();
    let mut candidates: Vec<Vec<Vec<String>>> = select(root, "table.tblGPASummary.mediaPC")
        .into_iter()
        .map(matrix)
        .collect();
    let overall_at = candidates.iter().position(|table| {
        let content = clean(
            &table
                .iter()
                .flatten()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        );
        content.contains("全ての科目") && content.contains("GPA") && content.contains("修得済単位")
    });
    let Some(overall_at) = overall_at else {
        if by_id(document, "ctl00_ContentPlaceHolder1").is_some_and(explicit_empty_element) {
            return complete(
                "gpa-summary",
                json!({ "categories": [], "overall": [], "terms": [] }),
                true,
            );
        }
        return Err("The GPA page shape is unknown.");
    };
    let summary = candidates.remove(overall_at);
    let mut overall = vec![summary];
    overall.extend(candidates);
    let term_table = first(root, "table.tblQ").ok_or("The GPA term table is missing.")?;
    let term_page = table_data(term_table, 0);
    if !term_page
        .headers
        .iter()
        .any(|header| header.contains("GPA"))
        || !term_page.headers.iter().any(|header| {
            regex!(r"修得時期|年度|学期").is_some_and(|pattern| pattern.is_match(header))
        })
    {
        return Err("The GPA term table is incomplete.");
    }
    let terms: Vec<Vec<String>> = term_page
        .rows
        .into_iter()
        .filter(|row| row.iter().any(|cell| !cell.is_empty()))
        .collect();
    if terms.is_empty() && !explicit_empty_element(term_table) {
        return Err("The GPA term list has no explicit rows.");
    }
    let categories: Vec<Value> = select(root, r#"h3[id$="_h3"]"#)
        .into_iter()
        .filter_map(|heading| {
            let id = heading.attr("id")?;
            let found = regex!(r"resultUnit_([0-9]+)_h3$")?.captures(id)?;
            Some(json!({ "code": group(&found, 1), "name": clean_text(heading) }))
        })
        .collect();
    complete(
        "gpa-summary",
        json!({ "categories": categories, "overall": overall, "terms": terms }),
        false,
    )
}
