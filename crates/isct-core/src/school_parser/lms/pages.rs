//! Moodle pages read with the school session cookie: the dashboard's custom
//! timetable block, course pages and activity pages. These are the Rust ports
//! of the DOM reading the injected LMS script used to do. The transport owns
//! HTTP; this module only reads bounded HTML.
use super::{is_lms_origin, LMS_PATH};
use ego_tree::{NodeId, NodeRef};
use regex::Regex;
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::{json, Value};
use std::{borrow::Cow, collections::HashSet, sync::OnceLock};
use url::Url;

fn selector(raw: &str) -> Option<Selector> {
    Selector::parse(raw).ok()
}

fn select_all<'a>(scope: ElementRef<'a>, raw: &str) -> Vec<ElementRef<'a>> {
    selector(raw)
        .map(|selector| scope.select(&selector).collect())
        .unwrap_or_default()
}

fn text_of(element: ElementRef<'_>) -> String {
    element.text().collect()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn child_elements<'a>(element: ElementRef<'a>) -> impl Iterator<Item = ElementRef<'a>> {
    element.children().filter_map(ElementRef::wrap)
}

fn element_by_id<'a>(page: &'a Html, id: &str) -> Option<ElementRef<'a>> {
    page.root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
        .find(|element| element.value().id() == Some(id))
}

fn lms_path(rest: &str) -> String {
    format!("{LMS_PATH}{rest}")
}

/// The `M.cfg` values a transport needs from any signed-in Moodle page.
#[derive(Clone, Default)]
pub struct PageConfig {
    pub sesskey: Option<String>,
    pub language: String,
}

impl std::fmt::Debug for PageConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PageConfig")
            .field("sesskey", &self.sesskey.as_ref().map(|_| "[redacted]"))
            .field("language", &self.language)
            .finish()
    }
}

fn config_pattern(cell: &'static OnceLock<Option<Regex>>, raw: &str) -> Option<&'static Regex> {
    cell.get_or_init(|| Regex::new(raw).ok()).as_ref()
}

/// Reads `sesskey` and `language` from the page's `M.cfg` script. Both are
/// leaf values in a JSON object literal, so a pattern is enough.
pub fn page_config(html: &str) -> PageConfig {
    static SESSKEY: OnceLock<Option<Regex>> = OnceLock::new();
    static LANGUAGE: OnceLock<Option<Regex>> = OnceLock::new();
    let capture = |pattern: Option<&Regex>| {
        pattern
            .and_then(|pattern| pattern.captures(html))
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_string())
    };
    PageConfig {
        sesskey: capture(config_pattern(
            &SESSKEY,
            r#""sesskey":"([A-Za-z0-9]{1,64})""#,
        )),
        language: capture(config_pattern(
            &LANGUAGE,
            r#""language":"([A-Za-z_-]{1,16})""#,
        ))
        .unwrap_or_default(),
    }
}

fn login_form(page: &Html) -> bool {
    let root = page.root_element();
    !select_all(root, "input[type=password]").is_empty()
        || select_all(root, "form[action]").iter().any(|form| {
            form.value()
                .attr("action")
                .is_some_and(|action| action.to_ascii_lowercase().contains("login"))
        })
}

// ---------------------------------------------------------------------------
// Dashboard timetable block (`data-block="tttimetable"`)

/// The year and quarter choices of the timetable block, and the hidden
/// search flag its form posts back.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TimetableFilters {
    /// `(value, label)` of every offered year.
    pub years: Vec<(String, String)>,
    /// `(value, label)` of every offered quarter.
    pub quarters: Vec<(String, String)>,
    pub search: String,
}

fn timetable_block(page: &Html) -> Option<ElementRef<'_>> {
    select_all(page.root_element(), r#"[data-block="tttimetable"]"#)
        .into_iter()
        .next()
}

/// Rows of this table only, as `HTMLTableElement.rows` lists them.
fn table_rows(table: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    let mut rows = Vec::new();
    for child in child_elements(table) {
        match child.value().name() {
            "tr" => rows.push(child),
            "thead" | "tbody" | "tfoot" => {
                rows.extend(child_elements(child).filter(|row| row.value().name() == "tr"))
            }
            _ => {}
        }
    }
    rows
}

fn row_cells(row: ElementRef<'_>) -> Vec<ElementRef<'_>> {
    child_elements(row)
        .filter(|cell| matches!(cell.value().name(), "td" | "th"))
        .collect()
}

/// The option a single `<select>` submits: the `selected` one, else the first.
fn selected_label(block: ElementRef<'_>, name: &str) -> String {
    let Some(select) = select_all(block, &format!(r#"select[name="{name}"]"#))
        .into_iter()
        .next()
    else {
        return String::new();
    };
    let options = select_all(select, "option");
    options
        .iter()
        .find(|option| option.value().attr("selected").is_some())
        .or_else(|| options.first())
        .map(|option| text_of(*option).trim().to_string())
        .unwrap_or_default()
}

fn course_link_id(link: ElementRef<'_>, page_url: &Url) -> Option<u64> {
    let url = page_url.join(link.value().attr("href")?).ok()?;
    if !is_lms_origin(&url) || url.path() != lms_path("course/view.php") {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key == "id")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .filter(|id| *id > 0)
}

/// The grid of the selected year and quarter, in the raw shape
/// `validate_payload` accepts: `{year, quarter, slots:[{courseId, day,
/// period}], unscheduled}`. `None` when the block is missing or is not the
/// shape this reader knows; one unknown row rejects the whole grid.
pub fn read_timetable(html: &str, page_url: &Url) -> Option<Value> {
    let page = Html::parse_document(html);
    let block = timetable_block(&page)?;
    let tables = select_all(block, "table");
    let (grid, other) = (*tables.first()?, *tables.get(1)?);
    let rows = table_rows(grid);
    if rows.len() < 2 || row_cells(rows[0]).len() < 6 {
        return None;
    }
    let year = selected_label(block, "year")
        .parse::<u64>()
        .ok()
        .filter(|year| (2000..2100).contains(year))?;
    let quarter = selected_label(block, "quota");
    if !matches!(quarter.as_str(), "1" | "2" | "3" | "4") {
        return None;
    }
    let links = r#"a[href*="/course/view.php?id="]"#;
    let mut slots = Vec::new();
    for row in rows.iter().skip(1) {
        let cells = row_cells(*row);
        let period = cells
            .first()
            .and_then(|cell| text_of(*cell).trim().parse::<u64>().ok())
            .filter(|period| (1..=12).contains(period))?;
        for (index, cell) in cells.iter().skip(1).enumerate().take(7) {
            for link in select_all(*cell, links) {
                if let Some(id) = course_link_id(link, page_url) {
                    slots.push(json!({"courseId": id, "day": index + 1, "period": period}));
                }
            }
        }
    }
    let unscheduled: Vec<u64> = select_all(other, links)
        .into_iter()
        .filter_map(|link| course_link_id(link, page_url))
        .collect();
    Some(json!({"year": year, "quarter": quarter, "slots": slots, "unscheduled": unscheduled}))
}

/// The block's year and quarter choices; `None` without the block.
pub fn timetable_filters(html: &str) -> Option<TimetableFilters> {
    let page = Html::parse_document(html);
    let block = timetable_block(&page)?;
    let options = |name: &str, accept: fn(&str) -> bool| {
        select_all(block, &format!(r#"select[name="{name}"] option"#))
            .into_iter()
            .filter_map(|option| {
                let label = text_of(option).trim().to_string();
                accept(&label).then(|| {
                    let value = option.value().attr("value").unwrap_or(&label).to_string();
                    (value, label)
                })
            })
            .collect::<Vec<_>>()
    };
    Some(TimetableFilters {
        years: options("year", |label| {
            label.len() == 4 && label.starts_with("20") && label.bytes().all(|b| b.is_ascii_digit())
        }),
        quarters: options("quota", |label| matches!(label, "1" | "2" | "3" | "4")),
        search: select_all(block, r#"input[name="search"]"#)
            .first()
            .and_then(|input| input.value().attr("value"))
            .unwrap_or_default()
            .to_string(),
    })
}

/// What an unreadable grid looked like, for the timetable error list.
pub fn timetable_shape(html: &str) -> String {
    let page = Html::parse_document(html);
    let Some(block) = timetable_block(&page) else {
        return "tables=0, rows=, selection=".into();
    };
    let tables = select_all(block, "table");
    let rows = tables
        .iter()
        .map(|table| table_rows(*table).len().to_string())
        .collect::<Vec<_>>()
        .join(",");
    let selection = select_all(block, "select")
        .into_iter()
        .map(|select| {
            let name = select.value().attr("name").unwrap_or_default();
            format!("{name}={}", selected_label(block, name))
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "tables={}, rows={rows}, selection={selection}",
        tables.len()
    )
}

// ---------------------------------------------------------------------------
// Faithful re-serialization of a subtree, with URLs made absolute.

const VOID: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn escape(raw: &str, attribute: bool, out: &mut String) {
    for character in raw.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            other => out.push(other),
        }
    }
}

struct Writer<'u> {
    base: &'u Url,
    skip: HashSet<NodeId>,
}

impl Writer<'_> {
    fn inner(&self, node: NodeRef<'_, Node>) -> String {
        let mut out = String::new();
        for child in node.children() {
            self.node(child, &mut out);
        }
        out
    }

    fn outer(&self, node: NodeRef<'_, Node>) -> String {
        let mut out = String::new();
        self.node(node, &mut out);
        out
    }

    fn node(&self, node: NodeRef<'_, Node>, out: &mut String) {
        match node.value() {
            Node::Text(text) => escape(text, false, out),
            Node::Element(element) => {
                if self.skip.contains(&node.id()) {
                    return;
                }
                let name = element.name();
                out.push('<');
                out.push_str(name);
                for (attribute, value) in element.attrs() {
                    let value = if matches!(attribute, "href" | "src") {
                        // A URL that cannot be resolved is dropped, as before.
                        match self.base.join(value) {
                            Ok(url) => Cow::Owned(url.to_string()),
                            Err(_) => continue,
                        }
                    } else {
                        Cow::Borrowed(value)
                    };
                    out.push(' ');
                    out.push_str(attribute);
                    out.push_str("=\"");
                    escape(&value, true, out);
                    out.push('"');
                }
                out.push('>');
                if VOID.contains(&name) {
                    return;
                }
                for child in node.children() {
                    self.node(child, out);
                }
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
            Node::Comment(_) | Node::Doctype(_) | Node::ProcessingInstruction(_) => {}
            _ => {
                for child in node.children() {
                    self.node(child, out);
                }
            }
        }
    }

    /// Text, an image or a link survives the skipped parts.
    fn meaningful(&self, node: NodeRef<'_, Node>) -> bool {
        node.children().any(|child| match child.value() {
            Node::Text(text) => !text.trim().is_empty(),
            Node::Element(element) => {
                !self.skip.contains(&child.id())
                    && (element.name() == "img"
                        || (element.name() == "a" && element.attr("href").is_some())
                        || self.meaningful(child))
            }
            _ => false,
        })
    }
}

// ---------------------------------------------------------------------------
// Course page: teacher content around activities and in section summaries.

/// Bodies read from `course/view.php`, keyed by section id and cmid.
#[derive(Clone, Debug, Default)]
pub struct CoursePageBodies {
    pub sections: Vec<(u64, String)>,
    pub modules: Vec<(u64, String)>,
}

/// Theme chrome inside a copied node: controls, hidden helpers, same-page
/// anchors and theme icons.
fn chrome(root: ElementRef<'_>, base: &Url) -> HashSet<NodeId> {
    let mut skip = HashSet::new();
    for element in root.descendants().skip(1).filter_map(ElementRef::wrap) {
        let value = element.value();
        let name = value.name();
        if matches!(
            name,
            "script" | "style" | "input" | "button" | "label" | "form"
        ) || value.attr("aria-hidden") == Some("true")
        {
            skip.insert(element.id());
            continue;
        }
        if name == "a" {
            if let Some(target) = value.attr("href").and_then(|raw| base.join(raw).ok()) {
                if target.path() == base.path()
                    && target.query() == base.query()
                    && target.fragment().is_some()
                {
                    skip.insert(element.id());
                }
            }
        }
        if name == "img"
            && value
                .attr("src")
                .and_then(|raw| base.join(raw).ok())
                .is_some_and(|url| url.as_str().contains("/theme/image.php/"))
        {
            skip.insert(element.id());
        }
    }
    skip
}

fn id_of(value: Option<&Value>) -> Option<u64> {
    super::id_of(value)
}

/// Port of the course-content reader: activity identity comes from Moodle's
/// API state (`cm[].anchor`, `section[].anchor`), never from theme classes.
pub fn course_page_bodies(html: &str, page_url: &Url, state: &Value) -> CoursePageBodies {
    let page = Html::parse_document(html);
    let mut bodies = CoursePageBodies::default();
    let list = |key: &str| {
        state
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for cm in list("cm") {
        let Some(id) = id_of(cm.get("id")) else {
            continue;
        };
        let anchor = cm
            .get("anchor")
            .and_then(Value::as_str)
            .filter(|anchor| !anchor.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("module-{id}"));
        let Some(node) = element_by_id(&page, &anchor) else {
            continue;
        };
        let mut skip = chrome(node, page_url);
        // The activity's own link is the material row itself.
        if let Some(own) = cm
            .get("url")
            .and_then(Value::as_str)
            .and_then(|raw| page_url.join(raw).ok())
        {
            for link in select_all(node, "a[href]") {
                if link
                    .value()
                    .attr("href")
                    .and_then(|raw| page_url.join(raw).ok())
                    .is_some_and(|url| url == own)
                {
                    skip.insert(link.id());
                }
            }
        }
        let writer = Writer {
            base: page_url,
            skip,
        };
        if writer.meaningful(*node) {
            bodies.modules.push((id, writer.inner(*node)));
        }
    }
    for section in list("section") {
        let Some(id) = id_of(section.get("id")) else {
            continue;
        };
        let anchor = section
            .get("anchor")
            .and_then(Value::as_str)
            .filter(|anchor| !anchor.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "section-{}",
                    section.get("number").and_then(Value::as_i64).unwrap_or(0)
                )
            });
        let Some(node) = element_by_id(&page, &anchor) else {
            continue;
        };
        let mut skip = chrome(node, page_url);
        // Strip complete activities and section navigation; what remains is
        // the teacher's summary, including text, tables and attachments.
        for element in node.descendants().skip(1).filter_map(ElementRef::wrap) {
            let value = element.value();
            if value.id().is_some_and(|id| {
                id.strip_prefix("module-")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            }) {
                skip.insert(element.id());
            }
            if value.name() == "a"
                && page_url
                    .join(value.attr("href").unwrap_or(page_url.as_str()))
                    .is_ok_and(|url| url.path().ends_with("/course/section.php"))
            {
                let heading = element
                    .ancestors()
                    .take_while(|ancestor| ancestor.id() != node.id())
                    .filter_map(ElementRef::wrap)
                    .find(|ancestor| {
                        matches!(
                            ancestor.value().name(),
                            "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                        )
                    });
                skip.insert(heading.map_or(element.id(), |heading| heading.id()));
            }
        }
        let writer = Writer {
            base: page_url,
            skip,
        };
        if writer.meaningful(*node) {
            bodies.sections.push((id, writer.inner(*node)));
        }
    }
    bodies
}

// ---------------------------------------------------------------------------
// Activity pages (`mod/*/view.php`).

/// The page is the activity `cmid` and not a sign-in form.
pub fn module_page_matches(html: &str, cmid: u64) -> bool {
    let page = Html::parse_document(html);
    let class = format!("cmid-{cmid}");
    !login_form(&page)
        && select_all(page.root_element(), "body")
            .first()
            .is_some_and(|body| body.value().classes().any(|name| name == class))
}

/// The description and module content an activity page shows, as the
/// injected reader collected them (`#intro` plus the folder tree or page box).
pub fn module_body(html: &str, page_url: &Url, module: &str) -> String {
    let page = Html::parse_document(html);
    let nodes = match module {
        "folder" => "#intro, .foldertree",
        "page" => "#intro, .generalbox",
        _ => "#intro",
    };
    let writer = Writer {
        base: page_url,
        skip: HashSet::new(),
    };
    select_all(page.root_element(), nodes)
        .into_iter()
        .map(|node| writer.inner(*node))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Discussion links of a forum page in page order, each once, as
/// `discuss.php?d=<id>`.
pub fn forum_discussion_links(html: &str, page_url: &Url) -> Vec<Url> {
    let page = Html::parse_document(html);
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    for link in select_all(page.root_element(), "a[href]") {
        let Some(mut url) = link
            .value()
            .attr("href")
            .and_then(|raw| page_url.join(raw).ok())
        else {
            continue;
        };
        if !is_lms_origin(&url) || url.path() != lms_path("mod/forum/discuss.php") {
            continue;
        }
        let Some(id) = url
            .query_pairs()
            .find(|(key, _)| key == "d")
            .map(|(_, value)| value.into_owned())
            .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        url.set_fragment(None);
        url.set_query(Some(&format!("d={id}")));
        if seen.insert(id) {
            links.push(url);
        }
    }
    links
}

/// Every post of a discussion page as an `<article>`: the subject as `<h5>`,
/// the shown date as `<time>`, the body and attachments outside it.
pub fn forum_posts(html: &str, page_url: &Url) -> String {
    let page = Html::parse_document(html);
    let writer = Writer {
        base: page_url,
        skip: HashSet::new(),
    };
    let mut articles = Vec::new();
    for post in page
        .root_element()
        .descendants()
        .filter_map(ElementRef::wrap)
    {
        let Some(number) = post.value().id().and_then(|id| id.strip_prefix('p')) else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Some(content) = element_by_id(&page, &format!("post-content-{number}")) else {
            continue;
        };
        let mut article = String::from("<article id=\"announcement-p");
        article.push_str(number);
        article.push_str("\" data-source-url=\"");
        escape(page_url.as_str(), true, &mut article);
        article.push_str("\">");
        if let Some(heading) = select_all(post, "h3").first() {
            article.push_str("<h5>");
            escape(&text_of(*heading), false, &mut article);
            article.push_str("</h5>");
        }
        if let Some(time) = select_all(post, "time").first() {
            article.push_str("<time>");
            escape(&text_of(*time), false, &mut article);
            article.push_str("</time>");
        }
        article.push_str(&writer.outer(*content));
        for link in select_all(post, "a[href]") {
            let inside = link
                .ancestors()
                .any(|ancestor| ancestor.id() == content.id());
            let attachment = link
                .value()
                .attr("href")
                .and_then(|raw| page_url.join(raw).ok())
                .is_some_and(|url| url.path().contains("/pluginfile.php/"));
            if attachment && !inside {
                article.push_str(&writer.outer(*link));
            }
        }
        article.push_str("</article>");
        articles.push(article);
    }
    articles.join("\n")
}

/// One's own work as an assignment or quiz page shows it, in the raw
/// `assignStatus` row shape: `{cmid, submission, grade, acceptsSubmission}`.
pub fn page_assign_state(html: &str, cmid: u64, module: &str) -> Option<Value> {
    if !module_page_matches(html, cmid) {
        return None;
    }
    let page = Html::parse_document(html);
    let root = page.root_element();
    match module {
        "assign" => {
            // Read through the classes Moodle puts on the status table cells.
            let table = *select_all(root, ".submissionstatustable").first()?;
            let has = |name: &str| !select_all(table, &format!("td.{name}")).is_empty();
            let submission = if has("submissiongraded") {
                "graded"
            } else if has("submissionstatussubmitted") {
                "submitted"
            } else {
                "open"
            };
            // Released feedback leads with the grade as plain text; without a
            // grade the first row is the grader or a comment, both markup.
            let grade = select_all(root, ".feedback .feedbacktable table tr td")
                .first()
                .filter(|cell| child_elements(**cell).next().is_none())
                .map(|cell| collapse_whitespace(&text_of(*cell)))
                .filter(|text| !text.is_empty() && text != "-");
            // The add/edit button shows only while Moodle takes work from
            // this student: before the cut-off date, late or not.
            let accepts = !select_all(
                root,
                r#"[href*="action=editsubmission"], input[name="action"][value="editsubmission"]"#,
            )
            .is_empty();
            Some(
                json!({"cmid": cmid, "submission": submission, "grade": grade,
                "acceptsSubmission": accepts}),
            )
        }
        "quiz" => {
            // The attempt box is always on the quiz view page.
            select_all(root, ".quizattempt").first()?;
            // A finished attempt adds completion rows to its summary table,
            // independent of the interface language.
            let finished = select_all(root, "table.quizreviewsummary")
                .into_iter()
                .any(|table| select_all(table, "tr").len() >= 3);
            static PAIR: OnceLock<Option<Regex>> = OnceLock::new();
            let pair = config_pattern(&PAIR, r"(\d[\d.,]*)\s*/\s*(\d[\d.,]*)");
            let grade = finished
                .then(|| {
                    select_all(root, "#feedback h3")
                        .into_iter()
                        .find_map(|heading| {
                            let text = text_of(heading);
                            let captures = pair?.captures(&text)?;
                            Some(format!("{} / {}", &captures[1], &captures[2]))
                        })
                })
                .flatten();
            let submission = if grade.is_some() {
                "graded"
            } else if finished {
                "submitted"
            } else {
                "open"
            };
            Some(json!({"cmid": cmid, "submission": submission, "grade": grade}))
        }
        _ => None,
    }
}
