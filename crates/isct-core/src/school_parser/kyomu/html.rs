//! DOM helpers shared by the 教務Web page and course-search readers.
//!
//! They reproduce the browser semantics the former injected readers relied on
//! (`textContent`, `HTMLTableElement.rows`, rowspan/colspan, form controls)
//! over `scraper`, so the Rust readers see the same text the scripts saw.

use scraper::{node::Node, ElementRef, Html, Selector};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

/// `String(value).normalize("NFKC").replace(/\s+/g, " ").trim()`.
pub(crate) fn clean(value: &str) -> String {
    let normalized: String = value.nfkc().collect();
    normalized
        .split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A capture group's text, "" when the group did not take part.
pub(crate) fn group<'t>(captures: &regex::Captures<'t>, index: usize) -> &'t str {
    captures.get(index).map_or("", |found| found.as_str())
}

/// `element.textContent`.
pub(crate) fn text_content(element: ElementRef<'_>) -> String {
    element.text().collect()
}

pub(crate) fn clean_text(element: ElementRef<'_>) -> String {
    clean(&text_content(element))
}

/// `root.querySelectorAll(css)`; the selectors are constants, so a selector
/// that fails to parse reads as "nothing found" rather than panicking.
pub(crate) fn select<'a>(root: ElementRef<'a>, css: &str) -> Vec<ElementRef<'a>> {
    Selector::parse(css)
        .map(|selector| root.select(&selector).collect())
        .unwrap_or_default()
}

/// `root.querySelector(css)`.
pub(crate) fn first<'a>(root: ElementRef<'a>, css: &str) -> Option<ElementRef<'a>> {
    let selector = Selector::parse(css).ok()?;
    let found = root.select(&selector).next();
    found
}

/// `clean(root.querySelector(css)?.textContent)`.
pub(crate) fn text_at(root: ElementRef<'_>, css: &str) -> String {
    first(root, css).map(clean_text).unwrap_or_default()
}

/// `document.getElementById(id)`.
pub(crate) fn by_id<'a>(document: &'a Html, id: &str) -> Option<ElementRef<'a>> {
    document
        .root_element()
        .descendent_elements()
        .find(|element| element.value().id() == Some(id))
}

pub(crate) fn body(document: &Html) -> Option<ElementRef<'_>> {
    document
        .root_element()
        .child_elements()
        .find(|element| is(*element, "body"))
}

pub(crate) fn is(element: ElementRef<'_>, name: &str) -> bool {
    element.value().name() == name
}

pub(crate) fn has_class(element: ElementRef<'_>, class: &str) -> bool {
    element.value().classes().any(|name| name == class)
}

/// `element.closest(...)`: the element itself or its nearest matching ancestor.
pub(crate) fn closest<'a>(
    element: ElementRef<'a>,
    test: impl Fn(ElementRef<'a>) -> bool,
) -> Option<ElementRef<'a>> {
    std::iter::once(element)
        .chain(element.ancestors().filter_map(ElementRef::wrap))
        .find(|candidate| test(*candidate))
}

pub(crate) fn next_element_sibling(element: ElementRef<'_>) -> Option<ElementRef<'_>> {
    element.next_siblings().find_map(ElementRef::wrap)
}

/// `/対象データはありません/` over the cleaned text: the page says in words that
/// a list is empty, which is the only thing that proves an empty result.
pub(crate) fn explicit_empty(text: &str) -> bool {
    clean(text).contains("対象データはありません")
}

pub(crate) fn explicit_empty_element(element: ElementRef<'_>) -> bool {
    explicit_empty(&text_content(element))
}

/// Text split on `<br>` and line breaks, each line cleaned, empty ones dropped.
pub(crate) fn lines(element: Option<ElementRef<'_>>) -> Vec<String> {
    let Some(element) = element else {
        return Vec::new();
    };
    let mut raw = String::new();
    for node in element.descendants() {
        match node.value() {
            Node::Text(text) => raw.push_str(text),
            Node::Element(child) if child.name() == "br" => raw.push('\n'),
            _ => {}
        }
    }
    raw.split('\n')
        .map(clean)
        .filter(|line| !line.is_empty())
        .collect()
}

const HIDDEN_ELEMENTS: [&str; 7] = [
    "script", "style", "noscript", "template", "textarea", "head", "title",
];
const BLOCK_ELEMENTS: [&str; 30] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "caption",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "section",
    "table",
    "td",
    "th",
    "tr",
];

/// An approximation of `innerText`: no script, style or form-control text,
/// and block boundaries separate words the way rendered lines do.
pub(crate) fn rendered_text(element: ElementRef<'_>) -> String {
    let mut out = String::new();
    let mut stack: Vec<_> = element.children().rev().map(|node| (node, false)).collect();
    while let Some((node, closing)) = stack.pop() {
        if closing {
            out.push(' ');
            continue;
        }
        match node.value() {
            Node::Text(text) => out.push_str(text),
            Node::Element(child) => {
                let name = child.name();
                if HIDDEN_ELEMENTS.contains(&name) {
                    continue;
                }
                if BLOCK_ELEMENTS.contains(&name) {
                    out.push(' ');
                    stack.push((node, true));
                }
                stack.extend(node.children().rev().map(|node| (node, false)));
            }
            _ => {}
        }
    }
    out
}

/// The current outage notice, or `None` when the page is not a maintenance
/// page. Only a heading saying maintenance is happening *now* counts: a
/// future notice on a working page, or a course mentioning maintenance, must
/// not stop the service. No calendar is inferred; the school's text is kept.
pub(crate) fn maintenance_notice(document: &Html) -> Option<String> {
    let root = document.root_element();
    let pattern = regex!(
        r"(?i)(?:ただいま|現在).*(?:メンテナンス中)|under maintenance|maintenance in progress"
    )?;
    let heading = select(root, "h1,h2,h3,h4,[role=heading],title,strong,b,p,span")
        .into_iter()
        .find(|node| pattern.is_match(&clean_text(*node)))?;
    let region = closest(heading, |element| {
        is(element, "main") || is(element, "article") || element.attr("role") == Some("main")
    })
    .or_else(|| body(document))
    .unwrap_or(root);
    let details = clean(&rendered_text(region));
    let mut parts = Vec::new();
    if !details.is_empty() {
        parts.push(details.clone());
    }
    for note in select(region, "textarea").into_iter().map(clean_text) {
        if !note.is_empty() && !details.contains(&note) {
            parts.push(note);
        }
    }
    let notice: String = parts.join(" ").chars().take(1800).collect();
    (!notice.is_empty()).then_some(notice)
}

/// `HTMLTableElement.rows`: head rows, then body and direct rows in tree
/// order, then foot rows. Rows of nested tables are not included.
pub(crate) fn table_rows(table: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    fn section_rows(section: ElementRef<'_>) -> Vec<ElementRef<'_>> {
        section
            .child_elements()
            .filter(|row| is(*row, "tr"))
            .collect()
    }
    let (mut head, mut rows, mut foot) = (Vec::new(), Vec::new(), Vec::new());
    for child in table.child_elements() {
        match child.value().name() {
            "tr" => rows.push(child),
            "thead" => head.extend(section_rows(child)),
            "tbody" => rows.extend(section_rows(child)),
            "tfoot" => foot.extend(section_rows(child)),
            _ => {}
        }
    }
    head.extend(rows);
    head.extend(foot);
    head
}

/// `table.tHead?.rows.length || 0`.
pub(crate) fn thead_rows(table: ElementRef<'_>) -> usize {
    table
        .child_elements()
        .find(|child| is(*child, "thead"))
        .map_or(0, |head| {
            head.child_elements().filter(|row| is(*row, "tr")).count()
        })
}

/// `row.cells`.
pub(crate) fn cells(row: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    row.child_elements()
        .filter(|cell| is(*cell, "td") || is(*cell, "th"))
        .collect()
}

/// HTML's rules for parsing a non-negative span, clamped like the DOM does.
fn span(value: Option<&str>, max: usize) -> usize {
    let digits: String = value
        .unwrap_or("")
        .trim_start()
        .trim_start_matches('+')
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return 1;
    }
    digits
        .parse::<usize>()
        .map_or(max, |span| span.clamp(1, max))
}

/// A table expanded to a rectangular text matrix. Values covered by rowspan
/// or colspan are repeated so header lookup works for multi-row heads. A
/// rowspan never extends past the table's last row.
pub(crate) fn matrix(table: ElementRef<'_>) -> Vec<Vec<String>> {
    let rows = table_rows(table);
    let mut output: Vec<Vec<Option<String>>> = vec![Vec::new(); rows.len()];
    for (row_index, row) in rows.iter().enumerate() {
        let mut column = 0;
        for cell in cells(*row) {
            while output[row_index].get(column).is_some_and(Option::is_some) {
                column += 1;
            }
            let value = clean_text(cell);
            let down = span(cell.attr("rowspan"), 65_534).min(rows.len() - row_index);
            let across = span(cell.attr("colspan"), 1_000);
            for line in &mut output[row_index..row_index + down] {
                if line.len() < column + across {
                    line.resize(column + across, None);
                }
                for slot in &mut line[column..column + across] {
                    *slot = Some(value.clone());
                }
            }
            column += across;
        }
    }
    let width = output.iter().map(Vec::len).max().unwrap_or(0);
    output
        .into_iter()
        .map(|row| {
            (0..width)
                .map(|index| row.get(index).cloned().flatten().unwrap_or_default())
                .collect()
        })
        .collect()
}

pub(crate) struct TableData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// `tableData(table, knownHeaderRows)`: the semantic header of each column
/// and the data rows below the header.
pub(crate) fn table_data(table: ElementRef<'_>, known_header_rows: usize) -> TableData {
    let first_data_row = table_rows(table)
        .iter()
        .position(|row| first(*row, "td").is_some());
    let depth = header_depth(known_header_rows, thead_rows(table), first_data_row);
    split_header(matrix(table), depth)
}

/// `knownHeaderRows || tHead?.rows.length || (firstDataRow > 0 ? firstDataRow : 1)`.
pub(crate) fn header_depth(known: usize, thead: usize, first_data_row: Option<usize>) -> usize {
    if known > 0 {
        known
    } else if thead > 0 {
        thead
    } else {
        first_data_row.filter(|row| *row > 0).unwrap_or(1)
    }
}

/// Each column's header is the distinct non-empty values of the header rows
/// above it, joined with " / ".
pub(crate) fn split_header(mut grid: Vec<Vec<String>>, depth: usize) -> TableData {
    let width = grid.iter().map(Vec::len).max().unwrap_or(0);
    let rows = grid.split_off(depth.min(grid.len()));
    let headers = (0..width)
        .map(|column| {
            let mut parts: Vec<&str> = Vec::new();
            for value in grid.iter().filter_map(|row| row.get(column)) {
                if !value.is_empty() && !parts.contains(&value.as_str()) {
                    parts.push(value);
                }
            }
            parts.join(" / ")
        })
        .collect();
    TableData { headers, rows }
}

/// The live requirement tables have no `<thead>`, and their first row
/// already contains a `<td>`; the header is as deep as the top-left corner
/// cell spans. Zero when the corner does not span (no evidence of depth).
pub(crate) fn corner_depth(grid: &[Vec<String>]) -> usize {
    let corner = grid.first().and_then(|row| row.first());
    grid.iter()
        .position(|row| row.first() != corner)
        .unwrap_or(0)
}

/// The caption, else the text of the last `h1`–`h5` before the table.
pub(crate) fn heading_before(document: &Html, table: ElementRef<'_>) -> String {
    if let Some(caption) = table.child_elements().find(|child| is(*child, "caption")) {
        let caption = clean_text(caption);
        if !caption.is_empty() {
            return caption;
        }
    }
    let mut last = None;
    for node in document.root_element().descendants() {
        if node.id() == table.id() {
            break;
        }
        if let Some(element) = ElementRef::wrap(node) {
            if matches!(element.value().name(), "h1" | "h2" | "h3" | "h4" | "h5") {
                last = Some(element);
            }
        }
    }
    last.map(clean_text).unwrap_or_default()
}

/// `Number(raw)` as JSON: an integer when integral, `null` when not a number
/// (JSON.stringify(NaN) is null), so stored rows keep the shape they had.
pub(crate) fn js_number(raw: &str) -> Value {
    let trimmed = raw.trim();
    let parsed = if trimmed.is_empty() {
        Some(0.0)
    } else {
        trimmed.parse::<f64>().ok()
    };
    match parsed.filter(|number| number.is_finite()) {
        Some(number) if number.fract() == 0.0 && number.abs() < 9.0e15 => {
            Value::from(number as i64)
        }
        Some(number) => serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number),
        None => Value::Null,
    }
}

/// The `jwc` query value of a link resolved against the page URL, or "".
pub(crate) fn jwc(base: Option<&url::Url>, href: Option<&str>) -> String {
    base.and_then(|base| base.join(href.unwrap_or("")).ok())
        .and_then(|url| {
            url.query_pairs()
                .find(|(key, _)| key == "jwc")
                .map(|(_, value)| value.into_owned())
        })
        .unwrap_or_default()
}

pub(crate) fn nine_digits(value: &str) -> bool {
    value.len() == 9 && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether a control is disabled, directly or by a disabled fieldset it is
/// not inside the first legend of (`:disabled`).
pub(crate) fn disabled(element: ElementRef<'_>) -> bool {
    if element.attr("disabled").is_some() {
        return true;
    }
    element
        .ancestors()
        .filter_map(ElementRef::wrap)
        .filter(|ancestor| is(*ancestor, "fieldset") && ancestor.attr("disabled").is_some())
        .any(|fieldset| {
            let legend = fieldset.child_elements().find(|child| is(*child, "legend"));
            !legend.is_some_and(|legend| element.ancestors().any(|node| node.id() == legend.id()))
        })
}

#[cfg(test)]
#[path = "../../tests/school_parser__kyomu__html.rs"]
mod tests;
