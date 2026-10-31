//! Readable plain text from school HTML (an assignment's instructions, an
//! LMS page body), for readers that must not receive raw HTML: paragraphs,
//! line breaks and list items survive as lines, everything else as text.
use scraper::{ElementRef, Html, Node};

const BLOCKS: &[&str] = &[
    "address",
    "article",
    "blockquote",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "ol",
    "p",
    "pre",
    "section",
    "table",
    "tr",
    "ul",
];

pub fn plain_text(html: &str) -> String {
    let fragment = Html::parse_fragment(html);
    let mut out = String::new();
    walk(fragment.root_element(), &mut out);
    let mut text = String::new();
    let mut blank = false;
    for line in out.lines().map(str::trim) {
        if line.is_empty() {
            blank = !text.is_empty();
            continue;
        }
        if !text.is_empty() {
            text.push_str(if blank { "\n\n" } else { "\n" });
        }
        text.push_str(line);
        blank = false;
    }
    text
}

fn walk(element: ElementRef<'_>, out: &mut String) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => push_words(out, text),
            Node::Element(tag) => {
                let name = tag.name();
                if name == "br" {
                    out.push('\n');
                    continue;
                }
                if matches!(name, "script" | "style") {
                    continue;
                }
                let block = BLOCKS.contains(&name);
                // Paragraphs and whole lists stand apart; other blocks only
                // start a line.
                let blank = matches!(name, "p" | "ul" | "ol" | "table");
                if block {
                    break_line(out, blank);
                }
                if name == "li" {
                    out.push_str("- ");
                }
                if let Some(child) = ElementRef::wrap(child) {
                    walk(child, out);
                }
                if block {
                    break_line(out, blank);
                }
            }
            _ => {}
        }
    }
}

/// Ends the current line, leaving a blank line after it when `blank`.
fn break_line(out: &mut String, blank: bool) {
    while out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        return;
    }
    let have = out.len() - out.trim_end_matches('\n').len();
    for _ in have..usize::from(blank) + 1 {
        out.push('\n');
    }
}

/// Appends `text` with each run of whitespace as one space, and none at the
/// start of a line.
fn push_words(out: &mut String, text: &str) {
    let starts_space = text.starts_with(char::is_whitespace);
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        if starts_space && !out.is_empty() && !out.ends_with(['\n', ' ']) {
            out.push(' ');
        }
        return;
    }
    if starts_space && !out.is_empty() && !out.ends_with(['\n', ' ']) {
        out.push(' ');
    }
    out.push_str(&words.join(" "));
    if text.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

#[cfg(test)]
#[path = "tests/html_text.rs"]
mod tests;
