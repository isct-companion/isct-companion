use super::SchoolParseError;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use url::Url;

mod pages;
pub use pages::{
    course_page_bodies, forum_discussion_links, forum_posts, module_body, module_page_matches,
    page_assign_state, page_config, read_timetable, timetable_filters, timetable_shape,
    CoursePageBodies, PageConfig, TimetableFilters,
};

/// The LMS site root. The school has served Moodle under this fixed
/// academic-year path since the merger; `SCHOOL-LMS-YEAR-ROLLOVER` covers a
/// move.
pub const LMS_WWWROOT: &str = "https://lms.s.isct.ac.jp/2025";
/// The path every LMS page lives under.
pub const LMS_PATH: &str = "/2025/";

const MAX_COURSES: usize = 500;
const MAX_GRIDS: usize = 80;
const MAX_ITEMS: usize = 5_000;
const MAX_EVENTS: usize = 5_000;
const MAX_STATES: usize = 500;

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
    /// Whether the page still offers to add or edit a submission. Absent for
    /// quizzes.
    pub accepts_submission: Option<bool>,
}

/// `lms.enrolment/all`: every enrolled course and the timetable grids, the
/// dashboard's current grid first.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsEnrolment {
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub courses: Vec<LmsCourse>,
    #[serde(default)]
    pub timetables: Vec<LmsTimetable>,
}

/// `lms.deadlines/all`: open work (action events) and the upcoming view.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsDeadlines {
    #[serde(default)]
    pub action_events: Vec<Value>,
    #[serde(default)]
    pub dated_events: Vec<Value>,
}

/// `lms.course/{id}`: the course's whole outline. `modules` is the complete
/// inventory of its modules; one that is absent was removed.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsCourseContent {
    pub course_id: u64,
    #[serde(default)]
    pub sections: Vec<Value>,
    #[serde(default)]
    pub modules: Vec<Value>,
}

/// `lms.assign/{id}`: one's own state of each assignment and quiz of a
/// course, by cmid. A listed activity without a state said nothing about
/// one's work (no attempt record).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsAssignStates {
    pub course_id: u64,
    #[serde(default)]
    pub states: HashMap<u64, AssignState>,
}

/// `lms.history/{YYYY-MM}`: the assignment and quiz deadlines of one month's
/// calendar.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsHistoryMonth {
    pub month: String,
    #[serde(default)]
    pub events: Vec<Value>,
}

/// What the store holds of one module, so a course read fetches details only
/// for modules that changed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StoredModule {
    /// The content stamp of the last read (newest file time), when known.
    pub modified: Option<i64>,
    /// Whether a body is stored.
    pub has_body: bool,
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

pub fn id_of(value: Option<&Value>) -> Option<u64> {
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

pub fn lms_url(raw: &str) -> bool {
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

/// The trust boundary of `lms.enrolment`. The course list is the inventory
/// courses are marked missing against, so one invalid course or unreadable
/// grid rejects the whole key rather than dropping a course.
pub fn validate_enrolment(raw: Value) -> Result<LmsEnrolment, SchoolParseError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Raw {
        #[serde(default)]
        language: String,
        #[serde(default)]
        courses: Vec<Value>,
        #[serde(default)]
        timetables: Vec<Value>,
    }
    let raw: Raw = serde_json::from_value(raw).map_err(|_| "LMS enrolment has an invalid shape")?;
    if raw.courses.len() > MAX_COURSES || raw.timetables.len() > MAX_GRIDS {
        return Err("LMS enrolment exceeds a collection limit".into());
    }
    let mut ids = HashSet::new();
    let mut courses = Vec::with_capacity(raw.courses.len());
    for value in raw.courses {
        // Only a record without an id fails the key: one odd name or link
        // must not keep the whole course list from ever committing.
        let mut course = serde_json::from_value::<LmsCourse>(value)
            .ok()
            .filter(|course| course.id != 0)
            .ok_or("LMS enrolment lists a course without an id")?;
        if !ids.insert(course.id) {
            continue;
        }
        if course.fullname.trim().is_empty() {
            course.fullname = course.shortname.clone();
        }
        course.fullname = decode_entities(&course.fullname)
            .chars()
            .take(1_000)
            .collect();
        course.shortname = course.shortname.chars().take(255).collect();
        if !lms_url(&course.viewurl) {
            course.viewurl = format!("{LMS_WWWROOT}/course/view.php?id={}", course.id);
        }
        courses.push(course);
    }
    let mut timetables = Vec::with_capacity(raw.timetables.len());
    for grid in &raw.timetables {
        timetables.push(parse_timetable(grid, &ids).ok_or("LMS timetable grid is unreadable")?);
    }
    Ok(LmsEnrolment {
        language: raw.language,
        courses,
        timetables,
    })
}

/// Calendar entries without an activity, a course or a time are site and
/// personal events, not deadlines: they are dropped, not a failure.
fn deadline_events(list: Vec<Value>) -> Vec<Value> {
    list.into_iter()
        .filter(valid_event)
        .map(|mut event| {
            decode_fields(&mut event, &["name", "activityname"]);
            event
        })
        .collect()
}

/// The trust boundary of `lms.deadlines`.
pub fn validate_deadlines(raw: LmsDeadlines) -> Result<LmsDeadlines, SchoolParseError> {
    if raw.action_events.len() > MAX_EVENTS || raw.dated_events.len() > MAX_EVENTS {
        return Err("LMS deadlines exceed the event limit".into());
    }
    Ok(LmsDeadlines {
        action_events: deadline_events(raw.action_events),
        dated_events: deadline_events(raw.dated_events),
    })
}

/// The trust boundary of `lms.course`: the outline `{section, cm}` of one
/// course. Its modules are the inventory the course's items are marked
/// missing against, so one invalid module rejects the key.
pub fn validate_course(course_id: u64, state: Value) -> Result<LmsCourseContent, SchoolParseError> {
    let listed = state
        .get("cm")
        .and_then(Value::as_array)
        .ok_or("LMS course outline has no module list")?;
    if listed.len() > MAX_ITEMS {
        return Err("LMS course exceeds the item limit".into());
    }
    let mut modules = Vec::with_capacity(listed.len());
    for cm in listed {
        if !valid_cm(cm) {
            return Err("LMS course outline has an invalid module".into());
        }
        let mut cm = cm.clone();
        decode_fields(&mut cm, &["name"]);
        modules.push(cm);
    }
    let mut sections = state
        .get("section")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for section in &mut sections {
        decode_fields(section, &["title"]);
        if let Some(id) = id_of(section.get("id")) {
            section["id"] = json!(id);
        }
    }
    Ok(LmsCourseContent {
        course_id,
        sections,
        modules,
    })
}

/// The trust boundary of `lms.assign`: one state per assignment or quiz, as
/// the transport mapped it.
pub fn validate_assign(
    course_id: u64,
    rows: Vec<Value>,
) -> Result<LmsAssignStates, SchoolParseError> {
    if rows.len() > MAX_STATES {
        return Err("LMS assignment states exceed the limit".into());
    }
    let mut states = HashMap::with_capacity(rows.len());
    for row in &rows {
        let cmid = id_of(row.get("cmid")).ok_or("LMS assignment state has no activity")?;
        let submission = row
            .get("submission")
            .and_then(Value::as_str)
            .filter(|state| matches!(*state, "open" | "submitted" | "graded"))
            .ok_or("LMS assignment state is unknown")?;
        let grade = row
            .get("grade")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|grade| !grade.is_empty() && grade.chars().count() <= 80)
            .map(str::to_string);
        states.insert(
            cmid,
            AssignState {
                submission: submission.to_string(),
                grade,
                accepts_submission: row.get("acceptsSubmission").and_then(Value::as_bool),
            },
        );
    }
    Ok(LmsAssignStates { course_id, states })
}

/// The trust boundary of `lms.history`: `month` is `YYYY-MM`.
pub fn validate_history(
    month: &str,
    events: Vec<Value>,
) -> Result<LmsHistoryMonth, SchoolParseError> {
    let valid = month.len() == 7
        && month.as_bytes()[4] == b'-'
        && month
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || byte.is_ascii_digit());
    if !valid {
        return Err("LMS history month is malformed".into());
    }
    if events.len() > MAX_EVENTS {
        return Err("LMS history exceeds the event limit".into());
    }
    Ok(LmsHistoryMonth {
        month: month.to_string(),
        events: deadline_events(events),
    })
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
#[path = "tests/lms.rs"]
mod tests;
