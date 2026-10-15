use super::SchoolParseError;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use url::Url;

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_COURSES: usize = 500;
const MAX_STRUCTURES: usize = 500;
const MAX_ITEMS: usize = 5_000;
const MAX_EVENTS: usize = 5_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsCourse {
    pub id: u64,
    #[serde(default)]
    pub fullname: String,
    #[serde(default)]
    pub shortname: String,
    #[serde(default)]
    pub idnumber: String,
    #[serde(default, deserialize_with = "category_number")]
    pub coursecategory: u64,
    #[serde(default)]
    pub startdate: i64,
    #[serde(default)]
    pub enddate: i64,
    #[serde(default)]
    pub viewurl: String,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsStructure {
    pub course_id: u64,
    pub complete: bool,
    pub state: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsPayload {
    pub status: String,
    pub source_url: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub courses_complete: bool,
    #[serde(default)]
    pub courses: Vec<LmsCourse>,
    #[serde(default)]
    pub structures: Vec<LmsStructure>,
    #[serde(default)]
    pub action_events: Vec<Value>,
    #[serde(default)]
    pub dated_events: Vec<Value>,
    #[serde(default)]
    pub assign_status: HashMap<u64, AssignState>,
    #[serde(default)]
    pub timetable: Option<LmsTimetable>,
    #[serde(default, skip_deserializing)]
    pub skipped: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLmsPayload {
    status: String,
    source_url: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    courses_complete: bool,
    #[serde(default)]
    courses: Vec<Value>,
    #[serde(default)]
    structures: Vec<Value>,
    #[serde(default)]
    action_events: Vec<Value>,
    #[serde(default)]
    dated_events: Vec<Value>,
    #[serde(default)]
    timetable: Option<Value>,
    #[serde(default)]
    assign_status: Vec<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LmsTimetable {
    pub term: String,
    pub meetings: HashMap<u64, Vec<Value>>,
    pub unscheduled: Vec<u64>,
}

/// What an assignment or quiz page showed about one's own work.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AssignState {
    pub submission: String,
    pub grade: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LmsStage {
    Ready,
    ReconnectRequired,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolvedResource {
    File { url: String },
    External { url: String },
    Folder { files: Vec<ResolvedFile> },
    Login,
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResolvedFile {
    pub name: String,
    pub url: String,
}

pub(crate) fn id_of(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn category_number<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(number) => number.as_u64().unwrap_or(0),
        Value::String(text) => text.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

pub fn aggregate_slots(cells: &[(u64, u8, u8)]) -> HashMap<u64, Vec<Value>> {
    let mut by_course: HashMap<u64, std::collections::BTreeMap<u8, Vec<u8>>> = HashMap::new();
    for &(course, day, period) in cells {
        by_course
            .entry(course)
            .or_default()
            .entry(day)
            .or_default()
            .push(period);
    }
    by_course
        .into_iter()
        .map(|(course, days)| {
            let mut meetings = Vec::new();
            for (day, mut periods) in days {
                periods.sort_unstable();
                periods.dedup();
                let mut start = periods[0];
                let mut end = start;
                for &period in &periods[1..] {
                    if period == end + 1 {
                        end = period;
                    } else {
                        meetings.push(json!({"day": day, "periods": [start, end]}));
                        start = period;
                        end = period;
                    }
                }
                meetings.push(json!({"day": day, "periods": [start, end]}));
            }
            (course, meetings)
        })
        .collect()
}

fn parse_timetable(raw: &Value, courses: &HashSet<u64>) -> Option<LmsTimetable> {
    let year = raw
        .get("year")?
        .as_u64()
        .filter(|year| (2000..2100).contains(year))?;
    let quarter = raw
        .get("quarter")?
        .as_str()?
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|quarter| (1..=4).contains(quarter))?;
    let slots = raw.get("slots")?.as_array()?;
    if slots.len() > 500 {
        return None;
    }
    let mut cells = Vec::new();
    for slot in slots {
        let course = id_of(slot.get("courseId"))?;
        let day = slot
            .get("day")?
            .as_u64()
            .filter(|day| (1..=7).contains(day))? as u8;
        let period = slot
            .get("period")?
            .as_u64()
            .filter(|period| (1..=12).contains(period))? as u8;
        if courses.contains(&course) {
            cells.push((course, day, period));
        }
    }
    let unscheduled = raw
        .get("unscheduled")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|id| id_of(Some(id)))
                .filter(|id| courses.contains(id))
                .take(200)
                .collect()
        })
        .unwrap_or_default();
    Some(LmsTimetable {
        term: format!("{year}-{quarter}Q"),
        meetings: aggregate_slots(&cells),
        unscheduled,
    })
}

pub fn is_lms_origin(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("lms.s.isct.ac.jp")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

pub fn is_plugin_file_url(url: &Url) -> bool {
    is_lms_origin(url)
        && url.path_segments().is_some_and(|segments| {
            segments
                .into_iter()
                .any(|segment| segment == "pluginfile.php")
        })
}

pub(crate) fn lms_url(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|url| is_lms_origin(&url))
}

pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((character, end))
        });
        match decoded {
            Some((character, end)) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_fields(value: &mut Value, keys: &[&str]) {
    for key in keys {
        if let Some(Value::String(text)) = value.get_mut(*key) {
            *text = decode_entities(text);
        }
    }
}

fn valid_cm(cm: &Value) -> bool {
    id_of(cm.get("id")).is_some()
        && id_of(cm.get("sectionid")).is_some()
        && cm.get("module").and_then(Value::as_str).is_some()
        && cm.get("name").and_then(Value::as_str).is_some()
        && !cm
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| !url.is_empty() && !lms_url(url))
}

fn valid_event(event: &Value) -> bool {
    id_of(event.get("instance")).is_some()
        && id_of(event.get("course").and_then(|course| course.get("id"))).is_some()
        && event.get("timesort").and_then(Value::as_i64).is_some()
}

/// Validates and normalizes the bounded result produced by the LMS capture
/// adapter. Invalid records are counted and discarded without turning a
/// partial result into authoritative deletion input.
pub fn parse_payload(raw: &str) -> Result<(LmsStage, Option<LmsPayload>), SchoolParseError> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("LMS payload exceeds the size limit".into());
    }
    let raw: RawLmsPayload =
        serde_json::from_str(raw).map_err(|_| "LMS payload is not valid JSON")?;
    if !lms_url(&raw.source_url) {
        return Err("LMS payload has an invalid source URL".into());
    }
    if raw.status == "login" {
        return Ok((LmsStage::ReconnectRequired, None));
    }
    if raw.status != "ok" {
        return Err("LMS extraction was incomplete".into());
    }
    if raw.courses.len() > MAX_COURSES
        || raw.structures.len() > MAX_STRUCTURES
        || raw.action_events.len() > MAX_EVENTS
        || raw.dated_events.len() > MAX_EVENTS
    {
        return Err("LMS payload exceeds a collection limit".into());
    }
    let mut skipped = 0;
    let mut skipped_courses = false;
    let mut course_ids = HashSet::new();
    let mut courses = Vec::new();
    for value in raw.courses {
        let course = serde_json::from_value::<LmsCourse>(value)
            .ok()
            .filter(|course| {
                course.id != 0
                    && !course_ids.contains(&course.id)
                    && !course.fullname.trim().is_empty()
                    && course.fullname.len() <= 1_000
                    && course.shortname.len() <= 255
                    && lms_url(&course.viewurl)
            });
        match course {
            Some(mut course) => {
                course.fullname = decode_entities(&course.fullname);
                course_ids.insert(course.id);
                courses.push(course);
            }
            None => {
                skipped += 1;
                skipped_courses = true;
            }
        }
    }
    let mut item_count = 0;
    let mut structures = Vec::new();
    for value in raw.structures {
        let Ok(mut structure) = serde_json::from_value::<LmsStructure>(value) else {
            skipped += 1;
            continue;
        };
        if !course_ids.contains(&structure.course_id) {
            skipped += 1;
            continue;
        }
        if structure.complete {
            let cms = structure
                .state
                .as_ref()
                .and_then(|state| state.get("cm"))
                .and_then(Value::as_array)
                .cloned();
            let Some(cms) = cms else {
                structure.complete = false;
                structure.state = None;
                skipped += 1;
                structures.push(structure);
                continue;
            };
            let kept: Vec<Value> = cms
                .into_iter()
                .filter(valid_cm)
                .map(|mut cm| {
                    decode_fields(&mut cm, &["name"]);
                    cm
                })
                .collect();
            let total = structure
                .state
                .as_ref()
                .and_then(|state| state.get("cm"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if kept.len() < total {
                skipped += total - kept.len();
                structure.complete = false;
            }
            item_count += kept.len();
            if let Some(state) = structure.state.as_mut().and_then(Value::as_object_mut) {
                state.insert("cm".into(), Value::Array(kept));
                if let Some(Value::Array(sections)) = state.get_mut("section") {
                    for section in sections {
                        decode_fields(section, &["title"]);
                        if let Some(id) = id_of(section.get("id")) {
                            section["id"] = json!(id);
                        }
                    }
                }
            }
        }
        structures.push(structure);
    }
    if item_count > MAX_ITEMS {
        return Err("LMS structure exceeds the item limit".into());
    }
    let mut events = |list: Vec<Value>| {
        let total = list.len();
        let kept: Vec<Value> = list
            .into_iter()
            .filter(valid_event)
            .map(|mut event| {
                decode_fields(&mut event, &["name", "activityname"]);
                event
            })
            .collect();
        skipped += total - kept.len();
        kept
    };
    let action_events = events(raw.action_events);
    let dated_events = events(raw.dated_events);
    let timetable = raw
        .timetable
        .as_ref()
        .and_then(|grid| parse_timetable(grid, &course_ids));
    let assign_status = raw
        .assign_status
        .iter()
        .take(200)
        .filter_map(|row| {
            let state = row.get("submission")?.as_str()?;
            if !matches!(state, "open" | "submitted" | "graded") {
                return None;
            }
            let grade = row
                .get("grade")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|grade| !grade.is_empty() && grade.chars().count() <= 80)
                .map(str::to_string);
            Some((
                id_of(row.get("cmid"))?,
                AssignState {
                    submission: state.to_string(),
                    grade,
                },
            ))
        })
        .collect();
    Ok((
        LmsStage::Ready,
        Some(LmsPayload {
            status: raw.status,
            source_url: raw.source_url,
            language: raw.language,
            courses_complete: raw.courses_complete && !skipped_courses,
            courses,
            structures,
            action_events,
            dated_events,
            timetable,
            assign_status,
            skipped,
        }),
    ))
}

/// Parses a Moodle module page into a resource description. The downloader
/// owns HTTP and disk I/O; this function only understands bounded HTML.
pub fn parse_resource_page(html: &str, response_url: &Url, module: &str) -> ResolvedResource {
    if !is_lms_origin(response_url) {
        return ResolvedResource::Unresolved;
    }
    let page = Html::parse_document(html);
    let plugin_file = |raw: &str| response_url.join(raw).ok().filter(is_plugin_file_url);
    if module == "folder" {
        let Ok(selector) = Selector::parse(".foldertree a[href*='/pluginfile.php/']") else {
            return ResolvedResource::Unresolved;
        };
        let files = page
            .select(&selector)
            .filter_map(|link| {
                let url = plugin_file(link.value().attr("href")?)?;
                let name = link.text().collect::<String>().trim().to_string();
                let fallback = url
                    .path_segments()
                    .and_then(Iterator::last)
                    .unwrap_or("file")
                    .to_string();
                Some(ResolvedFile {
                    name: if name.is_empty() { fallback } else { name },
                    url: url.into(),
                })
            })
            .collect::<Vec<_>>();
        return if files.is_empty() {
            ResolvedResource::Unresolved
        } else {
            ResolvedResource::Folder { files }
        };
    }
    if module == "url" {
        let Ok(selector) = Selector::parse("#region-main a[href^='https://']") else {
            return ResolvedResource::Unresolved;
        };
        let Some(url) = page
            .select(&selector)
            .find_map(|link| response_url.join(link.value().attr("href")?).ok())
        else {
            return ResolvedResource::Unresolved;
        };
        if let Some(file) = plugin_file(url.as_str()) {
            return ResolvedResource::File { url: file.into() };
        }
        return ResolvedResource::External { url: url.into() };
    }
    for selector in [
        ".resourcecontent img[src]",
        ".resourcecontent object[data]",
        ".resourcecontent iframe[src]",
        ".resourcecontent video source[src], .resourcecontent audio source[src]",
        ".resourceworkaround a[href]",
        ".resourcecontent a[href]",
    ] {
        let Ok(selector) = Selector::parse(selector) else {
            continue;
        };
        if let Some(url) = page.select(&selector).find_map(|element| {
            element
                .value()
                .attr("src")
                .or_else(|| element.value().attr("data"))
                .or_else(|| element.value().attr("href"))
                .and_then(&plugin_file)
        }) {
            return ResolvedResource::File { url: url.into() };
        }
    }
    let Ok(selector) = Selector::parse(
        "[href*='pluginfile.php'], [src*='pluginfile.php'], [data*='pluginfile.php']",
    ) else {
        return ResolvedResource::Unresolved;
    };
    if let Some(url) = page.select(&selector).find_map(|element| {
        element
            .value()
            .attr("href")
            .or_else(|| element.value().attr("src"))
            .or_else(|| element.value().attr("data"))
            .and_then(&plugin_file)
    }) {
        return ResolvedResource::File { url: url.into() };
    }
    ResolvedResource::Unresolved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_page_accepts_any_academic_year_on_the_exact_lms_origin() {
        let page = Url::parse("https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
        let result = parse_resource_page(
            r#"<main><a href="/2026/pluginfile.php/1/mod_resource/content/0/file.pdf">File</a></main>"#,
            &page,
            "resource",
        );
        assert_eq!(
            result,
            ResolvedResource::File {
                url:
                    "https://lms.s.isct.ac.jp/2026/pluginfile.php/1/mod_resource/content/0/file.pdf"
                        .into()
            }
        );
    }

    #[test]
    fn resource_page_rejects_lookalike_origins() {
        let page =
            Url::parse("https://evil.lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
        assert_eq!(
            parse_resource_page(
                r#"<a href="/2026/pluginfile.php/1/mod_resource/content/0/file.pdf">File</a>"#,
                &page,
                "resource"
            ),
            ResolvedResource::Unresolved
        );
    }

    #[test]
    fn resource_page_parses_embed_folder_and_external_url_shapes() {
        let resource =
            Url::parse("https://lms.s.isct.ac.jp/2026/mod/resource/view.php?id=42").unwrap();
        assert!(matches!(
            parse_resource_page(
                r#"<div class="resourcecontent"><object data="/2026/pluginfile.php/1/object.pdf"></object></div>"#,
                &resource,
                "resource"
            ),
            ResolvedResource::File { url } if url.ends_with("/2026/pluginfile.php/1/object.pdf")
        ));

        let folder = Url::parse("https://lms.s.isct.ac.jp/2026/mod/folder/view.php?id=43").unwrap();
        assert!(matches!(
            parse_resource_page(
                r#"<div class="foldertree"><a href="/2026/pluginfile.php/1/a.txt">A</a><a href="/2026/pluginfile.php/1/b.txt">B</a></div>"#,
                &folder,
                "folder"
            ),
            ResolvedResource::Folder { files } if files.len() == 2
        ));

        let external = Url::parse("https://lms.s.isct.ac.jp/2026/mod/url/view.php?id=44").unwrap();
        assert_eq!(
            parse_resource_page(
                r#"<main id="region-main"><a href="https://example.edu/reading">Reading</a></main>"#,
                &external,
                "url"
            ),
            ResolvedResource::External {
                url: "https://example.edu/reading".into()
            }
        );
    }
}
