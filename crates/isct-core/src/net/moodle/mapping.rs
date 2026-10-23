//! Pure mapping from Moodle results to the raw per-key shapes that
//! `school_parser::lms` validates. No I/O.
use crate::school_parser::lms::{id_of, is_lms_origin, read_timetable, LMS_PATH, LMS_WWWROOT};
use chrono::{Datelike, FixedOffset, NaiveDate};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::Mutex;
use url::Url;

/// Every activity kind whose details a course read fills in.
pub(crate) const DETAIL_MODULES: [&str; 6] =
    ["resource", "folder", "url", "page", "forum", "assign"];

/// The generic block API returns the block's inner HTML. Reuse the same
/// school-specific parser as the cookie reader; no separate timetable rules.
pub(super) fn dashboard_timetable(value: &Value) -> Option<Value> {
    let blocks = value.get("blocks")?.as_array()?;
    let block = blocks.iter().find(|block| {
        block.get("name").and_then(Value::as_str) == Some("tttimetable")
            && block.get("visible").and_then(Value::as_bool) != Some(false)
    })?;
    let content = block.get("contents")?.get("content")?.as_str()?;
    let html = format!("<section data-block=\"tttimetable\">{content}</section>");
    let url = Url::parse(&format!("{LMS_WWWROOT}/my/")).ok()?;
    read_timetable(&html, &url)
}

/// A copy of `value` with only `keys`, as the extractor kept its records.
pub(crate) fn pick(value: &Value, keys: &[&str]) -> Value {
    let mut out = Map::new();
    for key in keys {
        if let Some(field) = value.get(*key) {
            out.insert((*key).to_string(), field.clone());
        }
    }
    Value::Object(out)
}

pub(crate) fn course_row(course: &Value) -> Value {
    pick(
        course,
        &[
            "id",
            "fullname",
            "shortname",
            "idnumber",
            "coursecategory",
            "startdate",
            "enddate",
            "viewurl",
            "hidden",
        ],
    )
}

/// The academic year in a shortname's `[2026]` suffix.
pub(crate) fn course_year(course: &Value) -> Option<i32> {
    let shortname = course.get("shortname").and_then(Value::as_str)?.trim_end();
    let inner = shortname.strip_suffix(']')?;
    let open = inner.rfind('[')?;
    let year = &inner[open + 1..];
    (year.len() == 4 && year.bytes().all(|b| b.is_ascii_digit()))
        .then(|| year.parse().ok())
        .flatten()
}

/// The `lms.history` months: April to March of every enrolled academic
/// year, up to today, at most 48.
pub(crate) fn months_to_read(years: &[i32], today: NaiveDate) -> Vec<(i32, u32)> {
    let mut seen = HashSet::new();
    let mut months = Vec::new();
    for year in years.iter().copied().filter(|year| seen.insert(*year)) {
        for index in 0..12u32 {
            if months.len() >= 48 {
                return months;
            }
            let (y, m) = if index < 9 {
                (year, 4 + index)
            } else {
                (year + 1, index - 8)
            };
            if NaiveDate::from_ymd_opt(y, m, 1).is_some_and(|first| first <= today) {
                months.push((y, m));
            }
        }
    }
    months
}

/// The cmid in an activity link (`/mod/<name>/view.php?id=<cmid>`).
pub(crate) fn view_url_cmid(raw: &str) -> Option<u64> {
    let url = Url::parse(raw).ok().filter(is_lms_origin)?;
    let rest = url.path().strip_prefix(LMS_PATH)?.strip_prefix("mod/")?;
    let (_, file) = rest.split_once('/')?;
    if file != "view.php" {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key == "id")
        .and_then(|(_, value)| value.parse().ok())
}

pub(crate) const ACTION_EVENT_KEYS: [&str; 9] = [
    "id",
    "instance",
    "modulename",
    "name",
    "activityname",
    "timesort",
    "overdue",
    "eventtype",
    "url",
];
pub(crate) const UPCOMING_EVENT_KEYS: [&str; 7] = [
    "id",
    "instance",
    "modulename",
    "name",
    "timesort",
    "eventtype",
    "url",
];

/// One calendar event in the stored shape. Ingest keys work by cmid; the
/// activity link names it, so it is used whenever present. (The calendar
/// exporter fills `instance` with the cmid as well; the link makes that
/// independent of the exporter.)
pub(crate) fn normalize_event(event: &Value, keys: &[&str]) -> Value {
    let mut out = pick(event, keys);
    if let Some(cmid) = event
        .get("url")
        .and_then(Value::as_str)
        .and_then(view_url_cmid)
    {
        out["instance"] = json!(cmid);
    }
    if out.get("timesort").is_none() {
        if let Some(time) = event.get("timesort").or_else(|| event.get("timestart")) {
            out["timesort"] = time.clone();
        }
    }
    out["course"] = json!({"id": event.get("course").and_then(|course| course.get("id"))});
    out
}

/// Assignment and quiz deadlines from a monthly calendar view.
pub(crate) fn month_deadlines(view: &Value) -> Vec<Value> {
    let list = |value: Option<&Value>, key: &str| {
        value
            .and_then(|value| value.get(key))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for week in list(Some(view), "weeks") {
        for day in list(Some(&week), "days") {
            for event in list(Some(&day), "events") {
                let module = event.get("modulename").and_then(Value::as_str);
                let kind = event.get("eventtype").and_then(Value::as_str);
                if matches!(module, Some("assign" | "quiz"))
                    && matches!(kind, Some("due" | "close"))
                {
                    out.push(normalize_event(
                        &event,
                        &["id", "instance", "modulename", "name", "eventtype", "url"],
                    ));
                }
            }
        }
    }
    out
}

/// A REST file URL in the form the app stores: `pluginfile.php` instead of
/// `webservice/pluginfile.php` and no credential, so the link stays valid
/// whichever transport later downloads it. `None` off the LMS origin.
pub(crate) fn plain_file_url(raw: &str) -> Option<String> {
    let mut url = Url::parse(raw).ok().filter(is_lms_origin)?;
    let path = url
        .path()
        .replacen("/webservice/pluginfile.php/", "/pluginfile.php/", 1);
    url.set_path(&path);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| !matches!(key.as_ref(), "token" | "wstoken"))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if kept.is_empty() {
        url.set_query(None);
    } else {
        url.query_pairs_mut().clear().extend_pairs(kept);
    }
    Some(url.into())
}

/// Teacher HTML from REST, with web-service file links in the stored form.
pub(crate) fn plain_html(html: &str) -> String {
    html.replace(
        &format!("{LMS_WWWROOT}/webservice/pluginfile.php/"),
        &format!("{LMS_WWWROOT}/pluginfile.php/"),
    )
}

fn non_empty_html(html: Option<&str>) -> Option<String> {
    html.map(str::trim)
        .filter(|html| !html.is_empty())
        .map(plain_html)
}

pub(crate) fn escape_html(raw: &str, out: &mut String) {
    for character in raw.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

/// A downloadable file listed by `core_course_get_contents`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContentFile {
    pub(crate) name: String,
    pub(crate) url: String,
}

/// What the token transport keeps about a module beyond the stored state.
#[derive(Clone, Debug, Default)]
pub(crate) struct ModuleInfo {
    pub(crate) module: String,
    pub(crate) uservisible: bool,
    pub(crate) files: Vec<ContentFile>,
    /// The newest `timemodified` of its contents; 0 when it lists none
    /// (assignments, forums), so its details cannot be known unchanged.
    pub(crate) modified: i64,
}

fn flag(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_i64().is_some_and(|n| n != 0),
        _ => default,
    }
}

/// `core_course_get_contents` as the stored course state: the same
/// `{section, cm}` outline the course-format state gave, with section
/// summaries and on-page descriptions as `bodyHtml`, and the file or link
/// of resources and URLs as `resourceUrl`.
pub(crate) fn structure_from_contents(contents: &Value) -> Option<(Value, Vec<(u64, ModuleInfo)>)> {
    let sections = contents.as_array()?;
    let mut section_rows = Vec::new();
    let mut cms = Vec::new();
    let mut modules = Vec::new();
    for section in sections {
        let section_id = id_of(section.get("id"))?;
        let number = section.get("section").and_then(Value::as_i64).unwrap_or(0);
        let mut row = json!({
            "id": section_id,
            "number": number,
            "title": section.get("name").and_then(Value::as_str).unwrap_or(""),
            "visible": flag(section.get("visible"), true) && flag(section.get("uservisible"), true),
            "anchor": format!("section-{number}"),
        });
        if let Some(body) = non_empty_html(section.get("summary").and_then(Value::as_str)) {
            row["bodyHtml"] = body.into();
        }
        section_rows.push(row);
        for module in section
            .get("modules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(cmid) = id_of(module.get("id")) else {
                continue;
            };
            let kind = module.get("modname").and_then(Value::as_str).unwrap_or("");
            let name = module.get("name").and_then(Value::as_str).unwrap_or("");
            let uservisible = flag(module.get("uservisible"), true);
            let entries: Vec<&Value> = module
                .get("contents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .collect();
            let files: Vec<ContentFile> = entries
                .iter()
                .filter(|entry| entry.get("type").and_then(Value::as_str) == Some("file"))
                .filter_map(|entry| {
                    let url = plain_file_url(entry.get("fileurl")?.as_str()?)?;
                    let path = entry
                        .get("filepath")
                        .and_then(Value::as_str)
                        .unwrap_or("/")
                        .trim_start_matches('/');
                    let file = entry.get("filename").and_then(Value::as_str).unwrap_or("");
                    Some(ContentFile {
                        name: format!("{path}{file}"),
                        url,
                    })
                })
                .collect();
            let modified = entries
                .iter()
                .filter_map(|entry| entry.get("timemodified").and_then(Value::as_i64))
                .max()
                .unwrap_or(0);
            let mut cm = json!({
                "id": cmid,
                "name": name,
                "module": kind,
                "sectionid": section_id,
                "visible": flag(module.get("visible"), true),
                "uservisible": uservisible,
                "anchor": format!("module-{cmid}"),
            });
            if let Some(url) = module.get("url").and_then(Value::as_str) {
                cm["url"] = url.into();
            }
            if modified > 0 {
                cm["modified"] = modified.into();
            }
            if let Some(body) = non_empty_html(module.get("description").and_then(Value::as_str)) {
                cm["bodyHtml"] = body.into();
            }
            let resource = match kind {
                "resource" => files.first().map(|file| file.url.clone()),
                // A URL module's single entry is the link itself.
                "url" => entries
                    .iter()
                    .find(|entry| entry.get("type").and_then(Value::as_str) == Some("url"))
                    .and_then(|entry| entry.get("fileurl").and_then(Value::as_str))
                    .map(|raw| plain_file_url(raw).unwrap_or_else(|| raw.to_string())),
                _ => None,
            };
            if let Some(resource) = resource {
                cm["resourceUrl"] = resource.into();
            }
            cms.push(cm);
            modules.push((
                cmid,
                ModuleInfo {
                    module: kind.to_string(),
                    uservisible,
                    files,
                    modified,
                },
            ));
        }
    }
    Some((json!({"section": section_rows, "cm": cms}), modules))
}

/// A folder's files as the folder tree the view page showed.
pub(crate) fn folder_tree(files: &[ContentFile]) -> String {
    let mut out = String::from("<ul class=\"foldertree\">");
    for file in files {
        out.push_str("<li><a href=\"");
        escape_html(&file.url, &mut out);
        out.push_str("\">");
        escape_html(&file.name, &mut out);
        out.push_str("</a></li>");
    }
    out.push_str("</ul>");
    out
}

/// Links to an intro's attachments (`introattachments`, `introfiles`).
pub(crate) fn attachment_links(files: Option<&Value>) -> String {
    let mut out = String::new();
    for file in files.and_then(Value::as_array).into_iter().flatten() {
        let Some(url) = file
            .get("fileurl")
            .and_then(Value::as_str)
            .and_then(plain_file_url)
        else {
            continue;
        };
        out.push_str("<a href=\"");
        escape_html(&url, &mut out);
        out.push_str("\">");
        escape_html(
            file.get("filename").and_then(Value::as_str).unwrap_or(""),
            &mut out,
        );
        out.push_str("</a>");
    }
    out
}

fn text_of_html(html: &str) -> String {
    scraper::Html::parse_fragment(html)
        .root_element()
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `mod_assign_get_submission_status` as the assignment page showed it.
/// The page's graded cell is Moodle's `graded`/`released` grading status;
/// only the exact `submitted` status counts as submitted (a draft does not).
pub(crate) fn assign_state(cmid: u64, status: &Value) -> Option<Value> {
    let attempt = status.get("lastattempt")?;
    let submission = attempt
        .get("submission")
        .or_else(|| attempt.get("teamsubmission"))
        .and_then(|submission| submission.get("status"))
        .and_then(Value::as_str);
    let grading = attempt.get("gradingstatus").and_then(Value::as_str);
    let state = if matches!(grading, Some("graded" | "released")) {
        "graded"
    } else if submission == Some("submitted") {
        "submitted"
    } else {
        "open"
    };
    let grade = status
        .get("feedback")
        .and_then(|feedback| feedback.get("gradefordisplay"))
        .and_then(Value::as_str)
        .map(text_of_html)
        .filter(|grade| !grade.is_empty() && grade != "-");
    Some(json!({
        "cmid": cmid,
        "submission": state,
        "grade": grade,
        "acceptsSubmission": flag(attempt.get("canedit"), false),
    }))
}

fn format_grade(value: f64, decimals: usize) -> String {
    format!("{value:.decimals$}")
}

/// A quiz as its view page showed it: graded once a grade is shown,
/// submitted once an attempt is finished, open otherwise.
pub(crate) fn quiz_state(cmid: u64, attempts: &Value, best: &Value, quiz: Option<&Value>) -> Value {
    let finished = attempts
        .get("attempts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|attempt| attempt.get("state").and_then(Value::as_str) == Some("finished"));
    let number = |value: Option<&Value>| match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    };
    let decimals = quiz
        .and_then(|quiz| quiz.get("decimalpoints"))
        .and_then(Value::as_u64)
        .unwrap_or(2)
        .min(5) as usize;
    let grade = (finished && flag(best.get("hasgrade"), false))
        .then(|| {
            let got = number(best.get("grade"))?;
            let max = number(quiz?.get("grade"))?;
            Some(format!(
                "{} / {}",
                format_grade(got, decimals),
                format_grade(max, decimals)
            ))
        })
        .flatten();
    let submission = if grade.is_some() {
        "graded"
    } else if finished {
        "submitted"
    } else {
        "open"
    };
    json!({"cmid": cmid, "submission": submission, "grade": grade})
}

/// A post date as Moodle's Japanese pages print it, which the desktop's
/// announcement reader recognizes.
pub(crate) fn jst_label(timestamp: i64) -> Option<String> {
    let offset = FixedOffset::east_opt(9 * 3600)?;
    let time = chrono::DateTime::from_timestamp(timestamp, 0)?.with_timezone(&offset);
    Some(format!(
        "{}年 {}月 {}日 {}",
        time.year(),
        time.month(),
        time.day(),
        time.format("%H:%M")
    ))
}

/// One discussion of `mod_forum_get_forum_discussions` as the announcement
/// article the forum page reader produced.
pub(crate) fn forum_article(discussion: &Value) -> Option<String> {
    let post = id_of(discussion.get("id"))?;
    let thread = id_of(discussion.get("discussion"))?;
    let subject = ["subject", "name"]
        .iter()
        .find_map(|key| discussion.get(*key).and_then(Value::as_str))
        .unwrap_or("");
    let mut out = format!(
        "<article id=\"announcement-p{post}\" data-source-url=\"{LMS_WWWROOT}/mod/forum/discuss.php?d={thread}\"><h5>"
    );
    escape_html(subject, &mut out);
    out.push_str("</h5>");
    if let Some(label) = ["created", "timemodified", "modified"]
        .iter()
        .find_map(|key| discussion.get(*key).and_then(Value::as_i64))
        .filter(|time| *time > 0)
        .and_then(jst_label)
    {
        out.push_str("<time>");
        out.push_str(&label);
        out.push_str("</time>");
    }
    out.push_str(&format!("<div id=\"post-content-{post}\">"));
    out.push_str(&plain_html(
        discussion
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ));
    out.push_str("</div>");
    out.push_str(&attachment_links(discussion.get("attachments")));
    out.push_str("</article>");
    Some(out)
}

/// Teacher text kept per course read: 100,000 characters per body and 3,000,000 in
/// all, as before. Once spent, bodies are not replaced, so cached ones stay.
pub(crate) struct BodyBudget(Mutex<usize>);

impl BodyBudget {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(3_000_000))
    }

    pub(crate) fn take(&self, text: &str) -> Option<String> {
        let mut remaining = self.0.lock().ok()?;
        if *remaining == 0 {
            return None;
        }
        let limit = (*remaining).min(100_000);
        let end = text
            .char_indices()
            .nth(limit)
            .map_or(text.len(), |(index, _)| index);
        let body = &text[..end];
        *remaining = remaining.saturating_sub(body.chars().count());
        Some(body.to_string())
    }
}

#[cfg(test)]
#[path = "../../tests/net__moodle__mapping.rs"]
mod tests;
