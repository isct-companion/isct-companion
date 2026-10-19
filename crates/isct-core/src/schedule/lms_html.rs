//! Teacher HTML from LMS as a bounded, inert subset every shell can render:
//! the rules of the desktop's `safeContent` (`features/lms/content.ts`), so
//! desktop and mobile show the same text. No scripts, forms or styles; links
//! only over https; images become links to themselves.
use std::collections::HashSet;

use ego_tree::{NodeId, NodeRef};
use scraper::{Html, Node, Selector};

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

pub(crate) fn safe_html(html: &str, without_activity_dates: bool) -> String {
    let mut end = html.len().min(MAX_SOURCE);
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    let page = Html::parse_fragment(&html[..end]);
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
    let mut out = String::with_capacity(end);
    for child in page.root_element().children() {
        write(child, &skip, &mut out);
    }
    out.trim().to_owned()
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

fn write(node: NodeRef<'_, Node>, skip: &HashSet<NodeId>, out: &mut String) {
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
                    write(child, skip, out);
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
                write(child, skip, out);
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
        Node::Comment(_) | Node::ProcessingInstruction(_) | Node::Doctype(_) => {}
        _ => {
            for child in node.children() {
                write(child, skip, out);
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
