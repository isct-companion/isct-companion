//! Teacher HTML from LMS as a bounded, inert subset every shell can render:
//! the rules of the desktop's `safeContent` (`features/lms/content.ts`), so
//! desktop and mobile show the same text. No scripts, forms or styles; links
//! only over https; images become links to themselves. Also what Core reads
//! from that HTML for a lesson: its one-line summary, whether a label is only
//! pictures, and where the course is in a lesson schedule table.
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use ego_tree::{NodeId, NodeRef};
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use unicode_normalization::UnicodeNormalization;

/// Longest source read, as on desktop.
const MAX_SOURCE: usize = 100_000;

/// Elements kept as they are; any other element gives way to its children.
const KEPT: [&str; 30] = [
    "article",
    "time",
    "p",
    "div",
    "span",
    "br",
    "b",
    "strong",
    "i",
    "em",
    "u",
    "s",
    "ul",
    "ol",
    "li",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "pre",
    "code",
    "blockquote",
    "h2",
    "h3",
    "h4",
    "h5",
    "hr",
    "a",
];

/// Elements dropped with everything inside them.
const DROPPED: &str =
    "script, style, iframe, object, embed, form, input, button, textarea, select, svg";

/// Moodle's own date lines on an activity, which the app states itself.
const ACTIVITY_DATES: &str =
    r#"[data-region="activity-dates"], .activity-dates, [data-region="activity-information"]"#;

const VOID: [&str; 2] = ["br", "hr"];

/// The first `max` bytes of `html`, cut at a character boundary.
fn head(html: &str, max: usize) -> &str {
    let mut end = html.len().min(max);
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    &html[..end]
}

pub(crate) fn safe_html(html: &str, without_activity_dates: bool) -> String {
    let source = head(html, MAX_SOURCE);
    let page = Html::parse_fragment(source);
    let mut skip: HashSet<NodeId> = HashSet::new();
    let selectors = [
        Some(DROPPED),
        without_activity_dates.then_some(ACTIVITY_DATES),
    ];
    for raw in selectors.into_iter().flatten() {
        if let Ok(selector) = Selector::parse(raw) {
            skip.extend(page.select(&selector).map(|element| element.id()));
        }
    }
    let mut out = String::with_capacity(source.len());
    for child in page.root_element().children() {
        write(child, &skip, &HashMap::new(), &mut out);
    }
    out.trim().to_owned()
}

/// Safe HTML with the rows of its lesson schedule tables marked against the
/// lesson the course is at (`current`): earlier rows `lmsRowPast`, its row
/// `lmsRowNow`, cancelled rows `lmsRowOff`. A table qualifies when at least
/// two of its rows start a cell with a lesson number (1回目, 第2回, 3).
pub(crate) fn mark_schedule(html: &str, current: Option<u32>) -> String {
    let Some(current) = current.filter(|_| html.contains("<table")) else {
        return html.to_owned();
    };
    let page = Html::parse_fragment(html);
    let marks = schedule_marks(&page, current);
    if marks.is_empty() {
        return html.to_owned();
    }
    let mut out = String::with_capacity(html.len() + 32 * marks.len());
    for child in page.root_element().children() {
        write(child, &HashSet::new(), &marks, &mut out);
    }
    out.trim().to_owned()
}

fn schedule_marks(page: &Html, current: u32) -> HashMap<NodeId, String> {
    static NUMBER: LazyLock<Result<Regex, regex::Error>> =
        LazyLock::new(|| Regex::new(r"^(?:第\s*)?(\d{1,2})\s*(?:回目?|$)"));
    static CANCELLED: LazyLock<Result<Regex, regex::Error>> =
        LazyLock::new(|| Regex::new(r"(?i)休講|cancell?ed|no class"));
    let mut marks = HashMap::new();
    let (Ok(number), Ok(cancelled), Ok(tables), Ok(row_selector)) = (
        NUMBER.as_ref(),
        CANCELLED.as_ref(),
        Selector::parse("table"),
        Selector::parse("tr"),
    ) else {
        return marks;
    };
    let row_number = |row: &ElementRef| {
        row.children()
            .filter_map(ElementRef::wrap)
            .filter(|cell| matches!(cell.value().name(), "td" | "th"))
            .find_map(|cell| {
                let text: String = cell.text().collect::<String>().nfkc().collect();
                number
                    .captures(text.trim())?
                    .get(1)?
                    .as_str()
                    .parse::<u32>()
                    .ok()
            })
    };
    for table in page.select(&tables) {
        let rows: Vec<ElementRef> = table.select(&row_selector).collect();
        let numbered: Vec<Option<u32>> = rows.iter().map(&row_number).collect();
        if numbered.iter().flatten().count() < 2 {
            continue;
        }
        let now = numbered.iter().position(|n| *n == Some(current));
        for (index, (row, number)) in rows.iter().zip(&numbered).enumerate() {
            let off = cancelled.is_match(&row.text().collect::<String>());
            let before = match now {
                Some(now) => index < now,
                None => number.is_some_and(|n| n < current),
            };
            let mut classes = Vec::new();
            if off {
                classes.push("lmsRowOff");
            }
            if *number == Some(current) {
                classes.push("lmsRowNow");
            } else if before && (number.is_some() || off) {
                classes.push("lmsRowPast");
            }
            if !classes.is_empty() {
                marks.insert(row.id(), classes.join(" "));
            }
        }
    }
    marks
}

/// A folded lesson's one-line summary: the first line of the section text,
/// without a short leading label such as "授業計画：" or "Course schedule:".
pub(crate) fn lesson_summary(html: &str) -> String {
    static LABEL: LazyLock<Result<Regex, regex::Error>> =
        LazyLock::new(|| Regex::new(r"^[^：:]{1,16}[：:]\s*"));
    const BLOCKS: [&str; 11] = [
        "p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "br",
    ];
    fn lines(node: NodeRef<'_, Node>, out: &mut String) {
        match node.value() {
            Node::Text(text) => out.push_str(text),
            Node::Element(element) if matches!(element.name(), "script" | "style") => {}
            Node::Element(element) => {
                for child in node.children() {
                    lines(child, out);
                }
                if BLOCKS.contains(&element.name()) {
                    out.push('\n');
                }
            }
            _ => {
                for child in node.children() {
                    lines(child, out);
                }
            }
        }
    }
    let page = Html::parse_fragment(head(html, 20_000));
    let mut text = String::new();
    lines(*page.root_element(), &mut text);
    let line = text
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    match LABEL.as_ref() {
        Ok(label) => label.replace(&line, "").into_owned(),
        Err(_) => line,
    }
}

/// Whether a label shows only LMS pictures, which shells lay out side by
/// side instead of one block each.
pub(crate) fn image_only(html: &str) -> bool {
    let page = Html::parse_fragment(head(html, MAX_SOURCE));
    let Ok(images) = Selector::parse("img") else {
        return false;
    };
    page.select(&images).any(|image| {
        image
            .attr("src")
            .is_some_and(|src| src.contains("/pluginfile.php/"))
    }) && page
        .root_element()
        .text()
        .all(|text| text.trim().is_empty())
}

/// An absolute https URL, as desktop's `safeUrl` accepts.
fn safe_url(raw: Option<&str>) -> Option<url::Url> {
    url::Url::parse(raw?)
        .ok()
        .filter(|url| url.scheme() == "https")
}

/// Moodle's expand/collapse controls inside cached section markup.
fn collapse_control(url: &url::Url, raw: &str) -> bool {
    url.host_str() == Some("lms.s.isct.ac.jp")
        && url.path().ends_with("/course/view.php")
        && (raw.ends_with('#')
            || url
                .fragment()
                .is_some_and(|hash| hash.starts_with("coursecontentcollapse")))
}

fn write(
    node: NodeRef<'_, Node>,
    skip: &HashSet<NodeId>,
    marks: &HashMap<NodeId, String>,
    out: &mut String,
) {
    match node.value() {
        Node::Text(text) => escape(text, false, out),
        Node::Element(element) => {
            if skip.contains(&node.id()) {
                return;
            }
            let name = element.name();
            if name == "img" {
                if let Some(url) = safe_url(element.attr("src"))
                    .filter(|url| !url.path().contains("/theme/image.php/"))
                {
                    out.push_str("<a href=\"");
                    escape(url.as_str(), true, out);
                    out.push_str("\">");
                    escape(
                        element
                            .attr("alt")
                            .filter(|alt| !alt.is_empty())
                            .unwrap_or("画像"),
                        false,
                        out,
                    );
                    out.push_str("</a>");
                }
                return;
            }
            if !KEPT.contains(&name) {
                for child in node.children() {
                    write(child, skip, marks, out);
                }
                return;
            }
            out.push('<');
            out.push_str(name);
            if name == "a" {
                let raw = element.attr("href").unwrap_or_default();
                if let Some(url) = safe_url(Some(raw)) {
                    if collapse_control(&url, raw) {
                        out.truncate(out.len() - 2);
                        return;
                    }
                    out.push_str(" href=\"");
                    escape(url.as_str(), true, out);
                    out.push('"');
                }
            }
            // Shells filter bilingual content by its language.
            if let Some(lang) = element
                .attr("lang")
                .map(str::to_ascii_lowercase)
                .filter(|lang| matches!(lang.as_str(), "ja" | "en"))
            {
                out.push_str(" lang=\"");
                out.push_str(&lang);
                out.push('"');
            }
            if let Some(classes) = marks.get(&node.id()) {
                out.push_str(" class=\"");
                out.push_str(classes);
                out.push('"');
            }
            for attribute in ["colspan", "rowspan"] {
                if let Some(value) = element.attr(attribute).filter(|value| {
                    (1..=2).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit())
                }) {
                    out.push(' ');
                    out.push_str(attribute);
                    out.push_str("=\"");
                    out.push_str(value);
                    out.push('"');
                }
            }
            out.push('>');
            if VOID.contains(&name) {
                return;
            }
            for child in node.children() {
                write(child, skip, marks, out);
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
        Node::Comment(_) | Node::ProcessingInstruction(_) | Node::Doctype(_) => {}
        _ => {
            for child in node.children() {
                write(child, skip, marks, out);
            }
        }
    }
}

fn escape(raw: &str, attribute: bool, out: &mut String) {
    for character in raw.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
#[path = "../tests/lms_html.rs"]
mod tests;
