//! Reader for the official course search (科目検索, `PID1_0_1.aspx`).
//!
//! The search page is an ASP.NET form. `net::kyomu` GETs it with the query
//! already in the URL, then posts the form back once to switch the result
//! grid to its largest page size; the server returns at most
//! [`MAX_SEARCH_ROWS`] rows per search, so one department fits one page.
//! Only the paging control is ever posted, never a registration control.

use super::html::{
    body, by_id, cells, clean, clean_text, closest, disabled, first, group, is, js_number, jwc,
    lines, maintenance_notice, nine_digits, select, table_rows, text_content,
};
use super::SchoolParseError;
use crate::course_query::OfficialFilterOption;
use scraper::{ElementRef, Html};
use serde_json::{json, Value};
use std::collections::HashSet;

/// The most rows the school returns for one search.
pub const MAX_SEARCH_ROWS: usize = 1000;
/// Rows beyond the largest page size the form offers mean the page is not
/// the search result this reader understands.
const MAX_PAGE_ROWS: usize = 2000;

pub enum SearchRead {
    Maintenance(String),
    /// The grid pages its rows: post these fields back once to show its
    /// largest page.
    Expand(Vec<(String, String)>),
    Results(SearchResults),
}

pub struct SearchResults {
    /// Rows in the course shape `DataStore::catalog_upsert_offerings` merges.
    pub courses: Vec<Value>,
    /// The public filter choices this account's form offers.
    pub options: Vec<OfficialFilterOption>,
    pub truncated: bool,
}

/// Reads one search response. `expand` allows answering with the paging
/// postback (the first, GET response); after the postback it is false and
/// the rows on the page are read as they are. `fetched_at` stamps each row.
pub fn read_search(
    html: &str,
    expand: bool,
    fetched_at: &str,
) -> Result<SearchRead, SchoolParseError> {
    let document = Html::parse_document(html);
    if let Some(notice) = maintenance_notice(&document) {
        return Ok(SearchRead::Maintenance(notice));
    }
    let grid = by_id(&document, "ctl00_ContentPlaceHolder1_gvGrid");
    let page_size = grid.and_then(|grid| first(grid, r#"select[id$="_drpPageSize"]"#));
    if expand {
        if let Some(size) = page_size {
            let form = closest(size, |element| is(element, "form"))
                .ok_or("The search paging controls changed.")?;
            let name = size.attr("name").unwrap_or("");
            let (current, values) = page_sizes(size);
            if name.is_empty() || largest(&values).is_none() {
                return Err("The search paging controls changed.".into());
            }
            let fields = form_data(&form_controls(form));
            if let Some(post) = expand_postback(fields, name, current.as_deref(), &values) {
                return Ok(SearchRead::Expand(post));
            }
        }
    }
    let options = public_options(&document);
    let Some(grid) = grid else {
        let empty = by_id(&document, "ctl00_ContentPlaceHolder1_cvKamoku");
        let hidden = empty
            .and_then(|empty| empty.attr("style"))
            .is_some_and(|style| {
                regex!(r"(?i)display:\s*none|visibility:\s*hidden")
                    .is_some_and(|pattern| pattern.is_match(style))
            });
        if empty.is_some_and(|empty| clean_text(empty).contains("指定した条件の科目はありません"))
            && !hidden
        {
            return Ok(SearchRead::Results(SearchResults {
                courses: Vec::new(),
                options,
                truncated: false,
            }));
        }
        return Err("The course search result is incomplete.".into());
    };
    let headers: Vec<String> = select(grid, "th").into_iter().map(clean_text).collect();
    let at = |name: &str| headers.iter().position(|header| header.starts_with(name));
    if [
        "Q",
        "曜日・時限",
        "科目コード",
        "授業科目名",
        "単位",
        "授業担当教員",
        "備考",
    ]
    .iter()
    .any(|name| at(name).is_none())
    {
        return Err("The course search columns changed.".into());
    }
    let page_text = clean(&body(&document).map(text_content).unwrap_or_default());
    let department = regex!(r"推奨\s*\[([^\]]+)\]")
        .and_then(|pattern| pattern.captures(&page_text))
        .and_then(|found| found.get(1).map(|value| value.as_str().to_string()))
        .unwrap_or_default();
    let rows: Vec<ElementRef<'_>> = table_rows(grid)
        .into_iter()
        .filter(|row| first(*row, r#"a[href*="jwc="]"#).is_some())
        .collect();
    if rows.len() > MAX_PAGE_ROWS {
        return Err("The course search returned more rows than one page holds.".into());
    }
    let base = url::Url::parse(&format!(
        "https://{}{}",
        super::HOST,
        crate::catalog::KYOMU_SEARCH_PATH
    ))
    .ok();
    let mut courses = Vec::with_capacity(rows.len());
    let mut seen = HashSet::new();
    for row in rows {
        let link = first(row, r#"a[href*="jwc="]"#).ok_or("A course search row is incomplete.")?;
        let id = jwc(base.as_ref(), link.attr("href"));
        let row_cells = cells(row);
        let cell = |name: &str| at(name).and_then(|index| row_cells.get(index)).copied();
        let value = |name: &str| cell(name).map(clean_text).unwrap_or_default();
        let course = course_row(CourseCells {
            id: &id,
            title: &clean_text(link),
            quarters: &value("Q"),
            slots: &lines(cell("曜日・時限")),
            code: &value("科目コード"),
            credits: &value("単位"),
            instructors: &value("授業担当教員"),
            language: &value("開講言語"),
            note: &value("備考"),
            category: &value("科目区分"),
            department: &department,
            fetched_at,
        })?;
        if cell("曜日・時限").is_none() || !seen.insert(id) {
            return Err("A course search row is incomplete.".into());
        }
        // The row must be a valid catalog offering, or the merge would store
        // a row the finder cannot read.
        serde_json::from_value::<crate::syllabus::model::Offering>(course.clone())
            .map_err(|_| SchoolParseError::from("kyomu course result is invalid"))?;
        courses.push(course);
    }
    if courses.is_empty()
        && !regex!(r"指定した条件の科目はありません|対象データはありません")
            .is_some_and(|pattern| pattern.is_match(&clean_text(grid)))
    {
        return Err("The empty course result is not explicit.".into());
    }
    let next_link = select(grid, "a")
        .into_iter()
        .any(|link| matches!(clean_text(link).as_str(), ">>" | "次へ" | "次のページ"));
    // A drop-down still short of its largest size means rows are on other
    // pages (the postback did not take effect).
    let shrunk = page_size.is_some_and(|size| {
        let (current, values) = page_sizes(size);
        current.as_deref().and_then(number) != largest(&values)
    });
    let truncated = is_truncated(courses.len(), next_link || shrunk);
    Ok(SearchRead::Results(SearchResults {
        courses,
        options,
        truncated,
    }))
}

/// Whether a search shows less than the school holds: it hit the school's
/// row cap, or the grid still offers another page.
pub(crate) fn is_truncated(rows: usize, more_pages: bool) -> bool {
    rows >= MAX_SEARCH_ROWS || more_pages
}

/// The page-size drop-down's current value and its enabled choices.
fn page_sizes(size: ElementRef<'_>) -> (Option<String>, Vec<String>) {
    let options = select_options(size);
    let current = chosen_options(&options, size.attr("multiple").is_some(), list_box(size))
        .first()
        .map(|option| option.value.clone());
    let values = options
        .into_iter()
        .filter(|option| !option.disabled)
        .map(|option| option.value)
        .collect();
    (current, values)
}

struct CourseCells<'a> {
    id: &'a str,
    title: &'a str,
    quarters: &'a str,
    slots: &'a [String],
    code: &'a str,
    credits: &'a str,
    instructors: &'a str,
    language: &'a str,
    note: &'a str,
    category: &'a str,
    department: &'a str,
    fetched_at: &'a str,
}

/// `1-2Q`, `1~2`, `1Q 3Q` → quarter labels.
fn quarters_of(raw: &str) -> Vec<String> {
    let normalized = clean(raw).replace('Q', "");
    if let Some(found) =
        regex!(r"^([1-4])[~〜～-]([1-4])$").and_then(|pattern| pattern.captures(&normalized))
    {
        let from = group(&found, 1).parse::<u8>().unwrap_or(0);
        let to = group(&found, 2).parse::<u8>().unwrap_or(0);
        return (from..=to).map(|quarter| format!("{quarter}Q")).collect();
    }
    let mut quarters = Vec::new();
    for quarter in normalized.chars().filter(|c| ('1'..='4').contains(c)) {
        let label = format!("{quarter}Q");
        if !quarters.contains(&label) {
            quarters.push(label);
        }
    }
    quarters
}

fn course_row(row: CourseCells<'_>) -> Result<Value, SchoolParseError> {
    let incomplete = || SchoolParseError::from("A course search row is incomplete.");
    let credits = regex!(r"^([0-9.]+)-([0-9.]+)-([0-9.]+)$")
        .and_then(|pattern| pattern.captures(row.credits))
        .ok_or_else(incomplete)?;
    let quarters = quarters_of(row.quarters);
    if !nine_digits(row.id) || quarters.is_empty() {
        return Err(incomplete());
    }
    let mut schedule = Vec::new();
    for raw in row.slots {
        let weekly = regex!(r"^([月火水木金土日])\s*([0-9]+)-([0-9]+)(?:\s*\((.*)\))?$")
            .and_then(|pattern| pattern.captures(raw));
        let Some(found) = weekly else {
            let kind = if raw.contains("申告専用") {
                "registration-only"
            } else if raw.contains("オンデマンド") {
                "on-demand"
            } else if raw.contains("集中") {
                "intensive"
            } else {
                "tba"
            };
            schedule.push(json!({ "kind": kind, "room": { "ja": "" } }));
            continue;
        };
        let day = "月火水木金土日"
            .chars()
            .position(|day| group(&found, 1).starts_with(day))
            .map_or(0, |index| index + 1);
        let (from, to) = (
            group(&found, 2).parse::<u64>().unwrap_or(u64::MAX),
            group(&found, 3).parse::<u64>().unwrap_or(u64::MAX),
        );
        if from < 1 || to > 12 || from > to {
            return Err("A course search period is invalid.".into());
        }
        schedule.push(json!({
            "kind": "weekly",
            "day": day,
            "periods": [from, to],
            "room": { "ja": group(&found, 4) },
        }));
    }
    let amount = |index: usize| group(&credits, index).parse::<f64>().unwrap_or(f64::NAN);
    let kinds: Vec<&str> = [(1, "lecture"), (2, "exercise"), (3, "experiment")]
        .into_iter()
        .filter(|(index, _)| amount(*index) > 0.0)
        .map(|(_, kind)| kind)
        .collect();
    let mark = regex!(r"^([A-Z])\s*\((◎|○|選択)\)").and_then(|pattern| pattern.captures(row.note));
    let language = if row.language.contains("英語") {
        "en"
    } else if row.language.contains("日本語") {
        "ja"
    } else {
        "other"
    };
    let instructors: Vec<Value> = row
        .instructors
        .split(',')
        .map(str::trim_start)
        .filter(|name| !name.is_empty())
        .map(|name| json!({ "ja": name }))
        .collect();
    let intensive = schedule.iter().any(|slot| slot["kind"] == "intensive");
    Ok(json!({
        "id": row.id,
        "year": row.id.get(..4).and_then(|year| year.parse::<u16>().ok()),
        "quarters": quarters,
        "code": row.code,
        "title": { "ja": row.title },
        "instructors": instructors,
        "credits": {
            "lecture": js_number(group(&credits, 1)),
            "exercise": js_number(group(&credits, 2)),
            "lab": js_number(group(&credits, 3)),
        },
        "instructionLanguage": language,
        "format": { "kinds": kinds },
        "media": {},
        "academicUnit": {},
        "intensive": intensive,
        "schedule": schedule,
        "syllabusUrl": format!("https://syllabus.s.isct.ac.jp/courses?jwc={}", row.id),
        "fetchedAt": {},
        "kyomu": {
            "department": row.department,
            "group": mark.as_ref().map_or("", |found| group(found, 1)),
            "mark": match mark.as_ref().map(|found| group(found, 2)) {
                Some("◎") => "required",
                Some("○") => "recommended",
                Some(_) => "elective",
                None => "",
            },
            "category": row.category,
            "prerequisite": row.note.contains("履修前提条件"),
            "fetchedAt": row.fetched_at,
        },
    }))
}

/// Semantic families of the official filter controls. The identities
/// describe meaning only; labels, choices and values always come from this
/// account's current form.
const FAMILIES: [(&str, &str); 11] = [
    ("kamokucodeseries", "level"),
    ("hisshukbn", "requirement"),
    ("youbi", "delivery"),
    ("kaisijigen", "start"),
    ("quarter", "quarter"),
    ("lang", "language"),
    ("gengo", "language"),
    ("kamokukbnmidashi", "category"),
    ("suisho", "recommended"),
    ("zengaku", "category"),
    ("senmon", "category"),
];

/// The public filter choices before the search button: checkboxes, radios
/// and select options outside the result grid, each with its visible label.
pub(crate) fn public_options(document: &Html) -> Vec<OfficialFilterOption> {
    let root = document.root_element();
    let search = first(root, r#"[id$="btnSearch"]"#);
    let labels = select(root, "label[for]");
    let mut options = Vec::new();
    for node in root.descendants() {
        let Some(input) = ElementRef::wrap(node) else {
            continue;
        };
        // Controls after the search button belong to the results.
        if Some(input) == search {
            break;
        }
        let kind = input.attr("type").unwrap_or("").to_ascii_lowercase();
        let choice =
            is(input, "select") || (is(input, "input") && (kind == "checkbox" || kind == "radio"));
        let Some(id) = input.attr("id").filter(|id| !id.is_empty()) else {
            continue;
        };
        if !choice
            || disabled(input)
            || closest(input, |element| {
                element.attr("id").is_some_and(|id| id.contains("gvGrid"))
            })
            .is_some()
        {
            continue;
        }
        let container = closest(input, |element| {
            is(element, "fieldset")
                || element.attr("role") == Some("group")
                || ((is(element, "table") || is(element, "div")) && element.attr("id").is_some())
        });
        let identity = format!(
            "{id} {} {}",
            input.attr("name").unwrap_or(""),
            container
                .and_then(|container| container.attr("id"))
                .unwrap_or("")
        )
        .to_lowercase();
        let family = FAMILIES
            .iter()
            .find(|(key, _)| identity.contains(key))
            .map_or("other", |(_, family)| *family);
        if is(input, "select") {
            for option in select_options(input) {
                if !option.value.is_empty() && !option.disabled {
                    options.push(OfficialFilterOption {
                        id: id.into(),
                        value: option.value,
                        label: clean(&option.text),
                        family: family.into(),
                    });
                }
            }
            continue;
        }
        let label = labels
            .iter()
            .find(|label| label.attr("for") == Some(id))
            .map(|label| text_content(*label))
            .filter(|text| !text.is_empty())
            .or_else(|| closest(input, |element| is(element, "label")).map(text_content))
            .map(|text| clean(&text))
            .unwrap_or_default();
        if !label.is_empty() {
            options.push(OfficialFilterOption {
                id: id.into(),
                value: input.attr("value").unwrap_or("on").into(),
                label,
                family: family.into(),
            });
        }
    }
    options
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ControlTag {
    Input,
    Select,
    Textarea,
    Button,
}

/// One form-associated element as the browser sees it when it builds the
/// form data set.
#[derive(Clone, Debug)]
pub(crate) struct FormControl {
    pub tag: ControlTag,
    /// Lowercased `type` of an input.
    pub kind: String,
    pub name: String,
    pub value: Option<String>,
    pub checked: bool,
    pub disabled: bool,
    pub multiple: bool,
    /// A `multiple` or `size > 1` select, which has no default choice.
    pub list_box: bool,
    pub options: Vec<SelectOption>,
}

#[derive(Clone, Debug)]
pub(crate) struct SelectOption {
    pub value: String,
    pub text: String,
    pub selected: bool,
    pub disabled: bool,
}

fn select_options(element: ElementRef<'_>) -> Vec<SelectOption> {
    let mut options = Vec::new();
    for child in element.child_elements() {
        let (group_disabled, items) = if is(child, "optgroup") {
            (
                child.attr("disabled").is_some(),
                child
                    .child_elements()
                    .filter(|item| is(*item, "option"))
                    .collect(),
            )
        } else if is(child, "option") {
            (false, vec![child])
        } else {
            (false, Vec::new())
        };
        for item in items {
            let text = text_content(item);
            options.push(SelectOption {
                // `option.value` falls back to its text with ASCII whitespace
                // stripped and collapsed.
                value: item.attr("value").map_or_else(
                    || text.split_ascii_whitespace().collect::<Vec<_>>().join(" "),
                    str::to_string,
                ),
                text,
                selected: item.attr("selected").is_some(),
                disabled: group_disabled || item.attr("disabled").is_some(),
            });
        }
    }
    options
}

fn list_box(element: ElementRef<'_>) -> bool {
    element.attr("multiple").is_some()
        || element
            .attr("size")
            .and_then(|size| size.trim().parse::<u32>().ok())
            .is_some_and(|size| size > 1)
}

/// The options a select submits. A multiple select submits every selected
/// option; a single select only its last `selected` one, as the parser keeps
/// it, and a drop-down with none selected its first enabled option.
pub(crate) fn chosen_options(
    options: &[SelectOption],
    multiple: bool,
    list_box: bool,
) -> Vec<&SelectOption> {
    let selected: Vec<&SelectOption> = options.iter().filter(|option| option.selected).collect();
    if multiple {
        return selected;
    }
    match selected.last() {
        Some(option) => vec![*option],
        None if !list_box => options
            .iter()
            .find(|option| !option.disabled)
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

/// The controls of `form`, in tree order.
pub(crate) fn form_controls(form: ElementRef<'_>) -> Vec<FormControl> {
    form.descendent_elements()
        .filter_map(|element| {
            let tag = match element.value().name() {
                "input" => ControlTag::Input,
                "select" => ControlTag::Select,
                "textarea" => ControlTag::Textarea,
                "button" => ControlTag::Button,
                _ => return None,
            };
            Some(FormControl {
                kind: match tag {
                    ControlTag::Input => {
                        element.attr("type").unwrap_or("text").to_ascii_lowercase()
                    }
                    _ => String::new(),
                },
                name: element.attr("name").unwrap_or("").into(),
                value: if tag == ControlTag::Textarea {
                    Some(text_content(element))
                } else {
                    element.attr("value").map(str::to_string)
                },
                checked: element.attr("checked").is_some(),
                disabled: disabled(element),
                multiple: element.attr("multiple").is_some(),
                list_box: tag == ControlTag::Select && list_box(element),
                options: if tag == ControlTag::Select {
                    select_options(element)
                } else {
                    Vec::new()
                },
                tag,
            })
        })
        .collect()
}

/// The form data set `new FormData(form)` builds without a submitter:
/// enabled, named controls; checked boxes and radios; chosen options; no
/// buttons, submit or image inputs, and no files.
pub(crate) fn form_data(controls: &[FormControl]) -> Vec<(String, String)> {
    // Checking a radio unchecks the others of its group, so only the last
    // checked radio of a name is submitted.
    let mut last_radio = std::collections::HashMap::new();
    for (index, control) in controls.iter().enumerate() {
        if control.tag == ControlTag::Input && control.kind == "radio" && control.checked {
            last_radio.insert(control.name.as_str(), index);
        }
    }
    let mut fields = Vec::new();
    for (index, control) in controls.iter().enumerate() {
        if control.disabled || control.name.is_empty() {
            continue;
        }
        let name = control.name.clone();
        match control.tag {
            ControlTag::Button => {}
            ControlTag::Input => match control.kind.as_str() {
                "submit" | "image" | "button" | "reset" | "file" => {}
                "radio" if last_radio.get(control.name.as_str()) != Some(&index) => {}
                "checkbox" | "radio" => {
                    if control.checked {
                        fields.push((name, control.value.clone().unwrap_or_else(|| "on".into())));
                    }
                }
                "hidden" if name.eq_ignore_ascii_case("_charset_") && control.value.is_none() => {
                    fields.push((name, "UTF-8".into()));
                }
                _ => fields.push((name, control.value.clone().unwrap_or_default())),
            },
            ControlTag::Select => {
                for option in chosen_options(&control.options, control.multiple, control.list_box) {
                    if !option.disabled {
                        fields.push((name.clone(), option.value.clone()));
                    }
                }
            }
            ControlTag::Textarea => fields.push((name, control.value.clone().unwrap_or_default())),
        }
    }
    fields
}

/// `URLSearchParams.set`: replace the first field of `name` and drop the
/// rest, or append it.
pub(crate) fn set_field(fields: &mut Vec<(String, String)>, name: &str, value: &str) {
    let mut found = false;
    fields.retain_mut(|(field, current)| {
        if field != name {
            return true;
        }
        if found {
            return false;
        }
        found = true;
        *current = value.into();
        true
    });
    if !found {
        fields.push((name.into(), value.into()));
    }
}

fn number(value: &str) -> Option<u32> {
    value.trim().parse().ok()
}

fn largest(values: &[String]) -> Option<u32> {
    values.iter().filter_map(|value| number(value)).max()
}

/// The postback that switches the grid's page-size drop-down (`name`) to its
/// largest choice, as its AutoPostBack `onchange` would; `None` when the
/// grid already shows its largest page or offers no numeric size.
pub(crate) fn expand_postback(
    mut fields: Vec<(String, String)>,
    name: &str,
    current: Option<&str>,
    choices: &[String],
) -> Option<Vec<(String, String)>> {
    let target = largest(choices)?;
    if current.and_then(number) == Some(target) {
        return None;
    }
    set_field(&mut fields, name, &target.to_string());
    set_field(&mut fields, "__EVENTTARGET", name);
    set_field(&mut fields, "__EVENTARGUMENT", "");
    Some(fields)
}

#[cfg(test)]
#[path = "../../tests/school_parser__kyomu__search.rs"]
mod tests;
